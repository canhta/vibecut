//! Applies the Audio tab's loudness controls. Scope switches and the target are
//! `audio.set_loudness` / `audio.clear_loudness` edits; Apply runs the
//! `audio.measure_loudness` job and stores its gains with `audio.apply_loudness`.
//! The panel never writes project data itself.
use crate::editor::properties::State;
use crate::editor::{Error, Port, Result};
use engine::{
    Session,
    control::{ControlResult, JobOutput, Revision},
    jobs::{JobId, JobRunner, JobState},
};
use project::{LoudnessScope, LoudnessTarget};

/// Why the last measurement ended without gains.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::editor) enum Failed {
    /// The job failed or its output could not be read.
    Measurement(String),
    /// The project changed while the job ran, so its gains would be stale.
    Stale,
}

#[derive(Default)]
pub(in crate::editor) struct LoudnessWorkflow {
    budget: Option<engine::ByteBudget>,
    runner: Option<JobRunner>,
    capture: Option<Capture>,
    /// Latest finished measurement. Shown only while the project still holds its gains.
    result: Option<engine::loudness::LoudnessResult>,
    /// Progress of the running measurement, 0..=1.
    progress: f32,
    failure: Option<Failed>,
    /// The target typed last, kept while no scope is on so a later switch uses it.
    typed: Option<LoudnessTarget>,
    /// The media a measurement reads; `None` reads the project's own files.
    sources: Option<SourceFactory>,
}

/// Builds the source provider one measurement reads. Tests replace the project's media with
/// a provider that is silent, fails, or holds the job on a gate.
pub(in crate::editor) type SourceFactory =
    std::sync::Arc<dyn Fn() -> Box<dyn render::SourceProvider> + Send + Sync>;

struct Capture {
    session: String,
    revision: Revision,
    job: JobId,
}

struct Human;
impl engine::control::Policy for Human {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// The target one scope holds, if it is switched on.
pub(in crate::editor) fn scope_target(
    project: &project::Project,
    scope: LoudnessScope,
) -> Option<LoudnessTarget> {
    let loudness = &project.data().settings.loudness;
    match scope {
        LoudnessScope::Clip => loudness.clip,
        LoudnessScope::Master => loudness.master,
    }
}

impl LoudnessWorkflow {
    pub(in crate::editor) fn attach(&mut self, budget: engine::ByteBudget) {
        self.budget = Some(budget);
    }

    /// Whether a measurement job is running.
    pub(in crate::editor) fn busy(&self) -> bool {
        self.capture.is_some()
    }

    /// The running measurement's progress, 0..=1, while one runs.
    pub(in crate::editor) fn progress(&self) -> Option<f32> {
        self.capture.as_ref().map(|_| self.progress)
    }

    pub(in crate::editor) fn failure(&self) -> Option<&Failed> {
        self.failure.as_ref()
    }

    /// The target the fields show: the switched-on scope's target, else the last typed one.
    pub(in crate::editor) fn shown_target(&self, project: &project::Project) -> LoudnessTarget {
        scope_target(project, LoudnessScope::Clip)
            .or_else(|| scope_target(project, LoudnessScope::Master))
            .or(self.typed)
            .unwrap_or(LoudnessTarget::DEFAULT)
    }

    /// The finished measurement, while the project still holds the gains it produced.
    pub(in crate::editor) fn result_for<'a>(
        &'a self,
        project: &project::Project,
    ) -> Option<&'a engine::loudness::LoudnessResult> {
        let result = self.result.as_ref()?;
        (project.data().settings.loudness.measured.as_ref() == Some(&result.gains))
            .then_some(result)
    }

    /// The shared job runner, created on first use with the attached budget.
    fn runner(&mut self) -> Result<&mut JobRunner> {
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
                    .ok_or(Error::Invalid("loudness budget unavailable"))?,
                std::sync::Arc::new(engine::jobs::SystemClock::default()),
            )?);
        }
        self.runner
            .as_mut()
            .ok_or(Error::Invalid("loudness runner unavailable"))
    }

    fn control(
        &mut self,
        session: &mut Session,
        command: engine::Command,
    ) -> Result<ControlResult> {
        let runner = self.runner()?;
        Ok(session.control_command(
            command,
            engine::control::Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Human,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            runner,
        )?)
    }

    fn cancel(&mut self) {
        if let Some(capture) = self.capture.take()
            && let Some(runner) = &mut self.runner
        {
            let _ = runner.cancel_job(capture.job);
        }
        self.progress = 0.0;
    }
}

impl State {
    pub(in crate::editor) fn attach_loudness(&mut self, budget: engine::ByteBudget) {
        self.loudness.attach(budget);
    }

    /// Replace the media the measurements read; the next Apply uses it.
    pub(in crate::editor) fn set_loudness_sources(&mut self, sources: SourceFactory) {
        self.loudness.sources = Some(sources);
    }

    /// One scope's switch. On sets the shown target on that scope; off clears it. One undo step.
    pub(in crate::editor) fn set_loudness_scope(
        &mut self,
        port: &mut Port<'_>,
        scope: LoudnessScope,
        enabled: bool,
    ) -> Result<()> {
        if self.loudness.busy() {
            return Err(Error::Invalid("loudness measurement is running"));
        }
        let project = port.view().session.project();
        if enabled == scope_target(project, scope).is_some() {
            return Ok(());
        }
        let command = if enabled {
            let target = self.loudness.shown_target(project);
            engine::Command::SetLoudness {
                scope,
                target_lufs: target.target_lufs,
                true_peak_db: target.true_peak_db,
            }
        } else {
            engine::Command::ClearLoudness { scope }
        };
        port.apply_result(command)?;
        Ok(())
    }

    /// The shared target. Every switched-on scope takes it in one batch, so one undo step.
    pub(in crate::editor) fn set_loudness_target(
        &mut self,
        port: &mut Port<'_>,
        target: LoudnessTarget,
    ) -> Result<()> {
        if !target.is_valid() {
            return Err(Error::Invalid("loudness target"));
        }
        if self.loudness.busy() {
            return Err(Error::Invalid("loudness measurement is running"));
        }
        self.loudness.typed = Some(target);
        let project = port.view().session.project();
        let commands: Vec<engine::Command> = [LoudnessScope::Clip, LoudnessScope::Master]
            .into_iter()
            .filter(|scope| scope_target(project, *scope).is_some_and(|set| set != target))
            .map(|scope| engine::Command::SetLoudness {
                scope,
                target_lufs: target.target_lufs,
                true_peak_db: target.true_peak_db,
            })
            .collect();
        if commands.is_empty() {
            return Ok(());
        }
        port.apply_result(engine::Command::Batch { commands })?;
        Ok(())
    }

    /// Apply: start the measurement of the switched-on scopes.
    pub(in crate::editor) fn start_loudness(&mut self, port: &mut Port<'_>) -> Result<()> {
        if self.loudness.busy() {
            return Ok(());
        }
        let project = port.view().session.project();
        if scope_target(project, LoudnessScope::Clip).is_none()
            && scope_target(project, LoudnessScope::Master).is_none()
        {
            return Err(Error::Invalid("loudness scope"));
        }
        if port.view().session.transaction_active() {
            return Err(Error::Invalid("finish the open edit before measuring"));
        }
        // The same engine job `audio.measure_loudness` queues, submitted with this editor's
        // source factory so tests can hold, fail or silence the measurement.
        let project = port.view().session.project().clone();
        let factory = self.loudness.sources.clone();
        let sources = move || -> Box<dyn render::SourceProvider> {
            match &factory {
                Some(make) => (**make)(),
                None => Box::new(render::FileSources::default()),
            }
        };
        let job = engine::loudness::submit_loudness(self.loudness.runner()?, project, sources)
            .map_err(|_| Error::Invalid("loudness job"))?;
        let view = port.view();
        self.loudness.capture = Some(Capture {
            session: view.session.session_id().into(),
            revision: view.session.revision(),
            job,
        });
        self.loudness.progress = 0.0;
        self.loudness.result = None;
        self.loudness.failure = None;
        Ok(())
    }

    pub(in crate::editor) fn cancel_loudness(&mut self) {
        self.loudness.cancel();
    }

    /// Advance the measurement once per frame. A finished job stores its gains through
    /// `audio.apply_loudness`; a failure or a stale project is shown by the panel.
    pub(in crate::editor) fn poll_loudness(&mut self, port: &mut Port<'_>) {
        let Some(job) = self.loudness.capture.as_ref().map(|capture| capture.job) else {
            return;
        };
        let stale = {
            let view = port.view();
            self.loudness.capture.as_ref().is_some_and(|capture| {
                capture.session != view.session.session_id()
                    || capture.revision != view.session.revision()
            })
        };
        if stale {
            self.loudness.cancel();
            self.loudness.failure = Some(Failed::Stale);
            return;
        }
        let Some(runner) = self.loudness.runner.as_mut() else {
            self.loudness.cancel();
            return;
        };
        if let Err(error) = runner.tick() {
            self.loudness.cancel();
            self.loudness.failure = Some(Failed::Measurement(error.to_string()));
            return;
        }
        runner.drain_events();
        let Some(snapshot) = runner.snapshot(job) else {
            self.loudness.cancel();
            self.loudness.failure = Some(Failed::Measurement(String::new()));
            return;
        };
        self.loudness.progress = snapshot.progress;
        match snapshot.state {
            JobState::Completed => {
                let status = port.with_session(|session| {
                    self.loudness
                        .control(session, engine::Command::JobStatus { job })
                });
                self.loudness.capture = None;
                self.loudness.progress = 0.0;
                let output = match status {
                    Ok(ControlResult::Job { snapshot }) => snapshot.output,
                    _ => None,
                };
                let Some(JobOutput::Loudness(result)) = output else {
                    self.loudness.failure =
                        Some(Failed::Measurement("loudness job output".to_owned()));
                    return;
                };
                match port.apply_result(engine::Command::ApplyLoudness {
                    gains: result.gains.clone(),
                }) {
                    Ok(_) => self.loudness.result = Some(result),
                    Err(error) => {
                        self.loudness.failure = Some(Failed::Measurement(error.to_string()));
                    }
                }
            }
            JobState::Failed => {
                let reason = snapshot
                    .error
                    .map_or_else(String::new, |error| error.to_string());
                self.loudness.capture = None;
                self.loudness.failure = Some(Failed::Measurement(reason));
            }
            JobState::Cancelled => {
                self.loudness.capture = None;
                self.loudness.progress = 0.0;
            }
            _ => {}
        }
    }
}
