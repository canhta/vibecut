//! Chapters page of the Captions tab: the generated chapter list, the title and description the
//! creator edits, and the sidecar pair written to a chosen folder. Generation is the
//! `chapters.generate` job and export is `chapters.export_sidecars`; both are registry commands.
//! The upload sheet is the panel's own `TaskEvents`, shared with every other text upload.
use super::view::{Controls, time_text};
use super::{Repaint, Subtitles, terminal};
use crate::{
    egui,
    i18n::{self, Locale, Text},
    kit::Icon,
    preferences::SubtitleOptions as Options,
    services::{
        ai_access::{self, Consent, TextTask},
        ai_task::TaskEvents,
    },
};
use ai::text::chapters::{Chapter, ChapterDocument, MAX_DESCRIPTION_BYTES, MAX_TITLE_BYTES};
use captions::{LanguageTag, NfcText};
use engine::{
    Session,
    chapters::ChapterRequest,
    control::{ControlResult, ErrorCode},
    jobs::{JobId, JobState},
    transcript::Cancellation,
};
use project::ItemParams;
use std::{path::PathBuf, pin::Pin, sync::Arc, task::Poll};

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
    /// The native folder dialog of the export, polled on every tick.
    folder: Option<FolderPick>,
    /// The folder the dialog chose; the next tick writes the pair into it.
    pub(super) export_to: Option<PathBuf>,
}

/// One chapter generation job and the cancellation and consent its text access shares.
pub(super) struct Run {
    pub(super) id: JobId,
    pub(super) cancel: Cancellation,
    pub(super) events: Arc<TaskEvents>,
}

/// The pending folder dialog and the waker that repaints the window when it answers.
struct FolderPick {
    future: Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>,
    waker: std::task::Waker,
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
            self.choose_folder(ui.ctx());
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
        if ChapterRequest::new(session.project(), language.clone(), CHAPTER_LIMIT).is_err() {
            self.chapters.problem = Some(Text::ChaptersUnsupported);
            return;
        }
        if self.flush_drafts(session).is_err() {
            self.chapters.problem = Some(Text::ChaptersAiFailed);
            return;
        }
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let consent: Consent = events.clone();
        let access = match (self.text)(
            TextTask::Chapters,
            &options.text_provider,
            &options.chapters_model,
            consent,
        ) {
            Ok(access) => access,
            Err(_) => {
                self.chapters.problem = Some(Text::ChaptersAiFailed);
                return;
            }
        };
        let accepted = ai_access::run_text(
            session,
            &mut self.runner,
            &access,
            "chapters.generate",
            serde_json::json!({"language": language.as_str(), "max_chapters": CHAPTER_LIMIT}),
        )
        .and_then(ai_access::accepted);
        drop(access);
        match accepted {
            Ok((id, _)) => {
                self.chapters.run = Some(Run { id, cancel, events });
            }
            Err(_) => self.chapters.problem = Some(Text::ChaptersAiFailed),
        }
    }

    /// Open the native folder dialog for the export; its answer arrives on a later tick.
    fn choose_folder(&mut self, ctx: &egui::Context) {
        self.chapters.folder = Some(FolderPick {
            future: Box::pin(rfd::AsyncFileDialog::new().pick_folder()),
            waker: std::task::Waker::from(Arc::new(Repaint(ctx.clone()))),
        });
    }

    /// Native folder-dialog injection: the same export the page's folder choice starts. The
    /// next tick writes the pair; an existing pair is refused by the engine.
    pub fn export_chapters_to(&mut self, dir: PathBuf) {
        self.chapters.export_to = Some(dir);
    }

    /// Apply a finished chapter job and a chosen folder. Called on every tick, where the session
    /// is mutable.
    pub(super) fn tick_chapters(&mut self, session: &mut Session) {
        self.poll_folder();
        self.finish_chapter_run();
        if let Some(dir) = self.chapters.export_to.take() {
            self.export_chapters(session, dir);
        }
    }

    fn poll_folder(&mut self) {
        let Some(pick) = self.chapters.folder.as_mut() else {
            return;
        };
        let mut context = std::task::Context::from_waker(&pick.waker);
        if let Poll::Ready(folder) = std::future::Future::poll(pick.future.as_mut(), &mut context) {
            self.chapters.folder = None;
            self.chapters.export_to = folder.map(|folder| folder.path().to_owned());
        }
    }

    fn finish_chapter_run(&mut self) {
        let Some(run) = self.chapters.run.as_ref() else {
            return;
        };
        let (id, cancelled) = (run.id, run.cancel.cancelled());
        let Some(snapshot) = self.runner.snapshot(id) else {
            self.chapters.run = None;
            self.chapters.problem = Some(Text::ChaptersAiFailed);
            return;
        };
        if !terminal(snapshot.state) {
            return;
        }
        self.chapters.run = None;
        if snapshot.state == JobState::Completed {
            let decoded = self.runner.output_bytes(id).map(engine::chapters::decode);
            self.runner.take_output(id);
            match decoded {
                Some(Ok(document)) => self.chapters.load(document),
                _ => self.chapters.problem = Some(Text::ChaptersAiFailed),
            }
            return;
        }
        self.runner.take_output(id);
        if cancelled || snapshot.state == JobState::Cancelled {
            return;
        }
        // A declined upload fails the job with the permission code; any other failure is the
        // provider's, shown in one line and never retried.
        let declined = snapshot
            .error
            .as_ref()
            .is_some_and(|error| matches!(error.code(), ErrorCode::Permission));
        self.chapters.problem = Some(if declined {
            Text::ChaptersDeclined
        } else {
            Text::ChaptersAiFailed
        });
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
            Err(engine::control::Error::Edit(engine::EditError::Invalid(
                "chapter sidecar destination exists",
            ))) => self.chapters.problem = Some(Text::ChaptersExists),
            Ok(_) | Err(_) => self.chapters.problem = Some(Text::ChaptersExportFailed),
        }
    }
}
