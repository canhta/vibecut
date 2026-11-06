//! Executes admitted control commands.
use super::{
    Command, ControlResult, EditError, JobKind, JobRunner, JobState, JobView, MAX_RESPONSE_BYTES,
    Policy, ProviderView, Result, Session, bounded_json, registry, validate_wire,
};
impl Session {
    pub(super) fn run_control(
        &mut self,
        command: Command,
        runner: &mut JobRunner,
        fingerprint: blake3::Hash,
        policy: &dyn Policy,
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
                entries: crate::subtitles::provider_options()
                    .into_iter()
                    .map(ProviderView::from)
                    .collect(),
            }),
            Command::FontsList { query } => Ok(ControlResult::Fonts {
                entries: self
                    .font_store()?
                    .list(query.as_ref().map(captions::NfcText::as_str))?,
            }),
            Command::FontsLoad { family } => Ok(ControlResult::Font {
                font: self.font_store()?.load(family.as_str())?,
            }),
            Command::FontsDownload {
                family,
                accept_license,
            } => {
                let (job, _) =
                    self.font_store()?
                        .submit_download(runner, family.as_str(), accept_license)?;
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
                    if policy.snapshot_overwrite() {
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
                    control_key(fingerprint)?,
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
                let (job, _) =
                    self.submit_export_command(runner, control_key(fingerprint)?, command)?;
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

pub(crate) fn control_key(fingerprint: blake3::Hash) -> Result<crate::cache::CacheKey> {
    crate::cache::CacheKey::new(crate::cache::KeyInputs {
        kind: crate::cache::ArtifactKind::Preview,
        source: [0; 32],
        provider: "control",
        model: "command",
        version: "1",
        settings: fingerprint.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}
