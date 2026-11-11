//! Executes admitted control commands.
use super::{Caller, output};
use crate::jobs::{JobKind, JobRunner, JobState};
use crate::providers::{CatalogOperation, ProviderOption};
use crate::session::wire::{
    ControlResult, JobView, MAX_RESPONSE_BYTES, ProviderView, bounded_json, validate_wire,
};
use crate::{Command, EditError, Result, Session, registry};

impl From<ProviderOption> for ProviderView {
    fn from(p: ProviderOption) -> Self {
        Self {
            operation: match p.operation {
                CatalogOperation::Transcribe => "transcribe",
                CatalogOperation::Translate => "translate",
                CatalogOperation::SuggestClips => "suggest_clips",
                CatalogOperation::GenerateChapters => "generate_chapters",
                CatalogOperation::ClassifyFillers => "classify_fillers",
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
fn font_access<'a>(caller: &Caller<'a>) -> Result<&'a super::FontAccess<'a>> {
    caller.fonts.ok_or(crate::FontError::Unavailable.into())
}
fn sound_access<'a>(caller: &Caller<'a>) -> Result<&'a super::SoundAccess<'a>> {
    caller
        .sounds
        .ok_or(EditError::Unsupported("sound library unavailable").into())
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
                    return Err(EditError::TransactionActive);
                }
                Ok(ControlResult::Clipboard {
                    payload: self.clipboard_payload(),
                })
            }
            Command::Inspect {} => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
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
                    return Err(EditError::TransactionActive);
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
            Command::FontsDownload {
                family,
                accept_license,
            } => {
                let job = (font_access(caller)?.download)(runner, family.as_str(), accept_license)?;
                Ok(ControlResult::JobAccepted {
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
            Command::JobStatus { job } => {
                let mut snapshot = JobView::from(
                    runner
                        .snapshot(job)
                        .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?,
                );
                if matches!(
                    snapshot.kind,
                    JobKind::GenerateWaveform
                        | JobKind::GenerateClipThumbnails
                        | JobKind::DetectScenes
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
                    return Err(EditError::TransactionActive);
                }
                let asset = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset)
                    .cloned()
                    .ok_or(EditError::NoAsset(asset))?;
                let media = caller
                    .media
                    .ok_or(EditError::Unsupported("media generation cache unavailable"))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .generate_waveform(runner, asset)?;
                Ok(ControlResult::JobAccepted {
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
                    return Err(EditError::TransactionActive);
                }
                let data = self.project().data();
                let clip = data
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| &track.items)
                    .find(|candidate| candidate.id == item)
                    .cloned()
                    .ok_or(EditError::NoItem(item))?;
                let asset_id = clip
                    .source_asset
                    .ok_or(EditError::Invalid("thumbnail clip has no asset"))?;
                let asset = data
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset_id)
                    .cloned()
                    .ok_or(EditError::NoAsset(asset_id))?;
                let media = caller
                    .media
                    .ok_or(EditError::Unsupported("media generation cache unavailable"))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .generate_clip_thumbnails(runner, asset, &clip, visible, pixels_per_second)?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::DetectScenes { asset, settings } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                let asset = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|candidate| candidate.id() == asset)
                    .cloned()
                    .ok_or(EditError::NoAsset(asset))?;
                let media = caller
                    .media
                    .ok_or(EditError::Unsupported("media generation cache unavailable"))?;
                let job = media
                    .lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .detect_scenes(runner, asset, settings)?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::MeasureLoudness {} => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                let job = crate::loudness::submit_loudness(runner, self.project().clone(), || {
                    Box::new(render::FileSources::default())
                })?;
                Ok(ControlResult::JobAccepted {
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
                    return Err(EditError::TransactionActive);
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
                    return Err(EditError::TransactionActive);
                }
                let access = caller
                    .chapters
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let request =
                    crate::chapters::ChapterRequest::new(self.project(), language, max_chapters)?;
                let job = request.submit(runner, access)?;
                Ok(ControlResult::JobAccepted {
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
                    return Err(EditError::TransactionActive);
                }
                let access = caller
                    .chapters
                    .ok_or(crate::providers::Error::ConsentRequired)?;
                let request =
                    crate::fillers::FillerRequest::new(self.project(), language, include_markers)?;
                let job = request.submit(runner, access)?;
                Ok(ControlResult::JobAccepted {
                    job: job.id,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::ChaptersExportSidecars { dir, document } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
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
                    return Err(EditError::TransactionActive);
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
            } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
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
                    crate::control_preview::control_key(fingerprint)?,
                )?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: Vec::new(),
                })
            }
            Command::CaptionsApply { job, start } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                let snapshot = runner
                    .snapshot(job)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                if snapshot.kind != JobKind::Transcribe {
                    return Err(EditError::Invalid("job is not a transcription"));
                }
                if snapshot.state != JobState::Completed {
                    return Err(EditError::StaleResult);
                }
                runner.validate_output_target(job, self)?;
                let result = crate::subtitles::decode_captions(
                    runner
                        .output_bytes(job)
                        .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?,
                )?;
                let asset = self
                    .project()
                    .data()
                    .assets
                    .iter()
                    .find(|a| a.id() == result.asset)
                    .ok_or(EditError::NoAsset(result.asset))?;
                if result.range.start() < time::Tick::ZERO
                    || asset
                        .probe()
                        .duration
                        .is_none_or(|d| result.range.end() > d)
                {
                    return Err(EditError::StaleResult);
                }
                crate::subtitles::validate_source(asset)?;
                let insert =
                    crate::subtitles::apply_command(result, start.unwrap_or(time::Tick::ZERO))?;
                self.apply_control(insert)
                    .map(|outcome| ControlResult::Edit { outcome })
            }
            Command::JobCancel { job } => {
                runner.cancel_job(job)?;
                Ok(ControlResult::CancelRequested { job })
            }
            Command::Preview { frames } => crate::control_preview::preview(self, frames, runner),
            Command::Snapshot { time, path } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
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
                    return Err(EditError::TransactionActive);
                }
                let mut settings = render::ExportSettings::from_project(self.project());
                settings.container = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .and_then(media::Container::from_extension)
                    .ok_or(EditError::Invalid("export container extension"))?;
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
                    _ => return Err(EditError::InvalidParams),
                };
                let (job, _) = self.submit_export_command(
                    runner,
                    crate::control_preview::control_key(fingerprint)?,
                    command,
                )?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: vec![path],
                })
            }
            command => self
                .apply_control(command)
                .map(|outcome| ControlResult::Edit { outcome }),
        }
    }
}
