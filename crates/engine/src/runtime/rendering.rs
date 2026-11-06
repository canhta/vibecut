//! Submits bounded project export work.
use crate::cache::CacheKey;
use crate::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use crate::{ByteLease, ByteOwner, EditError};
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
        let owner = match kind {
            render::MemoryKind::Decoded => ByteOwner::DecodedFrames,
            render::MemoryKind::Compositor => ByteOwner::Compositor,
            render::MemoryKind::Export => ByteOwner::Export,
        };
        self.memory_budget()
            .reserve(owner, bytes)
            .map(|lease| Box::new(lease) as Box<dyn render::Reservation>)
            .map_err(|error| match error {
                EditError::Io(source) => render::Error::Io(source),
                _ => render::Error::Budget {
                    requested: bytes,
                    limit: self
                        .memory_budget()
                        .snapshot()
                        .map_or(0, |s| s.limit.saturating_sub(s.used)),
                },
            })
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
            if matches!(e, EditError::Cancelled) {
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
) -> Result<(JobId, CancelToken), EditError> {
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
) -> Result<(JobId, CancelToken), EditError> {
    super::loudness_admission::admit(&project)?;
    runner.submit(
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
) -> Result<(JobId, CancelToken), EditError> {
    let scenes = options.ranges(&project)?;
    if scenes.len() != paths.len() {
        return Err(EditError::Invalid("scene output paths"));
    }
    if paths
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != paths.len()
    {
        return Err(EditError::Invalid("duplicate scene output paths"));
    }
    for (id, _) in &scenes {
        super::loudness_admission::admit(&project.for_scene(*id)?)?;
    }
    let mut settings = settings;
    options.apply(&mut settings)?;
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
                .ok_or(EditError::Invalid("scene output path"))?
                .clone(),
            sources,
        );
    }
    runner.submit(
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
            let hooks = SceneStagingHooks(context);
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
                .write(
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
                                return Err(EditError::Cancelled);
                            }
                            let count = input.read(&mut buffer)?;
                            if count == 0 {
                                break;
                            }
                            writer.write_all(
                                buffer
                                    .get(..count)
                                    .ok_or(EditError::Invalid("scene publication read"))?,
                            )?;
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
    fn new() -> Result<Self, EditError> {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| EditError::Invalid("export staging identity"))?;
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
struct SceneStagingHooks<'a>(&'a JobContext);
impl render::ExecutionHooks for SceneStagingHooks<'_> {
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
    fn parallelism(&self) -> usize {
        render::ExecutionHooks::parallelism(self.0)
    }
    fn run_tasks(
        &self,
        tasks: &mut [&mut (dyn FnMut() -> render::Result<()> + Send)],
    ) -> render::Result<()> {
        render::ExecutionHooks::run_tasks(self.0, tasks)
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        render::ExecutionHooks::reserve(self.0, kind, bytes)
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        render::ExecutionHooks::progress(self.0, stage, fraction)
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
) -> Result<(JobId, CancelToken), EditError> {
    runner.submit(
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
            .write(
                project::atomic::Overwrite::Replace,
                size.max(1),
                fs.as_ref(),
                |writer| {
                    loop {
                        if context.cancelled() {
                            return Err(EditError::Cancelled);
                        }
                        let count = source.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        writer.write_all(
                            buffer
                                .get(..count)
                                .ok_or(EditError::Invalid("publication read size"))?,
                        )?;
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

impl crate::Session {
    /// Dispatch a registered export command against this immutable snapshot.
    /// Export does not edit history; gestures/batches cannot publish files.
    pub fn submit_export_command(
        &self,
        runner: &mut JobRunner,
        input: CacheKey,
        command: crate::Command,
    ) -> Result<(JobId, CancelToken), EditError> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
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
                    return Err(EditError::Invalid("no visible caption item in burn range"));
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
                    .ok_or(EditError::NoTrack(track))?;
                if selected.kind != project::TrackKind::Text {
                    return Err(EditError::Invalid(
                        "subtitle export requires a caption track",
                    ));
                }
                if languages.is_empty()
                    || languages.len() > 2
                    || (languages.len() == 2 && languages.first() == languages.get(1))
                {
                    return Err(EditError::Invalid(
                        "select one language or two distinct languages",
                    ));
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
                    .ok_or(EditError::Limit)?;
                let bytes = text_bytes
                    .checked_mul(16)
                    .and_then(|n| n.checked_add(4096))
                    .ok_or(EditError::Limit)?;
                runner.submit(
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
                            return Err(EditError::Cancelled);
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
                                    return Err(EditError::Cancelled);
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
                )
            }
            _ => Err(EditError::Invalid("command is not an export command")),
        }
    }
    /// Strict JSON entry point using the same registry and admission caps as edits.
    pub fn execute_export(
        &self,
        runner: &mut JobRunner,
        input: CacheKey,
        command_id: &str,
        params: serde_json::Value,
    ) -> Result<(JobId, CancelToken), EditError> {
        let command = crate::session::wire::decode_command(command_id, params)?;
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
) -> Result<captions::Export, EditError> {
    let track = project
        .data()
        .scene()
        .timeline
        .iter()
        .find(|t| t.id == track)
        .ok_or(EditError::NoTrack(track))?;
    if track.kind != project::TrackKind::Text {
        return Err(EditError::Invalid(
            "subtitle export requires a caption track",
        ));
    }
    let mut mapped = captions::CueTrack { cues: Vec::new() };
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
