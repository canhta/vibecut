//! Executes admitted control commands.
use super::{Caller, output};
use crate::jobs::{JobKind, JobRunner, JobState};
use crate::providers::{CatalogOperation, ProviderOption};
use crate::session::wire::{
    ControlResult, JobView, MAX_RESPONSE_BYTES, ProviderView, bounded_json, validate_wire,
};
use crate::{Command, Session, control::Error as EngineError, control::Result, registry};

impl From<ProviderOption> for ProviderView {
    fn from(p: ProviderOption) -> Self {
        Self {
            operation: match p.operation {
                CatalogOperation::Transcribe => "transcribe",
                CatalogOperation::Translate => "translate",
                CatalogOperation::SuggestClips => "suggest_clips",
                CatalogOperation::GenerateChapters => "generate_chapters",
                CatalogOperation::ClassifyFillers => "classify_fillers",
                CatalogOperation::SelectEmphasis => "select_emphasis",
            }
            .into(),
            provider: p.id.into(),
            models: p.models,
            remote: !p.local,
            native_timing: p.native_timing,
            languages: p.languages,
            target_languages: p.target_languages,
        }
    }
}
fn font_access<'a>(
    caller: &Caller<'a>,
) -> std::result::Result<&'a super::FontAccess<'a>, crate::FontError> {
    caller.fonts.ok_or(crate::FontError::Unavailable)
}
fn clip_access<'a>(
    caller: &Caller<'a>,
) -> Result<(
    &'a crate::subtitles::CaptionAccess,
    &'a std::sync::Arc<dyn super::ClipWorkflow>,
)> {
    let unavailable =
        || EngineError::Edit(crate::EditError::Unsupported("clip workflow unavailable"));
    let access = caller.captions.ok_or_else(unavailable)?;
    let workflow = access.clips().ok_or_else(unavailable)?;
    Ok((access, workflow))
}
fn sound_access<'a>(caller: &Caller<'a>) -> crate::session::Result<&'a super::SoundAccess<'a>> {
    caller.sounds.ok_or(crate::session::EditError::Unsupported(
        "sound library unavailable",
    ))
}
fn music_access<'a>(caller: &Caller<'a>) -> crate::session::Result<&'a super::MusicAccess<'a>> {
    caller.music.ok_or(crate::session::EditError::Unsupported(
        "music library unavailable",
    ))
}

impl Session {
    pub(super) fn run_control(
        &mut self,
        command: Command,
        runner: &mut JobRunner,
        fingerprint: blake3::Hash,
        caller: &Caller<'_>,
    ) -> Result<ControlResult> {
        match command {
            Command::ReadClipboard {} => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                Ok(ControlResult::Clipboard {
                    payload: self.clipboard_payload(),
                })
            }
            Command::ImportLut {
                path,
                name,
                licence,
                expected_hash,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let expected = expected_hash
                    .map(|id| {
                        self.project()
                            .data()
                            .luts
                            .iter()
                            .find(|r| r.id == id)
                            .cloned()
                            .ok_or(EngineError::Edit(crate::EditError::Invalid(
                                "unregistered LUT relink",
                            )))
                    })
                    .transpose()?;
                let store = caller
                    .media
                    .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                        "LUT source store unavailable",
                    )))?
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .lut_store();
                let project = self.project().data().id;
                let job = crate::artifacts::lut::submit(
                    runner,
                    store,
                    project,
                    path,
                    name,
                    licence,
                    expected,
                    fingerprint,
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::Inspect {} => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                validate_wire(self.project().data())?;
                Ok(ControlResult::Inspection {
                    project: Box::new(self.project().data().clone()),
                })
            }
            Command::Catalog {} => {
                bounded_json(registry(), MAX_RESPONSE_BYTES - 4096)?;
                Ok(ControlResult::Catalog {
                    entries: registry().to_vec(),
                })
            }
            Command::ProviderCatalog {} => Ok(ControlResult::Providers {
                entries: crate::providers::provider_options()
                    .into_iter()
                    .map(ProviderView::from)
                    .collect(),
            }),
            Command::FontsList { query } => Ok(ControlResult::Fonts {
                entries: (font_access(caller)?.list)(
                    query.as_ref().map(captions::NfcText::as_str),
                )?,
            }),
            Command::FontsLoad { family } => Ok(ControlResult::Font {
                font: (font_access(caller)?.load)(family.as_str())?,
            }),
            Command::FillerCandidates {
                language,
                include_markers,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let candidates = crate::session::filler::scene_candidates(
                    self.project().data(),
                    Some(&language),
                    include_markers,
                )?
                .into_iter()
                .map(Into::into)
                .collect();
                Ok(ControlResult::Fillers { candidates })
            }
            Command::FontsImport {
                path,
                accept_license,
            } => {
                let job = (font_access(caller)?.import)(runner, &path, accept_license)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::FontsDownload {
                family,
                accept_license,
            } => {
                let job = (font_access(caller)?.download)(runner, family.as_str(), accept_license)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::SoundsSearch {
                query,
                page,
                page_size,
                commercial_only,
            } => Ok(ControlResult::SoundSearch {
                page: (sound_access(caller)?.search)(&super::SoundQuery {
                    query,
                    page,
                    page_size,
                    commercial_only,
                })?,
            }),
            Command::SoundsSave { sound } => Ok(ControlResult::SoundSaved {
                saved: (sound_access(caller)?.save)(&sound)?,
            }),
            Command::SoundsUnsave { id } => Ok(ControlResult::SoundUnsaved {
                id,
                removed: (sound_access(caller)?.unsave)(id)?,
            }),
            Command::MusicSearch {
                query,
                tag,
                page,
                page_size,
                saved,
            } => Ok(ControlResult::MusicSearch {
                page: (music_access(caller)?.search)(&super::MusicQuery {
                    query,
                    tag,
                    page,
                    page_size,
                    saved,
                })?,
            }),
            Command::MusicSave { music } => Ok(ControlResult::MusicSaved {
                saved: (music_access(caller)?.save)(&music)?,
            }),
            Command::MusicUnsave { id } => {
                let removed = (music_access(caller)?.unsave)(&id)?;
                Ok(ControlResult::MusicUnsaved { id, removed })
            }
            Command::JobStatus { job } => {
                let mut snapshot = JobView::from(
                    runner
                        .snapshot(job)
                        .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?,
                );
                if matches!(
                    snapshot.kind,
                    JobKind::GenerateWaveform
                        | JobKind::GenerateClipThumbnails
                        | JobKind::DetectScenes
                        | JobKind::AnalyzeReframe
                ) && let Some(media) = caller.media
                {
                    let mut media = media.lock().map_err(|_| crate::RuntimeError::Poisoned)?;
                    match snapshot.state {
                        JobState::Completed => media.finish(runner, job)?,
                        JobState::Cancelled | JobState::Failed => media.forget(job),
                        _ => {}
                    }
                }
                if snapshot.state == JobState::Completed {
                    snapshot.output = output::decode(snapshot.kind, runner.output_bytes(job))?;
                }
                Ok(ControlResult::Job { snapshot })
            }
            Command::GenerateWaveform { asset } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let asset = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoAsset(asset)))?;
                let media =
                    caller
                        .media
                        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                            "media generation cache unavailable",
                        )))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .generate_waveform(runner, asset)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::GenerateClipThumbnails {
                item,
                visible,
                pixels_per_second,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let data = self.project().data();
                let clip = data
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| &track.items)
                    .find(|candidate| candidate.id == item)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoItem(item)))?;
                let asset_id =
                    clip.source_asset
                        .ok_or(EngineError::Edit(crate::EditError::Invalid(
                            "thumbnail clip has no asset",
                        )))?;
                let asset = data
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset_id)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoAsset(asset_id)))?;
                let media =
                    caller
                        .media
                        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                            "media generation cache unavailable",
                        )))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .generate_clip_thumbnails(runner, asset, &clip, visible, pixels_per_second)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::DetectScenes { asset, settings } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let asset = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoAsset(asset)))?;
                let media =
                    caller
                        .media
                        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                            "media generation cache unavailable",
                        )))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .detect_scenes(runner, asset, settings)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::MeasureLoudness {} => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let job = crate::loudness::submit_loudness(runner, self.project().clone(), || {
                    Box::new(render::FileSources::default())
                })?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::PauseCandidates {
                min_pause_ms,
                pad_ms,
                crossfade_ms,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let settings = crate::pause_candidates::PauseSettings {
                    min_pause_ms,
                    pad_ms,
                    crossfade_ms,
                };
                let request = crate::pause_candidates::PauseCandidatesRequest::new(
                    self.project().clone(),
                    settings,
                )?;
                let job = request.submit(runner)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job: job.id,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::ChaptersGenerate {
                language,
                max_chapters,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let access = caller
                    .chapters
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let request =
                    crate::chapters::ChapterRequest::new(self.project(), language, max_chapters)?;
                let job = request.submit(runner, access)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job: job.id,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::FillersClassify {
                language,
                include_markers,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let access = caller
                    .chapters
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let request =
                    crate::fillers::FillerRequest::new(self.project(), language, include_markers)?;
                let job = request.submit(runner, access)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job: job.id,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::CaptionsSuggestEmphasis { item, cue_ids } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let access = caller
                    .chapters
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let request = crate::emphasis::EmphasisRequest::new(
                    self.project(),
                    item,
                    cue_ids.as_deref(),
                )?;
                let job = request.submit(runner, access)?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job: job.id,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::ChaptersExportSidecars { dir, document } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let protected = self.protected_paths().to_vec();
                let paths = crate::chapter_sidecars::write_to_directory(
                    self.project(),
                    &protected,
                    &dir,
                    &document,
                )?;
                Ok(ControlResult::ChapterSidecars { paths })
            }
            Command::CaptionsGenerate {
                asset,
                range,
                source_language,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let access = caller
                    .captions
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let job = crate::subtitles::generate(
                    self,
                    runner,
                    access,
                    asset,
                    range,
                    source_language,
                    crate::control_preview::control_key(fingerprint)?,
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::CaptionsTranslate {
                item,
                cues,
                target_language,
                force,
                glossary,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let access = caller
                    .captions
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let job = crate::subtitles::translate(
                    self,
                    runner,
                    access,
                    item,
                    cues,
                    target_language,
                    force,
                    glossary,
                    crate::control_preview::control_key(fingerprint)?,
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::ClipsSuggest {
                asset,
                source_language,
                provider,
                model,
                count,
                min_duration,
                max_duration,
                max_segments,
                min_segment_duration,
                overlap_tolerance,
                keywords,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let (access, workflow) = clip_access(caller)?;
                let source = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoAsset(asset)))?;
                let job = workflow.suggest(
                    runner,
                    access,
                    self,
                    super::ClipsSuggestRequest {
                        asset: source,
                        source_language,
                        provider,
                        model,
                        count,
                        min_duration,
                        max_duration,
                        max_segments,
                        min_segment_duration,
                        overlap_tolerance,
                        keywords,
                    },
                    crate::control_preview::control_key(fingerprint)?,
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::ClipsExport {
                job,
                select,
                item,
                languages,
                formats,
                directory,
                overwrite,
                allow_degraded,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let (_, workflow) = clip_access(caller)?;
                let snapshot = runner
                    .snapshot(job)
                    .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?;
                if snapshot.kind != JobKind::SuggestClips {
                    return Err(EngineError::Edit(crate::EditError::Invalid(
                        "job is not a clip suggestion",
                    )));
                }
                if snapshot.state != JobState::Completed {
                    return Err(EngineError::Session(crate::SessionError::StaleResult));
                }
                // The suggestion is bound to its source asset, not to this session's
                // revision: callers caption and translate the cues in between.
                let result = super::decode_clips(
                    runner
                        .output_bytes(job)
                        .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?,
                )?;
                let cues = match item {
                    None => captions::CueTrack::default(),
                    Some(item) => {
                        let found = self
                            .project()
                            .data()
                            .scene()
                            .timeline
                            .iter()
                            .flat_map(|track| &track.items)
                            .find(|candidate| candidate.id == item)
                            .ok_or(EngineError::Edit(crate::EditError::NoItem(item)))?;
                        let project::ItemParams::Caption(params) = &found.content else {
                            return Err(EngineError::Edit(crate::EditError::Invalid(
                                "clip captions need a caption item",
                            )));
                        };
                        params.cues.clone()
                    }
                };
                let shorts = workflow.export(
                    runner,
                    self.project().as_ref(),
                    super::ClipsExportRequest {
                        result,
                        cues,
                        select,
                        languages,
                        formats,
                        directory,
                        overwrite,
                        allow_degraded,
                    },
                )?;
                Ok(ControlResult::ClipExports {
                    revision: self.revision(),
                    shorts,
                })
            }
            Command::CaptionsApply { job, start } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let snapshot = runner
                    .snapshot(job)
                    .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?;
                if snapshot.kind != JobKind::Transcribe {
                    return Err(EngineError::Edit(crate::EditError::Invalid(
                        "job is not a transcription",
                    )));
                }
                if snapshot.state != JobState::Completed {
                    return Err(EngineError::Session(crate::SessionError::StaleResult));
                }
                runner.validate_output_target(job, self)?;
                let result = crate::subtitles::decode_captions(
                    runner
                        .output_bytes(job)
                        .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?,
                )?;
                let asset = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|a| a.id() == result.asset)
                    .ok_or(EngineError::Edit(crate::EditError::NoAsset(result.asset)))?;
                if result.range.start() < time::Tick::ZERO
                    || asset
                        .probe()
                        .duration
                        .is_none_or(|d| result.range.end() > d)
                {
                    return Err(EngineError::Session(crate::SessionError::StaleResult));
                }
                crate::subtitles::validate_source(asset)?;
                let insert =
                    crate::subtitles::apply_command(result, start.unwrap_or(time::Tick::ZERO))?;
                self.apply_control(insert)
                    .map(|outcome| ControlResult::Edit { outcome })
                    .map_err(EngineError::from)
            }
            Command::JobCancel { job } => {
                runner.cancel_job(job)?;
                Ok(ControlResult::CancelRequested { job })
            }
            Command::Preview { frames } => {
                Ok(crate::control_preview::preview(self, frames, runner)?)
            }
            Command::Snapshot { time, path } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let protected = self.protected_paths().to_vec();
                let snapshot = crate::snapshot::save_with_overwrite(
                    self.project(),
                    time,
                    &path,
                    &protected,
                    runner.memory_budget(),
                    if caller.policy.snapshot_overwrite() {
                        project::atomic::Overwrite::Replace
                    } else {
                        project::atomic::Overwrite::Refuse
                    },
                )?;
                Ok(ControlResult::Snapshot { snapshot })
            }
            Command::Export {
                settings: options,
                path,
                overwrite,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let mut settings = render::ExportSettings::from_project(self.project());
                settings.container = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .and_then(media::Container::from_extension)
                    .ok_or(EngineError::Edit(crate::EditError::Invalid(
                        "export container extension",
                    )))?;
                let paths = options.paths(self.project(), &path)?;
                for output in &paths {
                    caller.policy.authorize(
                        &Command::Export {
                            settings: options,
                            path: output.clone(),
                            overwrite,
                        },
                        self.project(),
                    )?;
                }
                settings.overwrite = if overwrite {
                    project::atomic::Overwrite::Replace
                } else {
                    project::atomic::Overwrite::Refuse
                };
                let word_highlight_warnings = options.caption_word_warnings(
                    self.project(),
                    &settings,
                    &crate::ByteBudgetHooks::new(runner.memory_budget(), None),
                )?;
                let (job, _) = crate::submit_scene_exports_with_sources(
                    runner,
                    crate::control_preview::control_key(fingerprint)?,
                    self.project().clone(),
                    options,
                    settings,
                    paths.clone(),
                    || Box::new(render::FileSources::default()),
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings,
                    job,
                    revision: self.revision(),
                    paths,
                })
            }
            Command::ExportClips {
                asset_ids,
                dir,
                overwrite,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let plan = crate::plan_asset_exports(self.project(), &asset_ids, &dir, overwrite)?;
                let paths = plan.iter().map(|export| export.path.clone()).collect();
                let (job, _) = crate::submit_asset_exports(
                    runner,
                    crate::control_preview::control_key(fingerprint)?,
                    plan,
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths,
                })
            }
            command @ (Command::ExportSubtitles { .. } | Command::BurnSubtitles { .. }) => {
                let path = match &command {
                    Command::ExportSubtitles { path, .. } | Command::BurnSubtitles { path, .. } => {
                        path.clone()
                    }
                    _ => {
                        return Err(EngineError::Control(
                            crate::session::wire::ControlError::InvalidParams,
                        ));
                    }
                };
                let word_highlight_warnings = if let Command::BurnSubtitles {
                    languages,
                    style,
                    portrait,
                    ..
                } = &command
                {
                    let mut settings = if *portrait {
                        render::RenderSettings::portrait(self.project())
                    } else {
                        render::RenderSettings::from_project(self.project())
                    };
                    settings.caption_languages = languages.clone();
                    settings.caption_style = style.clone();
                    render::Renderer::new(self.project(), settings)?.caption_word_warnings(
                        None,
                        &crate::ByteBudgetHooks::new(runner.memory_budget(), None),
                    )?
                } else {
                    project::CaptionWordWarnings::default()
                };
                let (job, _) = self.submit_export_command(
                    runner,
                    crate::control_preview::control_key(fingerprint)?,
                    command,
                )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings,
                    job,
                    revision: self.revision(),
                    paths: vec![path],
                })
            }
            Command::ReframeGenerate { item, target } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let data = self.project().data();
                let clip = data
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| &track.items)
                    .find(|candidate| candidate.id == item)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoItem(item)))?;
                if !matches!(clip.content, project::ItemParams::Video(_)) {
                    return Err(EngineError::Edit(crate::EditError::Invalid(
                        "reframe needs a video item",
                    )));
                }
                let asset_id =
                    clip.source_asset
                        .ok_or(EngineError::Edit(crate::EditError::Invalid(
                            "reframe item has no source",
                        )))?;
                let asset = data
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset_id)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoAsset(asset_id)))?;
                let vision =
                    caller
                        .vision
                        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                            "reframe detector unavailable",
                        )))?;
                let (detector, models) = vision.load()?;
                let media =
                    caller
                        .media
                        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                            "media generation cache unavailable",
                        )))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .generate_reframe(
                        runner,
                        self,
                        asset,
                        clip,
                        target.into(),
                        models,
                        crate::AnalysisSettings::default(),
                        detector,
                    )?;
                Ok(ControlResult::JobAccepted {
                    word_highlight_warnings: project::CaptionWordWarnings::default(),
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::ReframeApply {
                job,
                replace_manual,
            } => {
                if self.transaction_active() {
                    return Err(EngineError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                let snapshot = runner
                    .snapshot(job)
                    .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?;
                if snapshot.kind != JobKind::AnalyzeReframe {
                    return Err(EngineError::Edit(crate::EditError::Invalid(
                        "job is not a reframe analysis",
                    )));
                }
                if snapshot.state != JobState::Completed {
                    return Err(EngineError::Session(crate::SessionError::StaleResult));
                }
                runner.validate_output_target(job, self)?;
                let media =
                    caller
                        .media
                        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
                            "media generation cache unavailable",
                        )))?;
                let item_id = {
                    let mut media = media.lock().map_err(|_| crate::RuntimeError::Poisoned)?;
                    let item = media
                        .reframe_item(job)
                        .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?;
                    media.finish(runner, job)?;
                    item
                };
                let bytes = runner
                    .output_bytes(job)
                    .ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let analysis: crate::ReframeAnalysis =
                    serde_json::from_slice(bytes).map_err(|_| {
                        EngineError::Edit(crate::EditError::Invalid("reframe result is invalid"))
                    })?;
                let item = self
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| &track.items)
                    .find(|candidate| candidate.id == item_id)
                    .cloned()
                    .ok_or(EngineError::Edit(crate::EditError::NoItem(item_id)))?;
                let keys = crate::emit_reframe(&analysis, &item)?;
                let (crop, x, y, auto) =
                    crate::analysis::merge_reframe(&item, keys, replace_manual)?;
                // One atomic batch: one undo step, and the outcome the caller reports.
                let mut commands = vec![Command::SetItemField {
                    item: item_id,
                    patch: crate::ItemParamPatch::Crop(Some(crop)),
                }];
                for (path, channel) in [
                    (project::BuiltinPath::CropX, x),
                    (project::BuiltinPath::CropY, y),
                    (project::BuiltinPath::CropAuto, auto),
                ] {
                    commands.push(Command::SetAnimatedParam {
                        item: item_id,
                        path: project::AnimationPath::from(path),
                        param: project::Param::Value(channel),
                    });
                }
                let outcome = self.apply(Command::Batch { commands })?;
                Ok(ControlResult::Edit { outcome })
            }
            command => self
                .apply_control(command)
                .map(|outcome| ControlResult::Edit { outcome })
                .map_err(EngineError::from),
        }
    }
}
