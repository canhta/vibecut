//! Background lifecycle work: the jobs a window submits, the worker that runs them and the polling that applies their results.
use super::window::EditorWindow;
use crate::{
    Editor, Error, Result,
    i18n::{self, Text},
    preferences::Preferences,
    services::{
        lifecycle::{EditorSession, fresh},
        playback::{Playback, Proxied, Sources},
    },
};
use engine::{
    ByteBudget, Command, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    control::{Caller, Error as EngineError, Policy},
    jobs::{
        JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
        SystemClock,
    },
    recovery::{self, Autosave, Candidate, Discovery, Schedule},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

pub(super) type EditorSlot = Arc<Mutex<Option<Box<Editor>>>>;
pub(super) struct WorkingEditor {
    pub(super) slot: EditorSlot,
    pub(super) editor: Option<Box<Editor>>,
}
impl Drop for WorkingEditor {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = self.editor.take();
        }
    }
}
pub(super) enum Work {
    Discover,
    Open(PathBuf),
    New,
    Short(Box<Session>, Box<render::RenderSettings>),
    Recover(Box<Candidate>),
    Discard(Box<Candidate>),
    Persist {
        editor: EditorSlot,
        path: Option<PathBuf>,
        leave: bool,
        close: bool,
        journal_only: bool,
    },
}
pub(super) enum Completed {
    Discovery(Discovery),
    Opened(Box<Editor>),
    Discarded,
    Persisted {
        result: Result<()>,
        leave: bool,
        close: bool,
    },
}
pub(super) struct Pending {
    pub(super) id: JobId,
    pub(super) receive: mpsc::Receiver<Result<Completed>>,
    pub(super) text: Text,
    pub(super) cancel: engine::jobs::CancelToken,
    pub(super) editor: Option<EditorSlot>,
}

impl EditorWindow {
    pub(super) fn submit(&mut self, work: Work, text: Text) -> Result<()> {
        if self.editor.is_some() && matches!(&work, Work::Recover(_) | Work::Open(_) | Work::New) {
            return Err(Error::Invalid("leave active project before replacing it"));
        }
        self.retry_discovery =
            matches!(&work, Work::Discover | Work::Discard(_) | Work::Recover(_));
        let editor = match &work {
            Work::Persist { editor, .. } => Some(editor.clone()),
            _ => None,
        };
        let (send, receive) = mpsc::channel();
        let root = self.root.clone();
        let budget = if matches!(
            &work,
            Work::Open(_) | Work::New | Work::Recover(_) | Work::Short(..)
        ) {
            self.budget.child(512 << 20)?
        } else {
            self.budget.clone()
        };
        let persistence = self.persistence.child(64 << 20)?;
        let corrections = self.corrections.clone();
        let input = Arc::new(Mutex::new(Some(work)));
        let preferences = self.preferences.clone();
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "desktop",
            model: "project-lifecycle",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let worker_input = input.clone();
        let submitted = self.runner.borrow_mut().submit::<engine::control::Error>(
            JobSpec {
                kind: JobKind::Persist,
                class: ResourceClass::Cpu,
                input: key,
                bytes: JobBytes {
                    compositor: 48 << 20,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                let work = worker_input
                    .lock()
                    .map_err(|_| engine::RuntimeError::Poisoned)?
                    .take()
                    .ok_or(EngineError::Edit(engine::EditError::Invalid(
                        "missing lifecycle work",
                    )))?;
                let result = run(
                    work,
                    &root,
                    budget.clone(),
                    persistence.clone(),
                    corrections.clone(),
                    preferences.clone(),
                );
                send.send(result)
                    .map_err(|_| EngineError::Runtime(engine::RuntimeError::Cancelled))?;
                Ok(Vec::new())
            },
        );
        let (id, cancel) = match submitted {
            Ok(job) => job,
            Err(e) => {
                if let Some(editor) = &editor {
                    self.editor = editor.lock().map_err(|_| Error::Poisoned)?.take();
                }
                return Err(e.into());
            }
        };
        self.pending = Some(Pending {
            id,
            receive,
            text,
            cancel,
            editor,
        });
        self.error = None;
        Ok(())
    }
    pub(super) fn poll_pending(&mut self) {
        if let Err(e) = self.runner.borrow_mut().tick() {
            self.error = Some(e.into());
            return;
        }
        // Completion snapshots remain until their owning window consumes them.
        let Some(pending) = &self.pending else {
            return;
        };
        let Some(snapshot) = self.runner.borrow().snapshot(pending.id) else {
            return;
        };
        if !matches!(
            snapshot.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let save_as_from = self.save_as_from.take();
        let _ = self.runner.borrow_mut().take_output(pending.id);
        if let Some(editor) = &pending.editor {
            match editor.lock() {
                Ok(mut editor) => self.editor = editor.take(),
                Err(_) => {
                    self.error = Some(Error::Poisoned);
                    return;
                }
            }
        }
        let result =
            pending.receive.try_recv().unwrap_or_else(|_| {
                Err(snapshot.error.map(Error::Worker).unwrap_or(Error::Engine(
                    EngineError::Runtime(engine::RuntimeError::WorkerPanicked),
                )))
            });
        // A Save As whose publication already completed is a successful Save As
        // even when the cancel arrives afterwards: the engine has rekeyed the
        // editor to the copy, so dropping the completion would leave the window
        // on the new id without carrying its lanes. A cancel before publication
        // produced no completed Persist result and still changes nothing.
        let published_save_as = save_as_from.is_some()
            && matches!(&result, Ok(Completed::Persisted { result: Ok(()), .. }));
        if pending.cancel.cancelled() && !published_save_as {
            self.startup_path = None;
            self.retry_path = None;
            return;
        }
        match result {
            Ok(Completed::Discovery(discovery)) => {
                self.candidates = discovery.candidates;
                self.diagnostics = discovery.diagnostics;
            }
            Ok(Completed::Opened(mut editor)) => {
                editor.playback.set_device_available(self.device_available);
                self.editor = Some(editor);
                self.retry_path = None;
                self.startup_path = None;
            }
            Ok(Completed::Discarded) => {}
            Ok(Completed::Persisted {
                result,
                leave,
                close,
            }) => match result {
                Ok(()) => {
                    if let Some(from) = save_as_from
                        && let Some(editor) = &self.editor
                    {
                        let to = editor.session.project().data().id;
                        if from != to {
                            self.saved_as = Some((from, to));
                        }
                    }
                    if let Some(editor) = &mut self.editor {
                        if save_as_from.is_some() {
                            editor.playback.set_device_available(self.device_available);
                        }
                        if let EditorSession::Persistent(autosave) = &editor.session {
                            self.path = autosave.saved_path().map(ToOwned::to_owned);
                        }
                    }
                    self.retry_save = None;
                    if leave || close {
                        self.editor = None;
                    }
                    if close {
                        self.ready_to_close = true;
                    }
                    if leave || close {
                        self.close_prompt = false;
                    }
                }
                Err(e) => self.error = Some(e),
            },
            Err(e) => self.error = Some(e),
        }
    }
}
/// Preview reads proxies when the project's choice is on; export never does.
fn preview_sources() -> Arc<dyn Sources> {
    Arc::new(Proxied(engine::proxies::ProxyIndex::default()))
}

fn session(
    root: &std::path::Path,
    session: Session,
    budget: ByteBudget,
    corrections: engine::footage::Lookup,
) -> Result<Autosave> {
    Ok(Autosave::new(
        root,
        session,
        budget,
        corrections,
        Arc::new(SystemClock::default()),
        Schedule::default(),
    )?)
}
fn run(
    work: Work,
    root: &std::path::Path,
    budget: ByteBudget,
    persistence: ByteBudget,
    corrections: engine::footage::Lookup,
    preferences: Preferences,
) -> Result<Completed> {
    let autosave = match work {
        Work::Discover => {
            std::fs::create_dir_all(root)?;
            return Ok(Completed::Discovery(recovery::discover(root)?));
        }
        Work::Discard(candidate) => {
            candidate.discard()?;
            return Ok(Completed::Discarded);
        }
        Work::Recover(candidate) => {
            let mut autosave = session(
                root,
                candidate.recover(100)?,
                persistence.clone(),
                corrections.clone(),
            )?;
            // Publish the recovered copy before retiring its source journal.
            autosave.flush()?;
            candidate.discard()?;
            autosave
        }
        Work::New => session(
            root,
            fresh(
                i18n::text(preferences.locale, Text::NewProject),
                preferences.default_fps,
            )?,
            persistence.clone(),
            corrections.clone(),
        )?,
        Work::Short(short, settings) => {
            let autosave = session(root, *short, persistence.clone(), corrections.clone())?;
            let mut playback = Playback::new(autosave.session(), budget, preview_sources())?;
            playback.set_render_settings(*settings)?;
            return Ok(Completed::Opened(Box::new(Editor::persisted(
                autosave,
                playback,
                preferences,
            ))));
        }
        Work::Open(path) => match Autosave::open(
            root,
            &path,
            100,
            persistence.clone(),
            corrections.clone(),
            Arc::new(SystemClock::default()),
            Schedule::default(),
        ) {
            Ok(open) => open,
            Err(recovery::Error::Edit(engine::EditError::Project(project::Error::Io(e))))
                if e.kind() == std::io::ErrorKind::NotFound
                    && path.parent().is_some_and(|parent| parent.is_dir()) =>
            {
                session(
                    root,
                    fresh(
                        i18n::text(preferences.locale, Text::UntitledProject),
                        preferences.default_fps,
                    )?,
                    persistence.clone(),
                    corrections.clone(),
                )?
            }
            Err(e) => return Err(e.into()),
        },
        Work::Persist {
            editor,
            path,
            leave,
            close,
            journal_only,
        } => {
            let owned = editor
                .lock()
                .map_err(|_| Error::Poisoned)?
                .take()
                .ok_or(Error::Invalid("missing editor"))?;
            let mut active = WorkingEditor {
                slot: editor,
                editor: Some(owned),
            };
            let result = (|| -> Result<()> {
                let editor = active
                    .editor
                    .as_mut()
                    .ok_or(Error::Invalid("missing editor"))?;
                let autosave = editor
                    .session
                    .autosave()
                    .ok_or(Error::Unsupported("nonpersistent editor"))?;
                autosave.flush()?;
                if !journal_only {
                    if let Some(path) = &path {
                        // Save As is `project.save_as`: publish a copy, then continue
                        // on that copy and close this session cleanly.
                        // The copy's preview shares the ring bytes this one holds, so
                        // memory pressure from other windows cannot refuse the reopen.
                        let opened = save_as_copy(
                            autosave,
                            path,
                            root,
                            persistence.clone(),
                            corrections.clone(),
                        )?;
                        let playback = editor
                            .playback
                            .successor(opened.session(), preview_sources())?;
                        let reopened = open_editor(opened, playback, preferences.clone(), root)?;
                        if let Some(old) = active.editor.replace(reopened) {
                            retire(old.session);
                        }
                    } else {
                        autosave.apply(Command::SaveProject {})?;
                    }
                    let editor = active
                        .editor
                        .as_mut()
                        .ok_or(Error::Invalid("missing editor"))?;
                    let autosave = editor
                        .session
                        .autosave()
                        .ok_or(Error::Unsupported("nonpersistent editor"))?;
                    autosave.flush()?;
                    if autosave.saving() || autosave.is_dirty() {
                        return Err(EngineError::Session(engine::SessionError::StaleResult).into());
                    }
                }
                if leave || close {
                    let editor = active
                        .editor
                        .as_mut()
                        .ok_or(Error::Invalid("missing editor"))?;
                    let autosave = editor
                        .session
                        .autosave()
                        .ok_or(Error::Unsupported("nonpersistent editor"))?;
                    autosave.apply(Command::ThumbnailProject {})?;
                }
                Ok(())
            })();
            if result.is_ok() && (leave || close) {
                drop(active.editor.take());
            }
            return Ok(Completed::Persisted {
                result,
                leave,
                close,
            });
        }
    };
    let playback = Playback::new(autosave.session(), budget, preview_sources())?;
    Ok(Completed::Opened(open_editor(
        autosave,
        playback,
        preferences,
        root,
    )?))
}

/// An editor over an opened or recovered session, restored to its saved playhead.
fn open_editor(
    autosave: Autosave,
    mut playback: Playback,
    preferences: Preferences,
    root: &Path,
) -> Result<Box<Editor>> {
    let view = autosave.session().metadata().view;
    playback.seek(
        view.playhead.min(autosave.session().project().duration()),
        false,
        autosave.session().project().clone(),
    )?;
    let mut editor = Editor::persisted(autosave, playback, preferences);
    editor.set_thumbnail_root(root.parent().unwrap_or(root).join("thumbnails"));
    editor.use_native_file_clipboard();
    Ok(Box::new(editor))
}

/// The desktop owns the destination the native picker returned.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// Save As: run `project.save_as` on the session, then open the copy it wrote.
/// The native picker already confirmed the destination, so `replace` is set; the
/// session's own files are protected and still refused. History and playback
/// restart on the copy; the original project file is untouched.
fn save_as_copy(
    autosave: &mut Autosave,
    path: &Path,
    root: &Path,
    persistence: ByteBudget,
    corrections: engine::footage::Lookup,
) -> Result<Autosave> {
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_secs(1),
            progress_interval: Duration::from_millis(20),
        },
        persistence.clone(),
        Arc::new(SystemClock::default()),
    )?;
    let protected = autosave.protected_export_paths()?;
    autosave.session_mut().protect_paths(protected)?;
    let published = autosave.session_mut().control_command(
        Command::SaveProjectAs {
            path: path.to_owned(),
            replace: true,
        },
        Caller {
            principal: "vibecut-desktop",
            external: false,
            policy: &Trusted,
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
        &mut runner,
    );
    runner.shutdown();
    published?;
    Ok(Autosave::open(
        root,
        path,
        100,
        persistence,
        corrections,
        Arc::new(SystemClock::default()),
        Schedule::default(),
    )?)
}

/// Close the session a Save As replaced and drop its recovery journal. A failed
/// cleanup leaves a recovery candidate for the next start, never a lost save.
fn retire(old: EditorSession) {
    if let EditorSession::Persistent(autosave) = old {
        let _ = (*autosave).retire();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use engine::jobs::{JobRunner, RunnerConfig};
    use std::time::{Duration, Instant};
    #[test]
    fn unwind_returns_the_active_editor_before_reporting_worker_failure() {
        for mode in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let budget = ByteBudget::new(128 << 20).unwrap();
            let runner = JobRunner::new(
                RunnerConfig {
                    concurrency: [1; 3],
                    queue: 2,
                    retained: 2,
                    network_attempts: 1,
                    backoff: Duration::from_millis(1),
                    progress_interval: Duration::from_millis(10),
                },
                budget.clone(),
                Arc::new(SystemClock::default()),
            )
            .unwrap();
            let runner = std::rc::Rc::new(std::cell::RefCell::new(runner));
            let mut window = EditorWindow::new(
                root.path().to_owned(),
                Preferences::default(),
                budget.clone(),
                budget.clone(),
                engine::footage::uncorrected(),
                runner,
            );
            if mode == 2 {
                window
                    .submit(
                        Work::Persist {
                            editor: Arc::new(Mutex::new(None)),
                            path: None,
                            leave: false,
                            close: false,
                            journal_only: false,
                        },
                        Text::Discovering,
                    )
                    .unwrap();
            } else {
                let session = Session::new(
                    project::Project::empty(project::ProjectId(1), "retained".into()).unwrap(),
                    10,
                )
                .unwrap();
                let playback = Playback::new(
                    &session,
                    ByteBudget::new(64 << 20).unwrap(),
                    Arc::new(crate::services::playback::Files),
                )
                .unwrap();
                let slot = Arc::new(Mutex::new(Some(Box::new(Editor::new(
                    session,
                    playback,
                    Preferences::default(),
                )))));
                let worker_slot = slot.clone();
                let (send, receive) = mpsc::channel::<Result<Completed>>();
                drop(send);
                let key = CacheKey::new(KeyInputs {
                    kind: ArtifactKind::Preview,
                    source: [0; 32],
                    provider: "fixture",
                    model: "window",
                    version: "1",
                    settings: &[],
                    transcript: [0; 32],
                    cues: [0; 32],
                    clip_range: [0; 32],
                    translation: [0; 32],
                })
                .unwrap();
                let (id, cancel) = window
                    .runner
                    .borrow_mut()
                    .submit::<engine::control::Error>(
                        JobSpec {
                            kind: JobKind::Persist,
                            class: ResourceClass::Cpu,
                            input: key,
                            bytes: JobBytes {
                                compositor: 64,
                                result: 16,
                                ..Default::default()
                            },
                        },
                        move |_| {
                            let editor = worker_slot.lock().unwrap().take();
                            let _active = WorkingEditor {
                                slot: worker_slot.clone(),
                                editor,
                            };
                            if mode == 0 {
                                panic!("injected while the worker owns the editor");
                            }
                            Ok(Vec::new())
                        },
                    )
                    .unwrap();
                window.pending = Some(Pending {
                    id,
                    receive,
                    text: Text::Discovering,
                    cancel,
                    editor: Some(slot),
                });
            }
            let id = window.pending.as_ref().unwrap().id;
            let deadline = Instant::now() + Duration::from_secs(10);
            while window.pending.is_some() {
                window.poll_pending();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let snapshot = window.runner.borrow().snapshot(id).unwrap();
            match window.error.as_ref().unwrap() {
                Error::Worker(cause) if mode == 0 => {
                    assert!(Arc::ptr_eq(cause, snapshot.error.as_ref().unwrap()));
                    assert!(matches!(
                        (cause.as_ref() as &(dyn std::error::Error + 'static))
                            .downcast_ref::<engine::RuntimeError>(),
                        Some(engine::RuntimeError::WorkerPanicked)
                    ));
                    assert_eq!(snapshot.state, JobState::Failed);
                }
                Error::Engine(EngineError::Runtime(engine::RuntimeError::WorkerPanicked))
                    if mode == 1 =>
                {
                    assert_eq!(snapshot.state, JobState::Completed);
                    assert!(snapshot.error.is_none());
                }
                Error::Invalid("missing editor") if mode == 2 => {
                    assert_eq!(snapshot.state, JobState::Completed);
                    assert!(window.editor.is_none());
                }
                error => panic!("lost the original completion diagnostic: {error:?}"),
            }
            if mode != 2 {
                assert_eq!(
                    window
                        .editor
                        .as_ref()
                        .unwrap()
                        .session
                        .project()
                        .data()
                        .name
                        .as_str(),
                    "retained"
                );
            }
            assert!(window.runner.borrow_mut().take_output(id).is_none());
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
    }
}
