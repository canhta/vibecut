//! Executes admitted control commands.
use super::Caller;
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
impl Session {
    pub(super) fn run_control(
        &mut self,
        command: Command,
        runner: &mut JobRunner,
        fingerprint: blake3::Hash,
        caller: &Caller<'_>,
    ) -> Result<ControlResult> {
        match command {
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
            Command::JobStatus { job } => {
                let mut snapshot =
                    JobView::from(runner.snapshot(job).ok_or(EditError::JobNotRetained)?);
                if snapshot.kind == JobKind::Preview
                    && snapshot.state == JobState::Completed
                    && runner.output_bytes(job).is_none()
                {
                    return Err(EditError::JobNotRetained);
                }
                if snapshot.kind == JobKind::Preview {
                    snapshot.result = runner
                        .output_bytes(job)
                        .map(serde_json::from_slice)
                        .transpose()
                        .map_err(|_| EditError::InvalidParams)?;
                }
                if snapshot.kind == JobKind::AnalyzePauses && snapshot.state == JobState::Completed
                {
                    snapshot.pauses = Some(crate::pause_candidates::decode(
                        runner.output_bytes(job).ok_or(EditError::JobNotRetained)?,
                    )?);
                }
                if matches!(
                    snapshot.kind,
                    JobKind::GenerateWaveform | JobKind::GenerateClipThumbnails
                ) && let Some(media) = caller.media
                {
                    let mut media = media.borrow_mut();
                    match snapshot.state {
                        JobState::Completed => media.finish(runner, job)?,
                        JobState::Cancelled | JobState::Failed => media.forget(job),
                        _ => {}
                    }
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
                let job = media.borrow_mut().generate_waveform(runner, asset)?;
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
                let job = media.borrow_mut().generate_clip_thumbnails(
                    runner,
                    asset,
                    &clip,
                    visible,
                    pixels_per_second,
                )?;
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
