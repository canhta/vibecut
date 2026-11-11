//! Applies captured LUT import results to the selected clip.
use super::super::{State, read};
use crate::editor::{Desk, Error, ItemId, Result};
use engine::{
    Session,
    control::{ControlResult, JobOutput, MediaAccess, Revision},
    jobs::{JobId, JobRunner, JobState},
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(in crate::editor) struct LutWorkflow {
    access: Option<Arc<Mutex<MediaAccess>>>,
    budget: Option<engine::ByteBudget>,
    runner: Option<JobRunner>,
    sequence: u64,
    capture: Option<Capture>,
    pub(in crate::editor) unavailable: Option<project::LutId>,
}
struct Capture {
    snapshot: Arc<project::Project>,
    session: String,
    revision: Revision,
    item: ItemId,
    expected: Option<project::LutId>,
    job: Option<JobId>,
}
struct Human;
impl engine::control::Policy for Human {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}
impl LutWorkflow {
    pub(in crate::editor) fn attach(
        &mut self,
        access: Arc<Mutex<MediaAccess>>,
        budget: engine::ByteBudget,
    ) {
        self.access = Some(access);
        self.budget = Some(budget);
    }
    pub(in crate::editor) fn busy(&self) -> bool {
        self.capture.is_some()
    }
    fn request(
        &mut self,
        session: &mut Session,
        command: &str,
        params: serde_json::Value,
    ) -> Result<ControlResult> {
        if self.runner.is_none() {
            self.runner = Some(JobRunner::new(
                engine::jobs::RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 1,
                    retained: 4,
                    network_attempts: 1,
                    backoff: std::time::Duration::from_millis(100),
                    progress_interval: std::time::Duration::from_millis(40),
                },
                self.budget
                    .clone()
                    .ok_or(Error::Invalid("LUT budget unavailable"))?,
                Arc::new(engine::jobs::SystemClock::default()),
            )?);
        }
        self.sequence = self.sequence.wrapping_add(1);
        let key = format!("desktop-lut-{}", self.sequence);
        let request = engine::control::Request {
            api_version: engine::control::API_VERSION,
            registry_hash: Some(engine::control::registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: key.clone(),
            idempotency_key: Some(key),
            base_rev: Some(session.revision()),
            command: command.into(),
            params,
        };
        let access = self
            .access
            .as_ref()
            .ok_or(Error::Invalid("LUT source store unavailable"))?;
        Ok(session.control_typed(
            request,
            engine::control::Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Human,
                fonts: None,
                sounds: None,
                media: Some(access.as_ref()),
                chapters: None,
                captions: None,
            },
            self.runner
                .as_mut()
                .ok_or(Error::Invalid("LUT runner unavailable"))?,
        )?)
    }
    fn cancel(&mut self) {
        if let Some(capture) = self.capture.take()
            && let Some(job) = capture.job
            && let Some(runner) = &mut self.runner
        {
            let _ = runner.cancel_job(job);
        }
    }
}
impl State {
    pub(in crate::editor) fn abort_lut(&mut self) {
        self.lut.cancel();
    }
    pub(in crate::editor) fn attach_luts(
        &mut self,
        access: Arc<Mutex<MediaAccess>>,
        budget: engine::ByteBudget,
    ) {
        self.lut.attach(access, budget);
    }
    pub(in crate::editor) fn set_lut_failure(&mut self, error: Option<Arc<Error>>) {
        self.lut.unavailable = error.as_deref().and_then(|e| match e {
            Error::Render(render::Error::LutUnavailable { id, .. }) => Some(*id),
            _ => None,
        });
    }
    pub(in crate::editor) fn request_lut(
        &mut self,
        desk: &mut Desk<'_>,
        item: ItemId,
        expected: Option<project::LutId>,
    ) -> Result<()> {
        if read::item_locked(desk.view().session.project(), item)
            || desk.view().session.transaction_active()
        {
            return Err(Error::Invalid("LUT target unavailable"));
        }
        self.lut.cancel();
        self.lut.capture = Some(Capture {
            snapshot: desk.view().session.project().clone(),
            session: desk.view().session.session_id().into(),
            revision: desk.view().session.revision(),
            item,
            expected,
            job: None,
        });
        desk.request(crate::editor::Request::ImportLut);
        Ok(())
    }
    pub(in crate::editor) fn picked_lut(
        &mut self,
        desk: &mut Desk<'_>,
        path: Option<std::path::PathBuf>,
    ) -> Result<()> {
        let Some(path) = path else {
            self.lut.cancel();
            return Ok(());
        };
        if !self.valid_lut_capture(desk) {
            self.lut.cancel();
            return Err(Error::Invalid("stale LUT picker"));
        }
        let expected = self.lut.capture.as_ref().and_then(|c| c.expected);
        let result=desk.with_session(|session|self.lut.request(session,"color.import_lut",serde_json::json!({"path":path,"name":null,"licence":null,"expected_hash":expected})))?;
        let ControlResult::JobAccepted { job, .. } = result else {
            return Err(Error::Invalid("LUT import result"));
        };
        if let Some(capture) = &mut self.lut.capture {
            capture.job = Some(job);
        }
        Ok(())
    }
    fn valid_lut_capture(&self, desk: &Desk<'_>) -> bool {
        self.lut.capture.as_ref().is_some_and(|capture| {
            let view = desk.view();
            let kind = super::super::element_kind(view.session.project(), capture.item)
                .map(|(kind, _)| kind.id());
            capture.session == view.session.session_id()
                && capture.revision == view.session.revision()
                && Arc::ptr_eq(&capture.snapshot, view.session.project())
                && crate::editor::context::single(view.selection) == Some(capture.item)
                && !self.details
                && !view.session.transaction_active()
                && !read::item_locked(view.session.project(), capture.item)
                && kind
                    .and_then(|kind| view.preferences.properties_tabs.get(kind))
                    .is_some_and(|tab| tab == "adjust")
        })
    }
    pub(in crate::editor) fn poll_lut(&mut self, desk: &mut Desk<'_>) -> Result<()> {
        if self.lut.capture.is_none() {
            return Ok(());
        }
        if !self.valid_lut_capture(desk) {
            self.lut.cancel();
            return Ok(());
        }
        let Some(job) = self.lut.capture.as_ref().and_then(|c| c.job) else {
            return Ok(());
        };
        let runner = self
            .lut
            .runner
            .as_mut()
            .ok_or(Error::Invalid("LUT runner unavailable"))?;
        runner.tick()?;
        runner.drain_events();
        let snapshot = runner
            .snapshot(job)
            .ok_or(Error::Invalid("LUT output expired"))?;
        match snapshot.state {
            JobState::Completed => {
                let result = desk.with_session(|session| {
                    self.lut
                        .request(session, "job_status", serde_json::json!({"job":job}))
                })?;
                let ControlResult::Job { snapshot } = result else {
                    return Err(Error::Invalid("LUT job status"));
                };
                let Some(JobOutput::Lut(record)) = snapshot.output else {
                    return Err(Error::Invalid("LUT job output"));
                };
                if !self.valid_lut_capture(desk) {
                    self.lut.cancel();
                    return Ok(());
                }
                let capture = self
                    .lut
                    .capture
                    .take()
                    .ok_or(Error::Invalid("stale LUT import"))?;
                let grade = read::item(&capture.snapshot, capture.item)
                    .and_then(|item| item.effects.iter().find(|e| e.kind == "color_grade"));
                let same = grade.and_then(|effect| effect.value("lut"))
                    == Some(&project::EffectValue::Lut(Some(record.id)));
                let strength = if capture.expected.is_some() || same {
                    grade
                        .and_then(|effect| effect.value("lut_strength"))
                        .and_then(project::EffectValue::scalar)
                        .unwrap_or(100.)
                } else {
                    100.
                };
                desk.with_session(|session| {
                    Ok(session.apply_lut(capture.item, record, strength)?)
                })?;
                self.lut.unavailable = None;
                desk.sync();
            }
            JobState::Failed | JobState::Cancelled => {
                self.lut.capture = None;
                return Err(snapshot.error.map_or(Error::Cancelled, |error| {
                    Error::Engine(engine::EditError::Runtime(engine::RuntimeError::Failed(
                        error,
                    )))
                }));
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn select_lut(
        &mut self,
        desk: &mut Desk<'_>,
        item: ItemId,
        id: Option<project::LutId>,
    ) -> Result<()> {
        if let Some(id) = id {
            let current = read::item(desk.view().session.project(), item)
                .and_then(|i| i.effects.iter().find(|e| e.kind == "color_grade"));
            if current.and_then(|e| e.value("lut")) == Some(&project::EffectValue::Lut(Some(id))) {
                return Ok(());
            }
            let strength = current
                .and_then(|e| e.value("lut_strength"))
                .and_then(project::EffectValue::scalar)
                .unwrap_or(100.);
            let record = desk
                .view()
                .session
                .project()
                .data()
                .luts
                .iter()
                .find(|r| r.id == id)
                .cloned()
                .ok_or(Error::Invalid("unregistered LUT"))?;
            desk.with_session(|session| Ok(session.apply_lut(item, record, strength)?))?;
            desk.sync();
        } else if let Some(effect) = read::item(desk.view().session.project(), item)
            .and_then(|i| i.effects.iter().find(|e| e.kind == "color_grade"))
            .map(|e| e.id.clone())
        {
            desk.apply_result(
                engine::Command::UpdateEffectParam {
                    item,
                    effect,
                    key: "lut".into(),
                    value: project::EffectParamUpdate::LutRef(project::LutReference { lut: None }),
                },
                false,
            )?;
        }
        self.lut.unavailable = None;
        Ok(())
    }
}
