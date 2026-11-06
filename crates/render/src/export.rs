//! Runs a staged project export and hands the result to publication.
mod execution;
mod publication;

use crate::{
    Error, ExecutionHooks, FileSources, RenderSettings, Renderer, Result, SourceProvider, Stage,
    Unavailable, check,
};
use execution::{OmittedSources, admission, copy_export, copy_plan, encoded_export};
use media::{Container, EncodePreference};
use project::{
    ItemParams, Project,
    atomic::{Overwrite, StreamingStage},
};
use publication::publish;
pub use publication::validate_output_path;
use std::{collections::BTreeSet, path::Path};
use time::{FrameRate, Tick, TimeRange};
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
        .rendered_items(range.start(), range.end())
        .into_iter()
        .filter(|i| matches!(i.content, ItemParams::Video(_)))
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
    receipt.degraded = unavailable;
    publish(stage, hooks)?;
    Ok(receipt)
}
