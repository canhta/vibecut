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
fn font_store<'a>(caller: &Caller<'a>) -> Result<&'a crate::fonts::FontStore> {
    caller
        .fonts
        .ok_or(crate::fonts::FontError::Unavailable.into())
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
                entries: font_store(caller)?.list(query.as_ref().map(captions::NfcText::as_str))?,
            }),
            Command::FontsLoad { family } => Ok(ControlResult::Font {
                font: font_store(caller)?.load(family.as_str())?,
            }),
            Command::FontsDownload {
                family,
                accept_license,
            } => {
                let (job, _) =
                    font_store(caller)?.submit_download(runner, family.as_str(), accept_license)?;
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
                Ok(ControlResult::Job { snapshot })
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
                range,
                path,
                overwrite,
            } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                let mut settings = render::ExportSettings::from_project(self.project());
                settings.overwrite = if overwrite {
                    project::atomic::Overwrite::Replace
                } else {
                    project::atomic::Overwrite::Refuse
                };
                let (job, _) = crate::submit_export(
                    runner,
                    crate::control_preview::control_key(fingerprint)?,
                    self.project().clone(),
                    range,
                    settings,
                    path.clone(),
                )?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: vec![path],
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
