//! Caption transcript pane over the engine's word and cue commands.
use super::Subtitles;
use crate::{
    egui,
    i18n::{self, Locale, Text},
    kit::{self as theme, Icon, tinted},
};
use engine::{Command, CueId, EditError, Session, TranscriptView, WordId};
use project::ItemId;
use std::{collections::BTreeSet, sync::Arc};
use time::{TICKS_PER_SECOND, Tick};

/// Cues per page; the pane pages like the cue list above it.
const PAGE: usize = 50;

/// The pane's read of the active scene and its own selection. The view is keyed by
/// the project snapshot it was read from, so undo, redo and any edit rebuild it.
#[derive(Default)]
pub(super) struct Pane {
    view: Option<(Arc<project::Project>, Arc<TranscriptView>)>,
    words: BTreeSet<WordId>,
    anchor: Option<WordId>,
    cues: BTreeSet<CueId>,
    page: usize,
    refusal: Option<Text>,
}

impl Pane {
    pub(super) fn selected(&self) -> usize {
        self.words.len()
    }

    pub(super) fn refusal(&self) -> Option<Text> {
        self.refusal
    }

    fn clear_selection(&mut self) {
        self.words.clear();
        self.cues.clear();
        self.anchor = None;
    }
}

impl Subtitles {
    /// Rebuild the view when the project snapshot changed. Selection survives only
    /// while its words or cues still exist in the project; a scene switch keeps
    /// selection from the other scene so a cut there is refused, not silent.
    fn transcript_refresh(&mut self, session: &Session) {
        let project = session.project();
        if self
            .transcript
            .view
            .as_ref()
            .is_some_and(|(seen, _)| Arc::ptr_eq(seen, project))
        {
            return;
        }
        let Ok(view) = session.transcript() else {
            self.transcript.view = None;
            return;
        };
        let data = project.data();
        let scene = view.scene;
        let elsewhere = |item: ItemId| item_elsewhere(data, item, scene);
        let words: BTreeSet<WordId> = view
            .cues
            .iter()
            .flat_map(|cue| cue.words.iter().map(|word| word.id.clone()))
            .collect();
        let cues: BTreeSet<CueId> = view
            .cues
            .iter()
            .map(|cue| CueId {
                item: cue.item,
                cue: cue.id.clone(),
            })
            .collect();
        self.transcript
            .words
            .retain(|id| words.contains(id) || elsewhere(id.item));
        self.transcript
            .cues
            .retain(|id| cues.contains(id) || elsewhere(id.item));
        if self
            .transcript
            .anchor
            .as_ref()
            .is_some_and(|anchor| !self.transcript.words.contains(anchor))
        {
            self.transcript.anchor = None;
        }
        self.transcript.view = Some((project.clone(), Arc::new(view)));
    }

    /// Words and cues of every caption item in the active scene, below the cue list.
    pub(super) fn transcript_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        locale: Locale,
        dark: bool,
        widgets: &mut Vec<(String, egui::Response)>,
    ) {
        self.transcript_refresh(session);
        let Some((_, view)) = self.transcript.view.clone() else {
            return;
        };
        let ready = self.edit_ready(session);
        let rate = session.project().data().canvas.fps;
        let timecode = |tick: Tick| {
            time::format_timecode(tick, rate).unwrap_or_else(|_| {
                i18n::decimal(locale, tick.0 as f64 / TICKS_PER_SECOND as f64, 3)
            })
        };
        let mut cut = false;
        let mut merge = false;
        let mut previous = false;
        let mut next = false;
        let mut clicked_word = None;
        let mut clicked_cue = None;
        ui.horizontal(|ui| {
            cut = icon_button(
                ui,
                widgets,
                "subtitles.transcript.delete",
                Text::SubTranscriptDelete,
                Icon::Trash,
                ready && self.transcript.selected() > 0,
                locale,
            );
            merge = icon_button(
                ui,
                widgets,
                "subtitles.transcript.merge",
                Text::SubMerge,
                Icon::ArrowsInLineHorizontal,
                ready && self.transcript.cues.len() > 1,
                locale,
            );
        });
        if let Some(text) = self.transcript.refusal {
            ui.colored_label(theme::error_text(dark), i18n::text(locale, text));
        }
        if view.cues.is_empty() {
            ui.horizontal(|ui| {
                ui.add(tinted(Icon::Subtitles, 16.0, theme::text(dark)));
                let r = ui.label(i18n::text(locale, Text::SubTranscriptEmpty));
                remember(widgets, "subtitles.transcript.empty".into(), &r);
            });
            return;
        }
        let pages = view.cues.len().div_ceil(PAGE);
        self.transcript.page = self.transcript.page.min(pages.saturating_sub(1));
        if pages > 1 {
            ui.horizontal(|ui| {
                previous = icon_button(
                    ui,
                    widgets,
                    "subtitles.transcript.previous",
                    Text::SubPrevious,
                    Icon::SkipBack,
                    self.transcript.page > 0,
                    locale,
                );
                ui.label(i18n::format(
                    locale,
                    Text::SubPage,
                    &[
                        ("page", &(self.transcript.page + 1).to_string()),
                        ("pages", &pages.to_string()),
                    ],
                ));
                next = icon_button(
                    ui,
                    widgets,
                    "subtitles.transcript.next",
                    Text::SubNext,
                    Icon::SkipForward,
                    self.transcript.page + 1 < pages,
                    locale,
                );
            });
        }
        for cue in view
            .cues
            .iter()
            .skip(self.transcript.page * PAGE)
            .take(PAGE)
        {
            let cue_id = CueId {
                item: cue.item,
                cue: cue.id.clone(),
            };
            let cue_selected = self.transcript.cues.contains(&cue_id);
            ui.horizontal_wrapped(|ui| {
                let time = format!(
                    "{} – {}",
                    timecode(cue.output.start()),
                    timecode(cue.output.end())
                );
                let r = ui.selectable_label(cue_selected, time);
                remember(
                    widgets,
                    format!(
                        "subtitles.transcript.cue.{}.{}",
                        cue.item.0,
                        cue.id.as_str()
                    ),
                    &r,
                );
                if r.clicked() {
                    clicked_cue = Some((
                        cue_id.clone(),
                        cue.output.start(),
                        ui.input(|i| i.modifiers),
                    ));
                }
                if cue.locked {
                    ui.add(tinted(Icon::Lock, 16.0, theme::text(dark)))
                        .on_hover_text(i18n::text(locale, Text::SubTranscriptLocked));
                }
                if cue.protected {
                    ui.label(i18n::text(locale, Text::SubEdited));
                }
                for word in &cue.words {
                    let word_selected = self.transcript.words.contains(&word.id);
                    let text = word
                        .text
                        .as_ref()
                        .map_or(i18n::text(locale, Text::SubTranscriptNoText), |text| {
                            text.as_str()
                        });
                    let color = if word.protected {
                        theme::warning(dark)
                    } else {
                        theme::text(dark)
                    };
                    let r = ui
                        .selectable_label(word_selected, egui::RichText::new(text).color(color))
                        .on_hover_text(timecode(word.output.start()));
                    remember(
                        widgets,
                        format!(
                            "subtitles.transcript.word.{}.{}.{}",
                            word.id.item.0,
                            word.id.cue.as_str(),
                            word.id.word
                        ),
                        &r,
                    );
                    if r.clicked() {
                        clicked_word = Some((
                            word.id.clone(),
                            word.output.start(),
                            ui.input(|i| i.modifiers),
                        ));
                    }
                }
            });
        }
        if previous {
            self.transcript.page = self.transcript.page.saturating_sub(1);
        }
        if next {
            self.transcript.page += 1;
        }
        // Gestures apply after every row is drawn, so the snapshot cannot recreate stale rows.
        if let Some((id, at, modifiers)) = clicked_word {
            self.transcript_select_word(&view, id, at, modifiers);
        } else if let Some((id, at, modifiers)) = clicked_cue {
            self.transcript_select_cue(id, at, modifiers);
        }
        if cut {
            self.transcript_cut(session);
        }
        if merge {
            self.transcript_merge(session);
        }
        if cut || merge {
            ui.ctx().request_repaint();
        }
    }

    /// Shift extends from the anchor word, Cmd/Ctrl toggles, a plain click replaces.
    /// Every click moves the playhead to the word.
    fn transcript_select_word(
        &mut self,
        view: &TranscriptView,
        id: WordId,
        at: Tick,
        modifiers: egui::Modifiers,
    ) {
        let pane = &mut self.transcript;
        pane.refusal = None;
        if modifiers.shift
            && let Some(anchor) = pane.anchor.clone()
        {
            let order: Vec<&WordId> = view
                .cues
                .iter()
                .flat_map(|cue| cue.words.iter().map(|word| &word.id))
                .collect();
            let from = order.iter().position(|word| **word == anchor);
            let to = order.iter().position(|word| **word == id);
            if let (Some(from), Some(to)) = (from, to) {
                if !modifiers.command {
                    pane.words.clear();
                }
                let (low, high) = (from.min(to), from.max(to));
                pane.words.extend(
                    order
                        .iter()
                        .skip(low)
                        .take(high - low + 1)
                        .map(|word| (*word).clone()),
                );
            }
        } else if modifiers.command {
            if !pane.words.remove(&id) {
                pane.words.insert(id.clone());
            }
            pane.anchor = Some(id);
        } else {
            pane.words.clear();
            pane.words.insert(id.clone());
            pane.anchor = Some(id);
        }
        self.requested_seek = Some(at);
    }

    /// Cmd/Ctrl toggles a cue for merging; a plain click selects only that cue.
    fn transcript_select_cue(&mut self, id: CueId, at: Tick, modifiers: egui::Modifiers) {
        let pane = &mut self.transcript;
        pane.refusal = None;
        if modifiers.command {
            if !pane.cues.remove(&id) {
                pane.cues.insert(id);
            }
        } else {
            pane.cues.clear();
            pane.cues.insert(id);
        }
        self.requested_seek = Some(at);
    }

    /// Cut the selected words as one engine command. A refusal keeps the selection
    /// and shows its reason; success clears the selection and parks the playhead at
    /// the earliest cut word.
    fn transcript_cut(&mut self, session: &mut Session) {
        if self.flush_drafts(session).is_err() {
            self.transcript.refusal = Some(Text::SubEditFailed);
            return;
        }
        self.transcript_refresh(session);
        let Some((_, view)) = self.transcript.view.clone() else {
            return;
        };
        let at = view
            .cues
            .iter()
            .flat_map(|cue| cue.words.iter())
            .filter(|word| self.transcript.words.contains(&word.id))
            .map(|word| word.output.start())
            .min();
        let word_ids: Vec<WordId> = self.transcript.words.iter().cloned().collect();
        match session.apply(Command::TranscriptDeleteRange { word_ids }) {
            Ok(outcome) => {
                self.transcript.refusal = None;
                if outcome.changed {
                    self.transcript.clear_selection();
                    if let Some(at) = at {
                        self.requested_seek = Some(at);
                    }
                }
            }
            Err(error) => self.transcript.refusal = Some(refusal(&error)),
        }
    }

    /// Merge the selected cues as one engine command; the selection is cleared on
    /// success because merged cues renumber their words.
    fn transcript_merge(&mut self, session: &mut Session) {
        if self.flush_drafts(session).is_err() {
            self.transcript.refusal = Some(Text::SubEditFailed);
            return;
        }
        self.transcript_refresh(session);
        let cue_ids: Vec<CueId> = self.transcript.cues.iter().cloned().collect();
        match session.apply(Command::TranscriptMergeCues { cue_ids }) {
            Ok(outcome) => {
                self.transcript.refusal = None;
                if outcome.changed {
                    self.transcript.clear_selection();
                }
            }
            Err(error) => self.transcript.refusal = Some(refusal(&error)),
        }
    }
}

/// Whether an item lives in a scene other than `scene`.
fn item_elsewhere(data: &project::ProjectData, item: ItemId, scene: project::SceneId) -> bool {
    data.scenes
        .iter()
        .filter(|other| other.id != scene)
        .flat_map(|other| other.timeline.iter())
        .flat_map(|track| &track.items)
        .any(|candidate| candidate.id == item)
}

/// The creator-facing reason for a refused transcript command.
fn refusal(error: &EditError) -> Text {
    match error {
        EditError::Protected => Text::SubTranscriptProtected,
        EditError::SceneBoundary => Text::SubTranscriptScene,
        EditError::Locked(_) => Text::SubTranscriptLocked,
        EditError::CuesNotAdjacent | EditError::CuesAcrossTracks | EditError::CuesAcrossItems => {
            Text::SubTranscriptNotAdjacent
        }
        _ => Text::SubEditFailed,
    }
}

fn remember(widgets: &mut Vec<(String, egui::Response)>, id: String, r: &egui::Response) {
    r.ctx.accesskit_node_builder(r.id, |n| n.set_author_id(&id));
    widgets.push((id, r.clone()));
}

/// Icon-only button with its tooltip; the same 28 px box the cue tools use.
fn icon_button(
    ui: &mut egui::Ui,
    widgets: &mut Vec<(String, egui::Response)>,
    id: &str,
    text: Text,
    icon: Icon,
    enabled: bool,
    locale: Locale,
) -> bool {
    let scope = ui.make_persistent_id(id);
    let r = ui
        .scope_builder(egui::UiBuilder::new().scope_id(scope), |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::image(tinted(icon, 16.0, ui.visuals().text_color()))
                    .min_size(egui::vec2(28.0, 28.0)),
            )
        })
        .inner
        .on_hover_text(i18n::text(locale, text));
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled, i18n::text(locale, text))
    });
    remember(widgets, id.into(), &r);
    r.clicked()
}
