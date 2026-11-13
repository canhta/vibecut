//! Chapters page of the Captions tab: the generated chapter list, the title and description the
//! creator edits, and the sidecar pair written to a chosen folder. Generation is the
//! `chapters.generate` job and export is `chapters.export_sidecars`; both are registry commands.
//! The upload sheet is the panel's own `TaskEvents`, shared with every other text upload.
use super::Subtitles;
use super::view::{Controls, time_text};
use crate::{
    egui,
    i18n::{self, Locale, Text},
    kit::Icon,
    preferences::SubtitleOptions as Options,
    services::{
        ai_access::{self, Consent, TextOpener, TextTask},
        ai_task::TaskEvents,
    },
};
use ai::text::chapters::{Chapter, ChapterDocument, MAX_DESCRIPTION_BYTES, MAX_TITLE_BYTES};
use captions::{LanguageTag, NfcText};
use engine::{
    AnalysisError, ByteBudget, Session, chapters::ChapterRequest, control::ControlResult,
    transcript::Cancellation,
};
use project::ItemParams;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

/// Chapters requested per generation. The page offers no other count.
const CHAPTER_LIMIT: usize = 10;

/// The page's state. Its title and description are drafts: export writes what they hold, and
/// nothing here edits the project.
#[derive(Default)]
pub(super) struct Panel {
    /// The chapter generation whose consent sheet may be open.
    pub(super) run: Option<Run>,
    /// The generated document. Its language, duration and chapters stay; title and description
    /// are replaced by the drafts below.
    source: Option<ChapterDocument>,
    pub(super) title: String,
    pub(super) description: String,
    problem: Option<Text>,
    saved: Option<usize>,
    /// Export was pressed: the next frame hands the window a folder-picker request.
    pub(super) folder_requested: bool,
    /// The folder the window's picker chose; the next tick writes the pair into it.
    pub(super) export_to: Option<PathBuf>,
}

/// One chapter generation. It runs on its own thread because `ChapterRequest::submit`
/// requests the human consent synchronously; the sheet is the panel's `TaskEvents`.
pub(super) struct Run {
    pub(super) cancel: Cancellation,
    pub(super) events: Arc<TaskEvents>,
    receive: mpsc::Receiver<Result<ChapterDocument, Outcome>>,
}

/// Why a generation loaded no chapters.
#[derive(Clone, Copy)]
enum Outcome {
    Declined,
    Cancelled,
    Failed,
}

/// Everything the generation thread owns.
struct Input {
    request: ChapterRequest,
    budget: ByteBudget,
    opener: TextOpener,
    provider: String,
    model: String,
    consent: Consent,
    cancel: Cancellation,
}

fn outcome(error: AnalysisError) -> Outcome {
    match error {
        AnalysisError::Providers(engine::providers::Error::ConsentRequired) => Outcome::Declined,
        AnalysisError::Runtime(engine::RuntimeError::Cancelled) => Outcome::Cancelled,
        _ => Outcome::Failed,
    }
}

/// Runs one generation to completion on the calling thread (the worker, never the window).
fn generate(input: Input) -> Result<ChapterDocument, Outcome> {
    let access = (input.opener)(
        TextTask::Chapters,
        &input.provider,
        &input.model,
        input.consent,
    )
    .map_err(|error| outcome(AnalysisError::from(error)))?;
    let mut runner = super::job_runner(&input.budget).map_err(|_| Outcome::Failed)?;
    let job = input
        .request
        .submit(&mut runner, &access)
        .map_err(outcome)?;
    loop {
        if input.cancel.cancelled() {
            let _ = runner.cancel_job(job.id);
        }
        runner.tick().map_err(|_| Outcome::Failed)?;
        match job.finish(&mut runner) {
            Ok(Some(document)) => return Ok(document),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(outcome(error)),
        }
    }
}

impl Panel {
    fn load(&mut self, document: ChapterDocument) {
        self.title = document.title().as_str().to_owned();
        self.description = document.description().as_str().to_owned();
        self.source = Some(document);
        self.problem = None;
        self.saved = None;
    }
}

/// The language of the scene's caption cues. `None` when the scene has no caption text.
fn transcript_language(session: &Session) -> Option<LanguageTag> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .filter_map(|item| match &item.content {
            ItemParams::Caption(params) => Some(params),
            _ => None,
        })
        .flat_map(|params| params.cues.cues.iter())
        .next()
        .map(|cue| cue.source_lang.clone())
}

impl Subtitles {
    /// The chapters page: generation, the drafts and the chapter list, and export.
    pub(super) fn chapters_page(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        c: &mut Controls,
        locale: Locale,
        options: &Options,
    ) {
        let busy = self.busy();
        let awaiting = self.chapters.run.as_ref().is_some_and(|run| {
            run.events
                .state
                .lock()
                .is_ok_and(|state| state.upload.is_some())
        });
        let rate = session.project().data().canvas.fps;
        let chapters: Option<Vec<Chapter>> = self
            .chapters
            .source
            .as_ref()
            .map(|document| document.chapters().to_vec());
        let mut generate = false;
        let mut cancel = false;
        let mut export = false;
        ui.add_enabled_ui(!busy, |ui| {
            generate = c.button(
                ui,
                "subtitles.chapters.generate",
                Text::ChaptersGenerate,
                Icon::Sparkle,
                true,
            );
        });
        if self.chapters.run.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                let waiting = if awaiting {
                    Text::ChaptersAwaitingApproval
                } else {
                    Text::ChaptersWorking
                };
                ui.label(c.tr(waiting));
                cancel = c.button(ui, "subtitles.chapters.cancel", Text::Cancel, Icon::X, true);
            });
        }
        if let Some(problem) = self.chapters.problem {
            ui.label(c.tr(problem));
        }
        if let Some(count) = self.chapters.saved {
            ui.label(i18n::format(
                locale,
                Text::ChaptersSaved,
                &[("count", &i18n::number(locale, count as u64))],
            ));
        }
        if let Some(list) = &chapters {
            ui.add_enabled_ui(!busy, |ui| {
                let label = ui.label(c.tr(Text::ChaptersTitleField));
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.chapters.title)
                            .id_salt("subtitles.chapters.title")
                            .char_limit(MAX_TITLE_BYTES / 4)
                            .desired_width(ui.available_width()),
                    )
                    .labelled_by(label.id);
                c.remember("subtitles.chapters.title", &r);
                let label = ui.label(c.tr(Text::ChaptersDescriptionField));
                let r = ui
                    .add(
                        egui::TextEdit::multiline(&mut self.chapters.description)
                            .id_salt("subtitles.chapters.description")
                            .desired_rows(3)
                            .char_limit(MAX_DESCRIPTION_BYTES / 4)
                            .desired_width(ui.available_width()),
                    )
                    .labelled_by(label.id);
                c.remember("subtitles.chapters.description", &r);
                for chapter in list {
                    ui.label(format!(
                        "{} · {}",
                        time_text(chapter.start, rate, locale),
                        chapter.title.as_str()
                    ));
                }
                export = c.button(
                    ui,
                    "subtitles.chapters.export",
                    Text::ChaptersExport,
                    Icon::Export,
                    !self.chapters.title.trim().is_empty(),
                );
            });
        }
        if cancel {
            self.cancel();
        }
        if generate {
            self.generate_chapters(session, options);
        }
        if export {
            self.chapters.folder_requested = true;
        }
    }

    /// Start `chapters.generate` with the configured text provider. The job's own consent
    /// sheet gates every upload; nothing is sent before approval.
    fn generate_chapters(&mut self, session: &mut Session, options: &Options) {
        if self.busy() {
            return;
        }
        self.chapters.problem = None;
        self.chapters.saved = None;
        let Some(language) = transcript_language(session) else {
            self.chapters.problem = Some(Text::ChaptersNoTranscript);
            return;
        };
        let Ok(request) = ChapterRequest::new(session.project(), language, CHAPTER_LIMIT) else {
            self.chapters.problem = Some(Text::ChaptersUnsupported);
            return;
        };
        if self.flush_drafts(session).is_err() {
            self.chapters.problem = Some(Text::ChaptersAiFailed);
            return;
        }
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let consent: Consent = events.clone();
        let input = Input {
            request,
            budget: self.budget.clone(),
            opener: self.text.clone(),
            provider: options.text_provider.clone(),
            model: options.chapters_model.clone(),
            consent,
            cancel: cancel.clone(),
        };
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = send.send(generate(input));
        });
        self.chapters.run = Some(Run {
            cancel,
            events,
            receive,
        });
    }

    /// The folder the window's picker chose for the export, and the engine path every export
    /// takes. The next tick writes the pair; an existing pair is refused by the engine.
    pub fn export_chapters_to(&mut self, dir: PathBuf) {
        self.chapters.export_to = Some(dir);
    }

    /// Apply a finished chapter job and a chosen folder. Called on every tick, where the session
    /// is mutable.
    pub(super) fn tick_chapters(&mut self, session: &mut Session) {
        self.finish_chapter_run();
        if let Some(dir) = self.chapters.export_to.take() {
            self.export_chapters(session, dir);
        }
    }

    /// Apply a finished generation. Called on every tick, so the window never waits for it.
    fn finish_chapter_run(&mut self) {
        let Some(run) = &self.chapters.run else {
            return;
        };
        let result = match run.receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(Outcome::Failed),
        };
        self.chapters.run = None;
        match result {
            Ok(document) => self.chapters.load(document),
            Err(Outcome::Cancelled) => {}
            // A declined upload sent nothing; any other failure is the provider's, shown in one
            // line and never retried.
            Err(Outcome::Declined) => self.chapters.problem = Some(Text::ChaptersDeclined),
            Err(Outcome::Failed) => self.chapters.problem = Some(Text::ChaptersAiFailed),
        }
    }

    /// Write the sidecar pair with the edited title and description through
    /// `chapters.export_sidecars`. An existing pair is refused by the engine and left alone.
    fn export_chapters(&mut self, session: &mut Session, dir: PathBuf) {
        self.chapters.problem = None;
        self.chapters.saved = None;
        let Some(source) = self.chapters.source.clone() else {
            return;
        };
        if self.chapters.title.trim().is_empty() {
            self.chapters.problem = Some(Text::ChaptersBlankTitle);
            return;
        }
        let document = match ChapterDocument::new(
            source.language().clone(),
            source.duration(),
            NfcText::from(self.chapters.title.clone()),
            NfcText::from(self.chapters.description.clone()),
            source.chapters().to_vec(),
            Vec::new(),
        ) {
            Ok(document) => document,
            Err(_) => {
                self.chapters.problem = Some(Text::ChaptersExportFailed);
                return;
            }
        };
        let result = ai_access::run(
            session,
            &mut self.runner,
            None,
            None,
            "chapters.export_sidecars",
            serde_json::json!({"dir": dir, "document": document}),
        );
        match result {
            Ok(ControlResult::ChapterSidecars { .. }) => self.chapters.saved = Some(2),
            Err(engine::control::Error::Artifacts(engine::ArtifactError::Edit(
                engine::EditError::Invalid("chapter sidecar destination exists"),
            ))) => self.chapters.problem = Some(Text::ChaptersExists),
            Ok(_) | Err(_) => self.chapters.problem = Some(Text::ChaptersExportFailed),
        }
    }
}
