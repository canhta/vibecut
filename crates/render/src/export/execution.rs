//! Executes the encode or stream-copy pass of an export.

use super::{ExportReceipt, ExportSettings};
use crate::{
    AudioBlock, Error, ExecutionHooks, Image, MemoryKind, Renderer, Result, SourceProvider, Stage,
    Unavailable, check,
};
use media::{
    AssetId, AudioEncoder, AudioEncoderSettings, Container, Muxer, PacketReader, StreamKind,
    VideoEncoder, VideoEncoderSettings,
};
use project::{AudioSource, ItemParams, Project};
use std::collections::{BTreeMap, BTreeSet};
use time::{FrameRate, Rounding, Tick, TimeRange};

/// `video` is false for audio-only output: video items then neither render nor
/// count as missing sources.
pub(super) fn admission(
    project: &Project,
    range: TimeRange,
    sources: &mut dyn SourceProvider,
    video: bool,
    include_audio: bool,
    hash: bool,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<Unavailable>> {
    let mut status = BTreeMap::new();
    let mut out = Vec::new();
    let timeline = &project.data().scene().timeline;
    // Transition clips render from their handles, so their sources need admission too.
    let rendered: BTreeSet<_> = timeline
        .rendered_items(range.start(), range.end())
        .into_iter()
        .map(|i| i.id)
        .collect();
    for track in timeline.iter() {
        for item in &track.items {
            let video = video
                && rendered.contains(&item.id)
                && matches!(item.content, ItemParams::Video(_));
            let audio = include_audio
                && !track.muted
                && !project.data().settings.master_muted
                && crate::audio::has_source_audio(item);
            let overlaps = range.overlaps(TimeRange::new(item.start, item.end()?)?);
            if !(video || (audio && overlaps)) {
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
                let window = if overlaps {
                    let first = range.start().max(item.start);
                    let last = range.end().min(item.end()?);
                    // A reversed span runs from the source of its last output tick up
                    // to the source of its first (exclusive); a forward one the other way.
                    let (start, far) = if item.retime.reverse {
                        let last = last
                            .checked_sub(Tick(1))
                            .ok_or(Error::Invalid("diagnostic range"))?;
                        let far = item
                            .source_time_at(first)?
                            .checked_add(Tick(1))
                            .ok_or(Error::Invalid("diagnostic range"))?;
                        (item.source_time_at(last)?, far)
                    } else {
                        (item.source_time_at(first)?, item.source_time_at(last)?)
                    };
                    let end = far.max(
                        start
                            .checked_add(Tick(1))
                            .ok_or(Error::Invalid("diagnostic range"))?,
                    );
                    TimeRange::new(start, end)?
                } else {
                    item.source_range
                };
                out.push(Unavailable {
                    asset: id,
                    item: item.id,
                    range: window,
                    reason,
                });
            }
        }
    }
    Ok(out)
}
pub(super) struct CopyPlan<'a> {
    pub(super) asset: &'a media::SourceAsset,
    pub(super) source: TimeRange,
    pub(super) whole: bool,
}
pub(super) fn copy_plan<'a>(
    project: &'a Project,
    range: TimeRange,
    settings: &ExportSettings,
    hooks: &dyn ExecutionHooks,
) -> Result<Option<CopyPlan<'a>>> {
    if settings.profile != media::H264Profile::Automatic {
        return Ok(None);
    }
    if !settings.render.placements.is_empty() || settings.render.background_blur.is_some() {
        return Ok(None);
    }
    // Transition clips render from their handles, so they count even outside their own extent.
    let items: Vec<_> = project
        .data()
        .scene()
        .timeline
        .rendered_items(range.start(), range.end())
        .into_iter()
        .filter(|i| !matches!(i.content, ItemParams::Audio(_)))
        .collect();
    let Some(&item) = items.first() else {
        return Ok(None);
    };
    if item.blend_mode != project::BlendMode::Normal
        || item.masks.iter().any(|m| m.active())
        || !item.effects.is_empty()
        || items.len() != 1
        || !item.retime.speed.is_unity()
        || item.retime.curve.is_some()
        || item.start > range.start()
        || item.end()? < range.end()
        || item.animations.iter().any(|(path, _)| {
            path.builtin() == Some(project::BuiltinPath::Opacity)
                || path.builtin().is_some_and(project::BuiltinPath::is_flip)
                || project::TransformField::ALL
                    .into_iter()
                    .any(|field| path.builtin() == Some(field.builtin()))
        })
    {
        return Ok(None);
    }
    // Noise reduction, stabilization, background removal and flicker removal change pixels, so a
    // switched-on item cannot be copied through.
    let ItemParams::Video(params) = &item.content else {
        return Ok(None);
    };
    if params.reduce_noise
        || params.stabilize.is_some()
        || params.background_removal.is_some()
        || params.remove_flicker
    {
        return Ok(None);
    }
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
        // A GIF is always re-encoded from rendered frames.
        Container::Gif => false,
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
pub(super) fn safe_key_cut(
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
/// Offset of output frame `n` from the range start on the export grid.
pub(super) fn frame_offset(dt: Tick, n: u64) -> Result<Tick> {
    Ok(Tick(
        dt.0.checked_mul(n as i64)
            .ok_or(Error::Invalid("output frame tick"))?,
    ))
}
#[allow(clippy::too_many_arguments)]
pub(super) fn encoded_export(
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
        profile: settings.profile,
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
        Some(AudioEncoder::new(audio_encoder_settings(
            project, settings,
        ))?)
    } else {
        None
    };
    let mut mux = Muxer::new(file, settings.container)?;
    mux.set_metadata(&licence_tags(project)?)?;
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
        let offset = frame_offset(dt, n)?;
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
fn audio_encoder_settings(project: &Project, settings: &ExportSettings) -> AudioEncoderSettings {
    AudioEncoderSettings {
        bit_rate: 192_000,
        webm: matches!(settings.container, Container::WebM),
        ..AudioEncoderSettings::aac(project.data().settings.audio_sample_rate, settings.channels)
    }
}

/// Audio-only pass: the shared mixer feeds the same audio encoder and muxer as
/// the video pass, with no video stream and no renderer frames.
pub(super) fn audio_only_export(
    project: &Project,
    range: TimeRange,
    settings: &ExportSettings,
    file: project::atomic::StreamingWriter,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
) -> Result<ExportReceipt> {
    hooks.progress(Stage::Render, 0.0)?;
    let mut audio = AudioEncoder::new(audio_encoder_settings(project, settings))?;
    let mut mux = Muxer::new(file, settings.container)?;
    mux.set_metadata(&licence_tags(project)?)?;
    let ai = mux.add_parameters(&audio.parameters(), audio.time_base())?;
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
    fill_audio(
        &mut mixer,
        project,
        origin,
        total,
        &mut samples,
        settings,
        sources,
        hooks,
        &mut audio,
        &mut mux,
        ai,
    )?;
    for packet in audio.flush()? {
        mux.write_packet(ai, &packet)?;
    }
    check(hooks)?;
    mux.finish()?;
    Ok(ExportReceipt {
        video_frames: 0,
        audio_samples: samples as u64,
        stream_copied: false,
        degraded: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fill_audio(
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
pub(super) fn copy_export(
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
    mux.set_metadata(&licence_tags(project)?)?;
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
        Some(AudioEncoder::new(audio_encoder_settings(
            project, settings,
        ))?)
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
            true,
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
pub(super) struct OmittedSources<'a> {
    pub(super) inner: &'a mut dyn SourceProvider,
    pub(super) omitted: BTreeSet<AssetId>,
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

/// Refuses an export whose range shows a fetched sticker that is not downloaded or fails its
/// verification, so no placeholder box reaches a published file.
pub(super) fn admit_stickers(project: &Project, range: TimeRange) -> Result<()> {
    for item in project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
    {
        if let ItemParams::Sticker(params) = &item.content
            && range.overlaps(TimeRange::new(item.start, item.end()?)?)
        {
            crate::scene::admit_sticker(&params.sticker)?;
        }
    }
    Ok(())
}

/// Sound-library, music and sticker-collection attribution and licence records
/// carried into the container metadata. Duplicates are collapsed by exact provider
/// identity and URL; the combined comment is bounded by the muxer's own admission.
pub(super) fn licence_tags(project: &Project) -> media::Result<Vec<(String, String)>> {
    let mut records = BTreeSet::new();
    for item in project
        .data()
        .scenes
        .iter()
        .flat_map(|scene| scene.timeline.iter().flat_map(|track| track.items.iter()))
    {
        match &item.content {
            ItemParams::Audio(params) => match &params.source {
                AudioSource::Library { sound } => {
                    records.insert(format!(
                        "{} - {} - {}",
                        sound.attribution.as_str(),
                        sound.licence.as_str(),
                        sound.source_url
                    ));
                }
                AudioSource::Music { music } => {
                    records.insert(music.attribution_record());
                }
                AudioSource::Narration { narration } => {
                    records.insert(format!(
                        "AI-generated voice - {} {} - voice {}",
                        narration.provider.as_str(),
                        narration.model.as_str(),
                        narration.voice.as_str()
                    ));
                }
                AudioSource::Upload => {}
            },
            ItemParams::Sticker(params) => {
                if let Some(credit) = params.credit() {
                    records.insert(credit.to_owned());
                }
            }
            _ => {}
        }
    }
    if records.is_empty() {
        return Ok(Vec::new());
    }
    if records.len() > 64 {
        return Err(media::MediaError::TooLarge("sound attribution records"));
    }
    let mut comment = String::new();
    for record in records {
        let separator = usize::from(!comment.is_empty());
        if comment
            .len()
            .saturating_add(record.len())
            .saturating_add(separator)
            > 8 * 1024
        {
            return Err(media::MediaError::TooLarge("sound attribution bytes"));
        }
        if !comment.is_empty() {
            comment.push('\n');
        }
        comment.push_str(&record);
    }
    Ok(vec![("comment".into(), comment)])
}
