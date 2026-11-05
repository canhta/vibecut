use crate::{
    AudioBlock, Error, ExecutionHooks, FileSources, Image, MemoryKind, RenderSettings, Renderer,
    Result, SourceProvider, Stage, Unavailable, check,
};
use media::{
    AssetId, AudioEncoder, AudioEncoderSettings, Container, EncodePreference, Muxer, PacketReader,
    StreamKind, VideoEncoder, VideoEncoderSettings,
};
use project::{
    ItemParams, Project,
    atomic::{Overwrite, StreamingStage},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use time::{FrameRate, Rounding, Tick, TimeRange};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportMode {
    #[default]
    Auto,
    Encode,
    StreamCopy,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HdrPolicy {
    #[default]
    Refuse,
    Passthrough,
}
#[derive(Clone)]
pub struct ExportSettings {
    pub render: RenderSettings,
    pub fps: FrameRate,
    /// Output container; MP4/MOV encode H.264 + AAC, WebM encodes VP9 + Opus.
    pub container: Container,
    pub bit_rate: usize,
    pub quality: u8,
    pub encoder: EncodePreference,
    pub include_audio: bool,
    pub channels: u16,
    pub overwrite: Overwrite,
    pub allow_degraded: bool,
    /// Optional exact diagnostics accepted by an interactive caller; new degradation refuses.
    pub degraded_consent: Option<Vec<Unavailable>>,
    pub mode: ExportMode,
    pub accept_keyframe_cuts: bool,
    pub hdr: HdrPolicy,
    pub resume: bool,
    pub max_bytes: u64,
    pub audio_block: usize,
    /// Linear fades at each audible source cut; 0 disables, at most 1000 ms.
    pub audio_cut_fade_ms: u16,
}
impl ExportSettings {
    pub fn from_project(project: &Project) -> Self {
        Self {
            render: RenderSettings::from_project(project),
            fps: project.data().canvas.fps,
            container: Container::Mp4,
            bit_rate: 12_000_000,
            quality: 85,
            encoder: EncodePreference::default(),
            include_audio: true,
            channels: 2,
            overwrite: Overwrite::Refuse,
            allow_degraded: false,
            degraded_consent: None,
            mode: ExportMode::Auto,
            accept_keyframe_cuts: false,
            hdr: HdrPolicy::Refuse,
            resume: false,
            max_bytes: 64 << 30,
            audio_block: 2048,
            audio_cut_fade_ms: 0,
        }
    }
}
#[derive(Debug)]
pub struct ExportReceipt {
    pub video_frames: u64,
    pub audio_samples: u64,
    pub stream_copied: bool,
    pub degraded: Vec<Unavailable>,
}
pub fn export(
    project: &Project,
    range: TimeRange,
    settings: &ExportSettings,
    path: &Path,
    hooks: &dyn ExecutionHooks,
) -> Result<ExportReceipt> {
    export_with_sources(
        project,
        range,
        settings,
        path,
        &mut FileSources::default(),
        hooks,
    )
}
/// Complete staged export through injected domain sources. No output is promoted
/// before mux finish, synced finalization, final source validation and cancellation.
pub fn export_with_sources(
    project: &Project,
    range: TimeRange,
    settings: &ExportSettings,
    path: &Path,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
) -> Result<ExportReceipt> {
    check(hooks)?;
    if settings.resume {
        return Err(Error::Unsupported(
            "encoded export resume requires independently finalized segments and a validated manifest",
        ));
    }
    let frame = settings.fps.frame_duration()?;
    if range.start() < Tick::ZERO
        || range.end() > project.duration()
        || project.duration() == Tick::ZERO
        || frame <= Tick::ZERO
        || (i128::from(time::TICKS_PER_SECOND) * i128::from(settings.fps.den()))
            % i128::from(settings.fps.num())
            != 0
        || !matches!(settings.channels, 1 | 2)
        || settings.quality > 100
        || settings.bit_rate == 0
        || settings.audio_block == 0
        || settings.audio_block > 65536
        || settings.audio_cut_fade_ms > 1000
        || settings.max_bytes == 0
        || settings.max_bytes > 1 << 40
        || !settings.render.width.is_multiple_of(2)
        || !settings.render.height.is_multiple_of(2)
    {
        return Err(Error::Invalid("export settings/range"));
    }
    let frames = (i128::from(range.duration().0) + i128::from(frame.0) - 1) / i128::from(frame.0);
    if frames > 10_000_000 {
        return Err(Error::Invalid("export frame expansion"));
    }
    let mut renderer = Renderer::new(project, settings.render.clone())?;
    sources.begin_video(&[]);
    sources.begin_audio(&[]);
    let stage = StreamingStage::new(path, settings.overwrite, settings.max_bytes)?;
    hooks.progress(Stage::Admit, 0.0)?;
    let unavailable = admission(project, range, sources, settings.include_audio, true, hooks)?;
    if !unavailable.is_empty()
        && (!settings.allow_degraded
            || settings
                .degraded_consent
                .as_ref()
                .is_some_and(|accepted| unavailable.iter().any(|item| !accepted.contains(item))))
    {
        return Err(Error::Unavailable(unavailable));
    }
    let omitted: BTreeSet<_> = unavailable.iter().map(|u| u.asset).collect();
    validate_output_path(project, path, hooks)?;
    let copy = if settings.mode != ExportMode::Encode {
        copy_plan(project, range, settings, hooks)?
            .filter(|plan| !omitted.contains(&plan.asset.id()))
    } else {
        None
    };
    let copy = if settings.audio_cut_fade_ms == 0 {
        copy
    } else {
        None
    };
    if settings.mode == ExportMode::StreamCopy && copy.is_none() {
        return Err(Error::Unsupported(
            "stream copy requires an unchanged video span and explicitly accepted exact keyframe/no-reorder cuts",
        ));
    }
    let hdr = project
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|t| !t.hidden)
        .flat_map(|t| &t.items)
        .filter(|i| {
            matches!(i.content, ItemParams::Video(_))
                && i.start < range.end()
                && i.end().is_ok_and(|e| e > range.start())
        })
        .filter_map(|i| {
            project
                .data()
                .assets
                .iter()
                .find(|a| Some(a.id()) == i.source_asset)
        })
        .any(|a| {
            a.probe()
                .streams
                .iter()
                .any(|s| s.video.as_ref().is_some_and(|v| v.color.is_hdr()))
        });
    if hdr && (settings.hdr != HdrPolicy::Passthrough || copy.is_none()) {
        return Err(Error::Unsupported(
            "HDR requires unchanged passthrough; no implicit tone map",
        ));
    }
    let mut source = OmittedSources {
        inner: sources,
        omitted,
    };
    let result = if let Some(asset) = copy {
        copy_export(
            project,
            range,
            settings,
            stage.writer()?,
            asset,
            &mut source,
            hooks,
        )
    } else {
        encoded_export(
            project,
            range,
            settings,
            stage.writer()?,
            &mut renderer,
            &mut source,
            hooks,
            frames as u64,
        )
    };
    let mut receipt = match result {
        Ok(v) => v,
        Err(error) => {
            source.begin_video(&[]);
            source.begin_audio(&[]);
            let missing = admission(
                project,
                range,
                source.inner,
                settings.include_audio,
                false,
                hooks,
            )?;
            if missing
                .iter()
                .any(|item| !source.omitted.contains(&item.asset))
            {
                return Err(Error::Unavailable(missing));
            }
            return Err(error);
        }
    };
    drop(renderer);
    source.begin_video(&[]);
    source.begin_audio(&[]);
    hooks.progress(Stage::Finalize, 1.0)?;
    check(hooks)?;
    let final_missing = admission(
        project,
        range,
        source.inner,
        settings.include_audio,
        true,
        hooks,
    )?;
    if final_missing
        .iter()
        .any(|u| !source.omitted.contains(&u.asset))
    {
        return Err(Error::Unavailable(final_missing));
    }
    let finalized = stage.finalize()?;
    receipt.degraded = unavailable;
    hooks.progress(Stage::Finalized, 1.0)?;
    check(hooks)?;
    hooks.progress(Stage::Publish, 1.0)?;
    check(hooks)?;
    // The engine implementation atomically fences cancellation here. No fallible
    // callback or result allocation runs after successful irreversible publication.
    hooks.begin_publish()?;
    finalized.publish()?;
    Ok(receipt)
}
fn admission(
    project: &Project,
    range: TimeRange,
    sources: &mut dyn SourceProvider,
    include_audio: bool,
    hash: bool,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<Unavailable>> {
    let mut status = BTreeMap::new();
    let mut out = Vec::new();
    for track in project.data().scene().timeline.iter() {
        for item in &track.items {
            let video = !track.hidden && matches!(item.content, ItemParams::Video(_));
            let audio = include_audio
                && !track.muted
                && !project.data().settings.master_muted
                && crate::audio::has_source_audio(item);
            if (!video && !audio) || !range.overlaps(TimeRange::new(item.start, item.end()?)?) {
                continue;
            }
            let Some(id) = item.source_asset else {
                continue;
            };
            let asset = project
                .data()
                .assets
                .iter()
                .find(|a| a.id() == id)
                .ok_or(Error::Invalid("admission asset"))?;
            let reason = if let Some(value) = status.get(&id) {
                *value
            } else {
                let value = sources.status(asset, hash, hooks)?;
                status.insert(id, value);
                value
            };
            if let Some(reason) = reason {
                let start = item
                    .source_range
                    .start()
                    .checked_add(range.start().max(item.start) - item.start)
                    .ok_or(Error::Invalid("diagnostic range"))?;
                let end = item
                    .source_range
                    .start()
                    .checked_add(range.end().min(item.end()?) - item.start)
                    .ok_or(Error::Invalid("diagnostic range"))?;
                out.push(Unavailable {
                    asset: id,
                    item: item.id,
                    range: TimeRange::new(start, end)?,
                    reason,
                });
            }
        }
    }
    Ok(out)
}
struct CopyPlan<'a> {
    asset: &'a media::SourceAsset,
    source: TimeRange,
    whole: bool,
}
fn copy_plan<'a>(
    project: &'a Project,
    range: TimeRange,
    settings: &ExportSettings,
    hooks: &dyn ExecutionHooks,
) -> Result<Option<CopyPlan<'a>>> {
    if !settings.render.placements.is_empty() || settings.render.background_blur.is_some() {
        return Ok(None);
    }
    let items: Vec<_> = project
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|t| !t.hidden)
        .flat_map(|t| t.items.iter().map(move |i| (t, i)))
        .filter(|(_, i)| {
            !matches!(i.content, ItemParams::Audio(_))
                && i.start < range.end()
                && i.end().is_ok_and(|end| end > range.start())
        })
        .collect();
    let Some(&(_, item)) = items.first() else {
        return Ok(None);
    };
    if item.blend_mode != project::BlendMode::Normal
        || item.masks.iter().any(|m| m.active())
        || !item.effects.is_empty()
        || items.len() != 1
        || item.start > range.start()
        || item.end()? < range.end()
        || item
            .animations
            .iter()
            .any(|(path, _)| path.as_str() == "opacity")
    {
        return Ok(None);
    }
    let ItemParams::Video(_) = &item.content else {
        return Ok(None);
    };
    let asset = project
        .data()
        .assets
        .iter()
        .find(|a| Some(a.id()) == item.source_asset)
        .ok_or(Error::Invalid("copy source"))?;
    let video = asset
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .ok_or(Error::Invalid("copy video"))?;
    let shape = video
        .video
        .as_ref()
        .ok_or(Error::Invalid("copy dimensions"))?;
    let compatible = match settings.container {
        Container::Mp4 | Container::Mov => {
            matches!(video.codec.as_str(), "h264" | "hevc" | "mpeg4")
        }
        Container::WebM => matches!(video.codec.as_str(), "vp8" | "vp9" | "av1"),
    };
    if !compatible
        || shape.width != settings.render.width
        || shape.height != settings.render.height
        || video.frame_rate != Some(settings.fps)
    {
        return Ok(None);
    }
    let start = item
        .source_range
        .start()
        .checked_add(range.start() - item.start)
        .ok_or(Error::Invalid("copy start"))?;
    let end = start
        .checked_add(range.duration())
        .ok_or(Error::Invalid("copy end"))?;
    let source = TimeRange::new(start, end)?;
    let whole = start == Tick::ZERO && asset.probe().duration == Some(end);
    if !whole
        && (!settings.accept_keyframe_cuts || !safe_key_cut(asset, source, settings.fps, hooks)?)
    {
        return Ok(None);
    }
    Ok(Some(CopyPlan {
        asset,
        source,
        whole,
    }))
}
fn safe_key_cut(
    asset: &media::SourceAsset,
    range: TimeRange,
    fps: FrameRate,
    hooks: &dyn ExecutionHooks,
) -> Result<bool> {
    let _packets = hooks.reserve(MemoryKind::Decoded, 96 << 20)?;
    let mut reader = PacketReader::open(asset.path())?;
    let index = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .ok_or(Error::Invalid("key-cut video"))?
        .index;
    if reader.seek(range.start()).is_err() {
        return Ok(false);
    }
    let dt = fps.frame_duration()?;
    let mut start = false;
    let mut end = asset.probe().duration == Some(range.end());
    let mut last = None;
    while let Some(packet) = reader.next_packet()? {
        check(hooks)?;
        if packet.stream() != index {
            continue;
        }
        let Some(at) = packet.pts()? else {
            return Ok(false);
        };
        if at < range.start() {
            continue;
        }
        if packet.dts()? != Some(at) {
            return Ok(false);
        }
        if at == range.start() {
            start = packet.keyframe();
        }
        if at >= range.end() {
            end = at == range.end() && packet.keyframe();
            break;
        }
        if last.is_some_and(|last: Tick| last.checked_add(dt) != Some(at)) {
            return Ok(false);
        }
        last = Some(at);
    }
    Ok(start && end && last.is_some_and(|last| last.checked_add(dt) == Some(range.end())))
}
#[allow(clippy::too_many_arguments)]
fn encoded_export(
    project: &Project,
    range: TimeRange,
    settings: &ExportSettings,
    file: project::atomic::StreamingWriter,
    renderer: &mut Renderer<'_>,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    count: u64,
) -> Result<ExportReceipt> {
    let (threads, _encode) = crate::conversion_reservation(
        hooks,
        MemoryKind::Export,
        Image::bytes(settings.render.width, settings.render.height)? * 4 + (64 << 20),
    )?;
    let mut first = renderer.render_frame(range.start(), sources, hooks)?;
    let rgba = first.video_rgba8(1, hooks)?;
    let first_frame = media::VideoFrame::from_rgba8(
        settings.render.width,
        settings.render.height,
        Tick::ZERO,
        media::ColorInfo::default(),
        &rgba.bytes,
    )?;
    drop(rgba);
    let video_settings = VideoEncoderSettings {
        webm: matches!(settings.container, Container::WebM),
        bit_rate: settings.bit_rate,
        quality: settings.quality,
        conversion_threads: threads,
        ..VideoEncoderSettings::h264(
            settings.render.width,
            settings.render.height,
            settings.fps,
            media::ColorInfo::default(),
        )
    };
    let mut video = VideoEncoder::with_preference(video_settings, &first_frame, settings.encoder)?;
    drop(first_frame);
    let mut audio = if settings.include_audio {
        Some(AudioEncoder::new(AudioEncoderSettings {
            bit_rate: 192_000,
            webm: matches!(settings.container, Container::WebM),
            ..AudioEncoderSettings::aac(
                project.data().settings.audio_sample_rate,
                settings.channels,
            )
        })?)
    } else {
        None
    };
    let mut mux = Muxer::new(file, settings.container)?;
    let vi = mux.add_parameters(&video.parameters(), video.time_base())?;
    let ai = audio
        .as_ref()
        .map(|a| mux.add_parameters(&a.parameters(), a.time_base()))
        .transpose()?;
    let sample_rate = project.data().settings.audio_sample_rate;
    let total = range
        .duration()
        .to_units(i64::from(sample_rate), Rounding::Nearest)?;
    let origin = range
        .start()
        .to_units(i64::from(sample_rate), Rounding::Nearest)?;
    let mut mixer = crate::AudioMixer::new(
        project,
        settings.channels,
        settings.audio_cut_fade_ms,
        hooks,
    )?;
    let mut samples = 0i64;
    let dt = settings.fps.frame_duration()?;
    for n in 0..count {
        check(hooks)?;
        hooks.progress(Stage::Render, n as f32 / count as f32)?;
        let offset = Tick(
            dt.0.checked_mul(n as i64)
                .ok_or(Error::Invalid("output frame tick"))?,
        );
        if n > 0 {
            first = renderer.render_frame(
                range
                    .start()
                    .checked_add(offset)
                    .ok_or(Error::Invalid("render tick"))?,
                sources,
                hooks,
            )?;
        }
        let rgba = first.video_rgba8(1, hooks)?;
        let frame = media::VideoFrame::from_rgba8(
            settings.render.width,
            settings.render.height,
            offset,
            media::ColorInfo::default(),
            &rgba.bytes,
        )?;
        hooks.progress(Stage::Encode, n as f32 / count as f32)?;
        for packet in video.encode(&frame)? {
            mux.write_packet(vi, &packet)?;
        }
        let goal = offset
            .checked_add(dt)
            .ok_or(Error::Invalid("audio goal"))?
            .to_units(i64::from(sample_rate), Rounding::Nearest)?
            .min(total);
        if let (Some(audio), Some(ai)) = (audio.as_mut(), ai) {
            fill_audio(
                &mut mixer,
                project,
                origin,
                goal,
                &mut samples,
                settings,
                sources,
                hooks,
                audio,
                &mut mux,
                ai,
            )?;
        }
    }
    for packet in video.flush()? {
        mux.write_packet(vi, &packet)?;
    }
    if let (Some(audio), Some(ai)) = (audio.as_mut(), ai) {
        fill_audio(
            &mut mixer,
            project,
            origin,
            total,
            &mut samples,
            settings,
            sources,
            hooks,
            audio,
            &mut mux,
            ai,
        )?;
        for packet in audio.flush()? {
            mux.write_packet(ai, &packet)?;
        }
    }
    check(hooks)?;
    mux.finish()?;
    Ok(ExportReceipt {
        video_frames: count,
        audio_samples: samples as u64,
        stream_copied: false,
        degraded: Vec::new(),
    })
}
#[allow(clippy::too_many_arguments)]
fn fill_audio(
    mixer: &mut crate::AudioMixer<'_>,
    project: &Project,
    origin: i64,
    goal: i64,
    position: &mut i64,
    settings: &ExportSettings,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    audio: &mut AudioEncoder,
    mux: &mut Muxer,
    track: usize,
) -> Result<()> {
    let rate = project.data().settings.audio_sample_rate;
    while *position < goal {
        check(hooks)?;
        hooks.progress(Stage::Mix, 0.0)?;
        let count = (goal - *position).min(settings.audio_block as i64) as usize;
        let block = mixer.mix(
            origin
                .checked_add(*position)
                .ok_or(Error::Invalid("mix sample position"))?,
            count,
            sources,
            hooks,
        )?;
        let peak = block.peak();
        if peak > 1.0 {
            return Err(Error::AudioOverload { peak });
        }
        let _native = hooks.reserve(
            MemoryKind::Export,
            count as u64 * u64::from(settings.channels) * 8 + (8 << 20),
        )?;
        let frame = media::AudioFrame::from_planar_f32(
            Tick::from_units(*position, i64::from(rate), Rounding::Nearest)?,
            rate,
            block.planes(),
        )?;
        for packet in audio.encode(&frame)? {
            mux.write_packet(track, &packet)?;
        }
        *position = position
            .checked_add(count as i64)
            .ok_or(Error::Invalid("audio position"))?;
    }
    Ok(())
}
fn copy_export(
    project: &Project,
    range: TimeRange,
    settings: &ExportSettings,
    file: project::atomic::StreamingWriter,
    plan: CopyPlan<'_>,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
) -> Result<ExportReceipt> {
    let _packets = hooks.reserve(MemoryKind::Export, 128 << 20)?;
    let mut reader = PacketReader::open(plan.asset.path())?;
    let mut mux = Muxer::new(file, settings.container)?;
    // Timeline sources select the first video/audio stream, just like decode.
    // Alternate angles and audio languages must not leak into the final movie.
    let video_index = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .ok_or(Error::Invalid("copy video stream"))?
        .index;
    let mut tracks = BTreeMap::new();
    for stream in &reader.probe().streams {
        if stream.index == video_index {
            tracks.insert(stream.index, mux.add_stream(stream)?);
        }
    }
    let mut audio = if settings.include_audio {
        Some(AudioEncoder::new(AudioEncoderSettings {
            bit_rate: 192_000,
            webm: matches!(settings.container, Container::WebM),
            ..AudioEncoderSettings::aac(
                project.data().settings.audio_sample_rate,
                settings.channels,
            )
        })?)
    } else {
        None
    };
    let ai = audio
        .as_ref()
        .map(|a| mux.add_parameters(&a.parameters(), a.time_base()))
        .transpose()?;
    let mut frames = 0u64;
    let mut mixer = crate::AudioMixer::new(
        project,
        settings.channels,
        settings.audio_cut_fade_ms,
        hooks,
    )?;
    let mut samples = 0i64;
    let total = range.duration().to_units(
        i64::from(project.data().settings.audio_sample_rate),
        Rounding::Nearest,
    )?;
    if !plan.whole {
        reader.seek(plan.source.start())?;
    }
    let origin = range.start().to_units(
        i64::from(project.data().settings.audio_sample_rate),
        Rounding::Nearest,
    )?;
    while let Some(packet) = reader.next_packet()? {
        check(hooks)?;
        let source_at = packet.pts()?.unwrap_or(Tick::ZERO);
        let video = reader
            .probe()
            .streams
            .iter()
            .find(|s| s.index == packet.stream())
            .is_some_and(|s| s.kind == StreamKind::Video);
        if !plan.whole && (!video || !plan.source.contains(source_at)) {
            continue;
        }
        let at = source_at - plan.source.start();
        hooks.progress(
            Stage::Encode,
            (at.seconds() / range.duration().seconds()).clamp(0.0, 1.0) as f32,
        )?;
        let missing = admission(
            project,
            range,
            sources,
            settings.include_audio,
            false,
            hooks,
        )?;
        if !missing.is_empty() {
            return Err(Error::Unavailable(missing));
        }
        if let Some(track) = tracks.get(&packet.stream()) {
            if plan.whole {
                mux.write_packet(*track, &packet)?;
            } else {
                let delta = plan
                    .source
                    .start()
                    .checked_neg()
                    .ok_or(Error::Invalid("copy timestamp origin"))?;
                mux.write_packet(*track, &packet.shifted(delta)?)?;
            }
            if reader
                .probe()
                .streams
                .iter()
                .find(|s| s.index == packet.stream())
                .is_some_and(|s| s.kind == StreamKind::Video)
            {
                frames += 1;
            }
        }
        let goal = at
            .to_units(
                i64::from(project.data().settings.audio_sample_rate),
                Rounding::Nearest,
            )?
            .clamp(0, total);
        if let (Some(audio), Some(ai)) = (audio.as_mut(), ai) {
            fill_audio(
                &mut mixer,
                project,
                origin,
                goal,
                &mut samples,
                settings,
                sources,
                hooks,
                audio,
                &mut mux,
                ai,
            )?;
        }
    }
    if let (Some(audio), Some(ai)) = (audio.as_mut(), ai) {
        fill_audio(
            &mut mixer,
            project,
            origin,
            total,
            &mut samples,
            settings,
            sources,
            hooks,
            audio,
            &mut mux,
            ai,
        )?;
        for packet in audio.flush()? {
            mux.write_packet(ai, &packet)?;
        }
    }
    check(hooks)?;
    mux.finish()?;
    Ok(ExportReceipt {
        video_frames: frames,
        audio_samples: samples as u64,
        stream_copied: true,
        degraded: Vec::new(),
    })
}
struct OmittedSources<'a> {
    inner: &'a mut dyn SourceProvider,
    omitted: BTreeSet<AssetId>,
}
impl SourceProvider for OmittedSources<'_> {
    fn status(
        &mut self,
        a: &media::SourceAsset,
        hash: bool,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<crate::UnavailableReason>> {
        if self.omitted.contains(&a.id()) {
            Ok(None)
        } else {
            self.inner.status(a, hash, h)
        }
    }
    fn begin_audio(&mut self, ids: &[project::ItemId]) {
        self.inner.begin_audio(ids)
    }
    fn begin_video(&mut self, ids: &[project::ItemId]) {
        self.inner.begin_video(ids)
    }
    fn finish_audio(&mut self, id: project::ItemId) {
        self.inner.finish_audio(id)
    }
    fn video(
        &mut self,
        a: &media::SourceAsset,
        id: project::ItemId,
        t: Tick,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        if self.omitted.contains(&a.id()) {
            Ok(None)
        } else {
            self.inner.video(a, id, t, h)
        }
    }
    fn video_window(
        &mut self,
        a: &media::SourceAsset,
        id: project::ItemId,
        t: Tick,
        window: Option<std::ops::Range<u32>>,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        if self.omitted.contains(&a.id()) {
            Ok(None)
        } else {
            self.inner.video_window(a, id, t, window, h)
        }
    }
    fn audio(
        &mut self,
        a: &media::SourceAsset,
        id: project::ItemId,
        start: i64,
        count: usize,
        rate: u32,
        h: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        if self.omitted.contains(&a.id()) {
            AudioBlock::new(rate, 2, count, MemoryKind::Decoded, h)
        } else {
            self.inner.audio(a, id, start, count, rate, h)
        }
    }
}

/// Refuse destination aliases of source media before staging any exported file.
pub fn validate_output_path(
    project: &Project,
    path: &Path,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    check(hooks)?;
    // Refuse direct/symlink/hard-link aliases of every project source. Missing
    // destinations are normalized through their existing parent before staging.
    let normalized = if path.exists() {
        std::fs::canonicalize(path)?
    } else {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::canonicalize(parent)?
            .join(path.file_name().ok_or(Error::Invalid("output filename"))?)
    };
    for asset in &project.data().assets {
        let source = std::fs::canonicalize(asset.path()).or_else(|_| {
            let parent = asset.path().parent().unwrap_or(Path::new("."));
            std::fs::canonicalize(parent)
                .map(|parent| parent.join(asset.path().file_name().unwrap_or_default()))
        });
        if path == asset.path()
            || normalized == asset.path()
            || source.is_ok_and(|source| normalized == source)
        {
            return Err(Error::Invalid("output aliases a source"));
        }
        #[cfg(unix)]
        if let (Ok(output), Ok(source)) = (std::fs::metadata(path), std::fs::metadata(asset.path()))
        {
            use std::os::unix::fs::MetadataExt;
            if output.dev() == source.dev() && output.ino() == source.ino() {
                return Err(Error::Invalid("output aliases a source by hard link"));
            }
        }
        #[cfg(not(unix))]
        if path.exists()
            && media::ContentHash::read_with_cancel(path, || hooks.cancelled())?
                == asset.content_hash()
        {
            return Err(Error::Invalid("output may alias source content"));
        }
    }
    Ok(())
}
