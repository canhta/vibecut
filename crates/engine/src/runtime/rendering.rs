//! Submits bounded project export work.
use crate::cache::CacheKey;
use crate::session::wire::{ErrorCode, Failure};

impl Failure for media::MediaError {
    fn code(&self) -> ErrorCode {
        ErrorCode::Media
    }
}

impl Failure for render::Error {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Unavailable(_) => ErrorCode::Unavailable,
            _ => ErrorCode::Render,
        }
    }
}

use super::{Error as RuntimeError, Result};
use crate::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use crate::{ByteBudgetHooks, ByteLease};
use std::{path::PathBuf, sync::Arc};
impl render::Reservation for ByteLease {}
impl render::ExecutionHooks for JobContext {
    fn parallelism(&self) -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
    }
    fn run_tasks(
        &self,
        tasks: &mut [&mut (dyn FnMut() -> render::Result<()> + Send)],
    ) -> render::Result<()> {
        if self.cancelled() {
            return Err(render::Error::Cancelled);
        }
        if tasks.len() > self.parallelism() {
            return Err(render::Error::Invalid("pixel task count"));
        }
        if tasks.is_empty() {
            return Ok(());
        }
        // The engine owns every extra worker and its bounded stack. No global
        // pool, detached work or uncharged lifetime is introduced by render.
        const STACK: usize = 512 << 10;
        let _stacks = match self.reserve(
            render::MemoryKind::Compositor,
            (tasks.len().saturating_sub(1) * STACK) as u64,
        ) {
            Err(render::Error::Budget { .. }) => {
                for task in tasks {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                        .unwrap_or(Err(render::Error::WorkerPanicked))?;
                }
                return Ok(());
            }
            result => result?,
        };
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            handles.try_reserve_exact(tasks.len().saturating_sub(1))?;
            let Some((local, remote)) = tasks.split_last_mut() else {
                return Ok(());
            };
            for task in remote {
                handles.push(
                    std::thread::Builder::new()
                        .name("vibecut-pixel".into())
                        .stack_size(STACK)
                        .spawn_scoped(scope, move || {
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                                .unwrap_or(Err(render::Error::WorkerPanicked))
                        })?,
                );
            }
            let mut result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(local))
                .unwrap_or(Err(render::Error::WorkerPanicked));
            for handle in handles {
                let joined = handle.join().unwrap_or(Err(render::Error::WorkerPanicked));
                if result.is_ok() {
                    result = joined;
                }
            }
            result
        })
    }
    fn cancelled(&self) -> bool {
        self.cancelled()
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        render::ExecutionHooks::reserve(
            &ByteBudgetHooks::new(
                self.memory_budget(),
                Some(self as &dyn render::ExecutionHooks),
            ),
            kind,
            bytes,
        )
    }
    fn begin_publish(&self) -> render::Result<()> {
        self.begin_publication()
            .map_err(|_| render::Error::Cancelled)
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        self.progress(
            match stage {
                render::Stage::Admit => JobStage::Admit,
                render::Stage::Render => JobStage::Render,
                render::Stage::Mix => JobStage::Mix,
                render::Stage::Encode => JobStage::Encode,
                render::Stage::Finalize | render::Stage::Finalized => JobStage::Finalize,
                render::Stage::Publish => JobStage::Publish,
            },
            fraction,
        )
        .map_err(|e| {
            if matches!(e, super::Error::Cancelled) {
                render::Error::Cancelled
            } else {
                render::Error::Invalid("job progress")
            }
        })
    }
}
/// Export the captured immutable project through the production render path.
/// Render reserves its actual buffer lifetimes through this JobContext; the task
/// declares zero additional working bytes to avoid charging each buffer twice.
/// The output is a completed file, not an edit applied to the current Session.
pub fn submit_export(
    runner: &mut JobRunner,
    input: CacheKey,
    project: Arc<project::Project>,
    range: time::TimeRange,
    settings: render::ExportSettings,
    path: PathBuf,
) -> Result<(JobId, CancelToken)> {
    submit_export_with_sources(runner, input, project, range, settings, path, || {
        Box::new(render::FileSources::default())
    })
}
/// Same export job with an injected render source factory. The factory runs on
/// the engine worker; production and injected adapters share cancellation,
/// accounting and the atomic publication fence.
pub fn submit_export_with_sources(
    runner: &mut JobRunner,
    input: CacheKey,
    project: Arc<project::Project>,
    range: time::TimeRange,
    settings: render::ExportSettings,
    path: PathBuf,
    sources: impl Fn() -> Box<dyn render::SourceProvider> + Send + Sync + 'static,
) -> Result<(JobId, CancelToken)> {
    super::loudness_admission::admit(&project)?;
    runner.submit::<RuntimeError>(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::GpuEncoder,
            input,
            bytes: JobBytes::default(),
        },
        move |context| {
            render::export_with_sources(
                &project,
                range,
                &settings,
                &path,
                sources().as_mut(),
                context,
            )?;
            Ok(Vec::new())
        },
    )
}

/// `export.clip`: one export job over the clip's own timeline span. `Clip` renders only
/// that clip (see `clip_only`); `Duration` renders every track over the same span. The
/// session is never edited; the output is a completed file like any export.
pub fn submit_clip_export(
    runner: &mut JobRunner,
    input: CacheKey,
    project: Arc<project::Project>,
    clip: project::ItemId,
    mode: crate::session::ClipExportMode,
    overwrite: bool,
    path: PathBuf,
) -> Result<(JobId, CancelToken)> {
    let span = clip_export_range(&project, clip)?;
    let rendered = match mode {
        crate::session::ClipExportMode::Clip => Arc::new(clip_only(&project, clip)?),
        crate::session::ClipExportMode::Duration => project,
    };
    let mut settings = render::ExportSettings::from_project(&rendered);
    render::OutputOptions::default().apply(&mut settings)?;
    settings.container = render::OutputOptions::container_for_path(&path, false)?;
    settings.overwrite = if overwrite {
        project::atomic::Overwrite::Replace
    } else {
        project::atomic::Overwrite::Refuse
    };
    submit_export_with_sources(runner, input, rendered, span, settings, path, || {
        Box::new(render::FileSources::default())
    })
}

/// The clip's own timeline span, the range an `export.clip` job renders.
pub fn clip_export_range(
    project: &project::Project,
    clip: project::ItemId,
) -> Result<time::TimeRange> {
    let item = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == clip)
        .ok_or(RuntimeError::Edit(crate::EditError::Invalid("export clip")))?;
    time::TimeRange::new(item.start, item.end()?)
        .map_err(|_| RuntimeError::Edit(crate::EditError::Invalid("export clip range")))
}

/// The project with `clip` as its only item: every other item is removed and the
/// transitions are dropped, so the render holds the clip's own pixels and audio.
fn clip_only(project: &project::Project, clip: project::ItemId) -> Result<project::Project> {
    let mut data = project.clone().into_data();
    let timeline = &mut data.scene_mut().timeline;
    timeline.transitions.clear();
    for track in timeline.iter_mut() {
        track.items.retain(|item| item.id == clip);
    }
    Ok(project::Project::new(data)?)
}

/// One cancellable job finalizes every scene privately before publication.
/// Rendering failure/cancellation preserves all destinations; publication is
/// per-file atomic, so a later publication failure can leave earlier files.
pub fn submit_scene_exports_with_sources(
    runner: &mut JobRunner,
    input: CacheKey,
    project: Arc<project::Project>,
    options: render::OutputOptions,
    settings: render::ExportSettings,
    paths: Vec<PathBuf>,
    sources: impl Fn() -> Box<dyn render::SourceProvider> + Send + Sync + 'static,
) -> Result<(JobId, CancelToken)> {
    let scenes = options.ranges(&project)?;
    if scenes.len() != paths.len() {
        return Err(RuntimeError::Edit(crate::EditError::Invalid(
            "scene output paths",
        )));
    }
    if paths
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != paths.len()
    {
        return Err(RuntimeError::Edit(crate::EditError::Invalid(
            "duplicate scene output paths",
        )));
    }
    for (id, _) in &scenes {
        super::loudness_admission::admit(&project.for_scene(*id)?)?;
    }
    let mut settings = settings;
    options.apply(&mut settings)?;
    options.check_gif(&project, &settings)?;
    if let [(scene, range)] = scenes.as_slice() {
        let snapshot = if *scene == project.data().current_scene_id {
            project.clone()
        } else {
            Arc::new(project.for_scene(*scene)?)
        };
        settings.render.placements.retain(|id, _| {
            snapshot
                .data()
                .scene()
                .timeline
                .iter()
                .any(|track| track.items.iter().any(|item| item.id == *id))
        });
        return submit_export_with_sources(
            runner,
            input,
            snapshot,
            *range,
            settings,
            paths
                .first()
                .ok_or(RuntimeError::Edit(crate::EditError::Invalid(
                    "scene output path",
                )))?
                .clone(),
            sources,
        );
    }
    runner.submit::<RuntimeError>(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::GpuEncoder,
            input,
            bytes: JobBytes::default(),
        },
        move |context| {
            for path in &paths {
                render::validate_output_path(&project, path, context)?;
                // Admit overwrite, namespace and write permission before paying
                // for every scene; publication rechecks the destination later.
                drop(project::atomic::StreamingStage::new(
                    path,
                    settings.overwrite,
                    settings.max_bytes,
                )?);
            }
            let staging = SceneExportStaging::new()?;
            let hooks = ByteBudgetHooks::new(
                context.memory_budget(),
                Some(context as &dyn render::ExecutionHooks),
            );
            let mut completed = Vec::new();
            for (index, ((scene, range), path)) in scenes.iter().zip(&paths).enumerate() {
                let snapshot = project.for_scene(*scene)?;
                let temporary = staging.0.join(format!("scene-{index}"));
                let mut staged_settings = settings.clone();
                staged_settings.render.placements.retain(|id, _| {
                    snapshot
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .any(|track| track.items.iter().any(|item| item.id == *id))
                });
                staged_settings.overwrite = project::atomic::Overwrite::Refuse;
                render::export_with_sources(
                    &snapshot,
                    *range,
                    &staged_settings,
                    &temporary,
                    sources().as_mut(),
                    &hooks,
                )?;
                completed.push((temporary, path));
            }
            // All scenes are finalized before crossing the job's one publication
            // fence. Cancellation during rendering publishes no scene files.
            for (temporary, path) in completed {
                super::staged_publication::Publication {
                    project: &project,
                    path,
                    protected: &[],
                    hooks: context,
                }
                .write::<RuntimeError>(
                    settings.overwrite,
                    settings.max_bytes,
                    &project::atomic::Fs,
                    |writer| {
                        let mut input = std::fs::File::open(temporary)?;
                        use std::io::{Read, Write};
                        let _lease = render::ExecutionHooks::reserve(
                            context,
                            render::MemoryKind::Export,
                            64 << 10,
                        )?;
                        let mut buffer = vec![0; 64 << 10];
                        loop {
                            if context.cancelled() {
                                return Err(RuntimeError::Cancelled);
                            }
                            let count = input.read(&mut buffer)?;
                            if count == 0 {
                                break;
                            }
                            writer.write_all(buffer.get(..count).ok_or(RuntimeError::Edit(
                                crate::EditError::Invalid("scene publication read"),
                            ))?)?;
                        }
                        Ok(())
                    },
                )?;
            }
            Ok(Vec::new())
        },
    )
}

struct SceneExportStaging(PathBuf);
impl SceneExportStaging {
    fn new() -> Result<Self> {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| {
            RuntimeError::Edit(crate::EditError::Invalid("export staging identity"))
        })?;
        let path = std::fs::canonicalize(std::env::temp_dir())?.join(format!(
            "vibecut-scenes-{:032x}",
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for SceneExportStaging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// Publish a completed export on the runner. The snapshot and protected project
/// paths are admitted before staging and again before the cancellation fence.
/// Failure/cancel retains the completed input; the caller owns its lifetime.
pub fn submit_export_publication(
    runner: &mut JobRunner,
    input: CacheKey,
    project: Arc<project::Project>,
    source: PathBuf,
    destination: PathBuf,
    protected: Vec<PathBuf>,
    fs: Arc<dyn project::atomic::AtomicFs + Send + Sync>,
) -> Result<(JobId, CancelToken)> {
    runner.submit::<RuntimeError>(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::Cpu,
            input,
            bytes: JobBytes {
                export: 64 << 10,
                ..JobBytes::default()
            },
        },
        move |context| {
            use std::io::{Read, Write};
            let mut source = std::fs::File::open(&source)?;
            let size = source.metadata()?.len();
            let mut buffer = vec![0; 64 << 10];
            let mut copied = 0u64;
            crate::publication::Publication {
                project: &project,
                path: &destination,
                protected: &protected,
                hooks: context,
            }
            .write::<RuntimeError>(
                project::atomic::Overwrite::Replace,
                size.max(1),
                fs.as_ref(),
                |writer| {
                    loop {
                        if context.cancelled() {
                            return Err(RuntimeError::Cancelled);
                        }
                        let count = source.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        writer.write_all(buffer.get(..count).ok_or(RuntimeError::Edit(
                            crate::EditError::Invalid("publication read size"),
                        ))?)?;
                        copied += count as u64;
                        context.progress(
                            JobStage::Finalize,
                            (copied as f64 / size.max(1) as f64) as f32,
                        )?;
                    }
                    Ok(())
                },
            )?;
            Ok(Vec::new())
        },
    )
}

/// One whole-source output of `export.clips`, planned and validated before submission.
pub struct AssetExport {
    pub asset: media::AssetId,
    pub path: PathBuf,
    pub project: project::Project,
    pub range: time::TimeRange,
    pub settings: render::ExportSettings,
}

/// Most source assets one `export.clips` job admits.
const MAX_ASSET_EXPORTS: usize = 256;

/// Plan one full-source export per selected asset, in request order. Each output is
/// the asset's complete source `[0, duration)` in the project's export settings; used
/// ranges, trims, effects, scenes and the timeline play no part.
pub fn plan_asset_exports(
    project: &project::Project,
    assets: &[media::AssetId],
    directory: &std::path::Path,
    overwrite: bool,
) -> Result<Vec<AssetExport>> {
    if assets.is_empty() || assets.len() > MAX_ASSET_EXPORTS {
        return Err(RuntimeError::Edit(crate::EditError::Invalid(
            "asset export count",
        )));
    }
    if !directory.is_dir() {
        return Err(RuntimeError::Edit(crate::EditError::Invalid(
            "asset export directory",
        )));
    }
    let mut chosen = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut plan = Vec::with_capacity(assets.len());
    for &asset_id in assets {
        if !chosen.insert(asset_id) {
            return Err(RuntimeError::Edit(crate::EditError::Invalid(
                "duplicate asset export",
            )));
        }
        let asset = project
            .data()
            .assets
            .iter()
            .find(|candidate| candidate.id() == asset_id)
            .ok_or(RuntimeError::Edit(crate::EditError::Invalid(
                "export clip asset",
            )))?;
        if !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == media::StreamKind::Video)
        {
            return Err(RuntimeError::Edit(crate::EditError::Invalid(
                "export clip media",
            )));
        }
        let duration = asset
            .probe()
            .duration
            .filter(|duration| *duration > time::Tick::ZERO)
            .ok_or(RuntimeError::Edit(crate::EditError::Invalid(
                "export clip duration",
            )))?;
        let range = time::TimeRange::new(time::Tick::ZERO, duration)
            .map_err(|_| RuntimeError::Edit(crate::EditError::Invalid("export clip duration")))?;
        let derived = full_source_project(project, asset, range)?;
        let mut settings = render::ExportSettings::from_project(&derived);
        settings.overwrite = overwrite_policy(overwrite);
        render::Renderer::new(&derived, settings.render.clone())?;
        super::loudness_admission::admit(&derived)?;
        let stem = asset
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let name = unique_export_name(stem, &mut names);
        plan.push(AssetExport {
            asset: asset_id,
            path: directory.join(name),
            project: derived,
            range,
            settings,
        });
    }
    Ok(plan)
}

/// Submit one job that exports the planned sources in order, one file each. Every
/// destination is admitted before the first render. Cancellation stops the file in
/// progress, whose staged output is removed; files already published stay complete.
pub fn submit_asset_exports(
    runner: &mut JobRunner,
    input: CacheKey,
    plan: Vec<AssetExport>,
) -> Result<(JobId, CancelToken)> {
    submit_asset_exports_with_sources(runner, input, plan, || {
        Box::new(render::FileSources::default())
    })
}

/// [`submit_asset_exports`] with the caller's render source seam. Each output asks for its own
/// provider, so the seam sees every file the job reads.
pub fn submit_asset_exports_with_sources(
    runner: &mut JobRunner,
    input: CacheKey,
    plan: Vec<AssetExport>,
    sources: impl Fn() -> Box<dyn render::SourceProvider> + Send + Sync + 'static,
) -> Result<(JobId, CancelToken)> {
    let count = plan.len();
    runner.submit::<RuntimeError>(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::GpuEncoder,
            input,
            bytes: JobBytes::default(),
        },
        move |context| {
            for export in &plan {
                render::validate_output_path(&export.project, &export.path, context)?;
                drop(project::atomic::StreamingStage::new(
                    &export.path,
                    export.settings.overwrite,
                    export.settings.max_bytes,
                )?);
            }
            for (index, export) in plan.iter().enumerate() {
                let hooks = AssetProgress {
                    context,
                    index,
                    count,
                };
                render::export_with_sources(
                    &export.project,
                    export.range,
                    &export.settings,
                    &export.path,
                    sources().as_mut(),
                    &hooks,
                )?;
            }
            Ok(Vec::new())
        },
    )
}

/// Maps one file's render progress onto the whole job, so the job's fraction is monotonic.
struct AssetProgress<'a> {
    context: &'a JobContext,
    index: usize,
    count: usize,
}

impl render::ExecutionHooks for AssetProgress<'_> {
    fn cancelled(&self) -> bool {
        self.context.cancelled()
    }
    fn parallelism(&self) -> usize {
        render::ExecutionHooks::parallelism(self.context)
    }
    fn run_tasks(
        &self,
        tasks: &mut [&mut (dyn FnMut() -> render::Result<()> + Send)],
    ) -> render::Result<()> {
        render::ExecutionHooks::run_tasks(self.context, tasks)
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        render::ExecutionHooks::reserve(self.context, kind, bytes)
    }
    fn begin_publish(&self) -> render::Result<()> {
        render::ExecutionHooks::begin_publish(self.context)
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        let whole = (self.index as f32 + fraction) / self.count.max(1) as f32;
        render::ExecutionHooks::progress(self.context, stage, whole)
    }
}

/// `<stem>.mp4`, then `<stem>-2.mp4` and onward for repeats. Compared case-insensitively,
/// because the default Windows and macOS volumes are case-insensitive.
fn unique_export_name(stem: &str, taken: &mut std::collections::BTreeSet<String>) -> String {
    let mut index = 1usize;
    loop {
        let base = if index == 1 {
            stem.to_owned()
        } else {
            format!("{stem}-{index}")
        };
        let name = super::default_file_name(&base, "mp4");
        if taken.insert(name.to_lowercase()) {
            return name;
        }
        index += 1;
    }
}

/// A one-item project over one asset's whole source. It keeps the source canvas, rate,
/// LUTs and settings, and none of the source timeline.
fn full_source_project(
    source: &project::Project,
    asset: &media::SourceAsset,
    range: time::TimeRange,
) -> Result<project::Project> {
    let data = source.data();
    let timeline = project::Timeline {
        main: project::Track {
            id: project::TrackId(2),
            name: "Main Track".into(),
            kind: project::TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![project::Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: project::ItemId(1),
                source_asset: Some(asset.id()),
                source_range: range,
                start: time::Tick::ZERO,
                content: project::ItemParams::Video(project::VideoParams {
                    source_audio_enabled: true,
                    ..project::VideoParams::plain()
                }),
            }],
        },
        ..Default::default()
    };
    let scenes = vec![project::Scene {
        id: project::SceneId(0),
        name: "Main scene".into(),
        is_main: true,
        timeline,
        bookmarks: Vec::new(),
    }];
    let scenes: project::Scenes = scenes.try_into()?;
    Ok(project::Project::new(project::ProjectData {
        id: data.id,
        name: data.name.clone(),
        canvas: data.canvas.clone(),
        original_canvas_size: data.original_canvas_size,
        assets: vec![asset.clone()],
        luts: data.luts.clone(),
        scenes,
        current_scene_id: project::SceneId(0),
        settings: data.settings.clone(),
        caption_presets: data.caption_presets.clone(),
        text_presets: data.text_presets.clone(),
        recent_colors: data.recent_colors.clone(),
    })?)
}

impl crate::Session {
    /// Dispatch a registered export command against this immutable snapshot.
    /// Export does not edit history; gestures/batches cannot publish files.
    pub fn submit_export_command(
        &self,
        runner: &mut JobRunner,
        input: CacheKey,
        command: crate::Command,
    ) -> Result<(JobId, CancelToken)> {
        if self.transaction_active() {
            return Err(RuntimeError::Edit(
                crate::session::SessionError::TransactionActive.into(),
            ));
        }
        match command {
            crate::Command::BurnSubtitles {
                range,
                languages,
                style,
                portrait,
                path,
                overwrite,
            } => {
                let project = self.project().clone();
                if !project.data().scene().timeline.iter().any(|t| {
                    !t.hidden
                        && t.items.iter().any(|i| {
                            matches!(i.content, project::ItemParams::Caption(_))
                                && i.start < range.end()
                                && i.end().is_ok_and(|end| end > range.start())
                        })
                }) {
                    return Err(RuntimeError::Edit(crate::EditError::Invalid(
                        "no visible caption item in burn range",
                    )));
                }
                let mut settings = render::ExportSettings::from_project(&project);
                if portrait {
                    settings.render = render::RenderSettings::portrait(&project);
                }
                settings.render.caption_languages = languages;
                settings.render.caption_style = style;
                render::Renderer::new(&project, settings.render.clone())?;
                settings.mode = render::ExportMode::Encode;
                settings.overwrite = overwrite_policy(overwrite);
                submit_export(runner, input, project, range, settings, path)
            }
            crate::Command::ExportSubtitles {
                track,
                format,
                languages,
                path,
                overwrite,
            } => {
                let project = self.project().clone();
                let selected = project
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .find(|t| t.id == track)
                    .ok_or(RuntimeError::Edit(crate::EditError::NoTrack(track)))?;
                if selected.kind != project::TrackKind::Text {
                    return Err(RuntimeError::Edit(crate::EditError::Invalid(
                        "subtitle export requires a caption track",
                    )));
                }
                if languages.is_empty()
                    || languages.len() > 2
                    || (languages.len() == 2 && languages.first() == languages.get(1))
                {
                    return Err(RuntimeError::Edit(crate::EditError::Invalid(
                        "select one language or two distinct languages",
                    )));
                }
                // Bound mapping, serializer repair clones, escaped output and cue metadata
                // before allocation. Snapshot text is already owned by the Session.
                let text_bytes = selected
                    .items
                    .iter()
                    .try_fold(0u64, |n, item| {
                        let project::ItemParams::Caption(params) = &item.content else {
                            return Some(n);
                        };
                        params.cues.cues.iter().try_fold(n, |n, cue| {
                            cue.texts.iter().try_fold(
                                n.checked_add(
                                    (cue.id.as_str().len() + cue.source_lang.as_str().len()) as u64
                                        + 1024,
                                )?,
                                |n, (language, t)| {
                                    n.checked_add(
                                        (t.text.as_str().len()
                                            + language.as_str().len()
                                            + t.translation_ref
                                                .as_ref()
                                                .map_or(0, |r| r.as_str().len()))
                                            as u64
                                            + 256,
                                    )
                                },
                            )
                        })
                    })
                    .ok_or(RuntimeError::Edit(
                        crate::session::wire::ControlError::Limit.into(),
                    ))?;
                let bytes = text_bytes
                    .checked_mul(16)
                    .and_then(|n| n.checked_add(4096))
                    .ok_or(RuntimeError::Edit(
                        crate::session::wire::ControlError::Limit.into(),
                    ))?;
                Ok(runner.submit::<RuntimeError>(
                    JobSpec {
                        kind: JobKind::Export,
                        class: ResourceClass::Cpu,
                        input,
                        bytes: JobBytes {
                            export: bytes,
                            ..Default::default()
                        },
                    },
                    move |context| {
                        use std::io::Write;
                        context.progress(JobStage::Admit, 0.0)?;
                        render::validate_output_path(&project, &path, context)?;
                        let output = subtitle_file(&project, track, format, &languages)?;
                        context.progress(JobStage::Finalize, 0.5)?;
                        if context.cancelled() {
                            return Err(RuntimeError::Cancelled);
                        }
                        let stage = project::atomic::StreamingStage::new(
                            &path,
                            overwrite_policy(overwrite),
                            bytes,
                        )?;
                        {
                            let mut writer = stage.writer()?;
                            for chunk in output.text.as_bytes().chunks(65536) {
                                if context.cancelled() {
                                    return Err(RuntimeError::Cancelled);
                                }
                                writer.write_all(chunk)?;
                            }
                        }
                        let finalized = stage.finalize()?;
                        // No fallible work after publish; warnings are available from subtitle_file.
                        context.progress(JobStage::Publish, 0.95)?;
                        context.begin_publication()?;
                        finalized.publish()?;
                        Ok(Vec::new())
                    },
                )?)
            }
            _ => Err(RuntimeError::Edit(crate::EditError::Invalid(
                "command is not an export command",
            ))),
        }
    }
    /// Typed export entry point; the caller supplies the admitted command.
    pub fn execute_export(
        &self,
        runner: &mut JobRunner,
        input: CacheKey,
        command: crate::Command,
    ) -> Result<(JobId, CancelToken)> {
        self.submit_export_command(runner, input, command)
    }
}
fn overwrite_policy(replace: bool) -> project::atomic::Overwrite {
    if replace {
        project::atomic::Overwrite::Replace
    } else {
        project::atomic::Overwrite::Refuse
    }
}
/// Serialize a caption track's retained source windows at their output positions.
/// Reuses captions' mapping, bilingual ordering, repair and warnings; never opens media.
/// Callers can inspect warnings before submitting the identical file export job.
pub fn subtitle_file(
    project: &project::Project,
    track: project::TrackId,
    format: captions::Format,
    languages: &[captions::LanguageTag],
) -> Result<captions::Export> {
    let track = project
        .data()
        .scene()
        .timeline
        .iter()
        .find(|t| t.id == track)
        .ok_or(RuntimeError::Edit(crate::EditError::NoTrack(track)))?;
    if track.kind != project::TrackKind::Text {
        return Err(RuntimeError::Edit(crate::EditError::Invalid(
            "subtitle export requires a caption track",
        )));
    }
    let mut mapped = captions::CueTrack::default();
    for item in &track.items {
        let project::ItemParams::Caption(params) = &item.content else {
            continue;
        };
        let mapping = captions::EditMapping::new(vec![(item.source_range, item.start)])?;
        for mut cue in mapping.map_cues(&params.cues)?.cues {
            // Mapping occurrence identity is local to an item's source track.
            cue.id = format!("{}:{}", item.id.0, cue.id.as_str()).into();
            mapped.cues.push(cue);
        }
    }
    Ok(captions::export(&mapped, format, languages)?)
}
