//! Reviews a transcript's pauses, repeats and sentences before they are cut.
use super::{
    Subtitles,
    transcript::{icon_button, refusal, remember},
    view::{Controls, time_text},
};
use crate::{
    egui,
    i18n::{self, Locale, Text},
    kit::{self as theme, Icon},
};
use captions::NfcText;
use engine::{Command, Session, TranscriptAnalysis, TranscriptView, WordId, control::OutputSpan};
use project::ItemId;
use std::{
    collections::BTreeSet,
    sync::{Arc, mpsc},
};
use time::{TICKS_PER_SECOND, Tick};

/// The shortest silence reported until the creator changes it, the default of the pause
/// candidates (`PauseSettings`).
const DEFAULT_GAP_MS: i64 = 500;
/// Crossfade of a removed pause, the default of `timeline.pause_candidates`.
const PAUSE_CROSSFADE_MS: i64 = 20;
/// List row pitch (14b/110059, 2x Retina: 94 px between hairlines, 47 logical).
const ROW_HEIGHT: f32 = 47.0;
/// Pause capsule height (14b/110059, 2x Retina: 36 px, 18 logical).
const PILL_HEIGHT: f32 = 18.0;
/// Rows shown before the list scrolls. Ours, unmeasured: the reference list fills its window.
const LIST_ROWS: f32 = 8.0;
/// Mini timeline height. Ours, unmeasured: the reference strip fills the window foot.
const TIMELINE_HEIGHT: f32 = 48.0;
/// Analyzing modal button (14b/110018, 2x Retina: 320 x 44 px, 160 x 22 logical).
const MODAL_BUTTON: egui::Vec2 = egui::vec2(160.0, 22.0);

/// The list a review tab shows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    All,
    Pauses,
    Repeats,
}

/// One analysis running on a worker. Dropping it cancels: the worker's result has no receiver.
pub(super) struct Run {
    pub(super) receive: mpsc::Receiver<TranscriptAnalysis>,
    pub(super) project: Arc<project::Project>,
    pub(super) view: Arc<TranscriptView>,
}

/// A finished analysis and the rows the creator ticked. It belongs to one project snapshot.
struct Open {
    project: Arc<project::Project>,
    view: Arc<TranscriptView>,
    analysis: TranscriptAnalysis,
    tab: Tab,
    gaps: BTreeSet<usize>,
    repeats: BTreeSet<usize>,
    cues: BTreeSet<(ItemId, NfcText)>,
}

/// Words one cut removed, kept for Show deleted words while that cut stays in the history.
struct Deleted {
    text: String,
    at: Tick,
    depth: usize,
}

/// The review's state, never persisted.
pub(super) struct Panel {
    pub(super) run: Option<Run>,
    open: Option<Open>,
    min_gap_ms: i64,
    show_deleted: bool,
    deleted: Vec<Deleted>,
    problem: Option<Text>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            run: None,
            open: None,
            min_gap_ms: DEFAULT_GAP_MS,
            show_deleted: false,
            deleted: Vec::new(),
            problem: None,
        }
    }
}

impl Panel {
    /// Remember the words of one finished cut. `depth` is the undo depth after the cut.
    pub(super) fn record_deleted<'a>(
        &mut self,
        view: &TranscriptView,
        ids: impl Iterator<Item = &'a WordId>,
        depth: usize,
    ) {
        let wanted: BTreeSet<&WordId> = ids.collect();
        let mut at: Option<Tick> = None;
        let mut text = Vec::new();
        for word in view.cues.iter().flat_map(|cue| cue.words.iter()) {
            if wanted.contains(&word.id) {
                let start = word.output.start();
                at = Some(at.map_or(start, |earliest| earliest.min(start)));
                if let Some(word_text) = &word.text {
                    text.push(word_text.as_str());
                }
            }
        }
        if let Some(at) = at {
            self.deleted.push(Deleted {
                text: text.join(" "),
                at,
                depth,
            });
        }
    }
}

/// What one list row stands for.
enum Kind {
    Cue(ItemId, NfcText),
    Pause(usize),
    Repeat(usize),
}

struct Row {
    kind: Kind,
    label: String,
    start: Tick,
    checked: bool,
}

impl Row {
    fn id(&self) -> String {
        match &self.kind {
            Kind::Cue(item, cue) => format!("subtitles.review.cue.{}.{}", item.0, cue.as_str()),
            Kind::Pause(index) => format!("subtitles.review.pause.{index}"),
            Kind::Repeat(index) => format!("subtitles.review.repeat.{index}"),
        }
    }
}

impl Open {
    fn new(
        project: Arc<project::Project>,
        view: Arc<TranscriptView>,
        analysis: TranscriptAnalysis,
    ) -> Self {
        Self {
            project,
            view,
            analysis,
            tab: Tab::All,
            gaps: BTreeSet::new(),
            repeats: BTreeSet::new(),
            cues: BTreeSet::new(),
        }
    }

    /// The rows of the current tab in time order. All holds sentences and pauses.
    fn rows(&self, no_text: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.tab != Tab::Repeats {
            for (index, gap) in self.analysis.gaps.iter().enumerate() {
                rows.push(Row {
                    kind: Kind::Pause(index),
                    label: String::new(),
                    start: gap.span.start,
                    checked: self.gaps.contains(&index),
                });
            }
        }
        if self.tab == Tab::All {
            for cue in &self.view.cues {
                let words: Vec<&str> = cue
                    .words
                    .iter()
                    .map(|word| word.text.as_ref().map_or(no_text, |text| text.as_str()))
                    .collect();
                let key = (cue.item, cue.id.clone());
                rows.push(Row {
                    checked: self.cues.contains(&key),
                    kind: Kind::Cue(key.0, key.1),
                    label: words.join(" "),
                    start: cue.output.start(),
                });
            }
        }
        if self.tab == Tab::Repeats {
            for (index, repeat) in self.analysis.repeats.iter().enumerate() {
                rows.push(Row {
                    kind: Kind::Repeat(index),
                    label: repeat.text.as_str().to_owned(),
                    start: repeat.span.start,
                    checked: self.repeats.contains(&index),
                });
            }
        }
        rows.sort_by_key(|row| row.start);
        rows
    }

    /// Ticks the ticked rows would remove, overlaps counted once.
    fn removed(&self) -> i64 {
        let mut spans: Vec<(i64, i64)> = Vec::new();
        for gap in self.gaps.iter().filter_map(|i| self.analysis.gaps.get(*i)) {
            spans.push((gap.span.start.0, gap.span.end.0));
        }
        for repeat in self
            .repeats
            .iter()
            .filter_map(|i| self.analysis.repeats.get(*i))
        {
            spans.push((repeat.span.start.0, repeat.span.end.0));
        }
        for cue in &self.view.cues {
            if self.cues.contains(&(cue.item, cue.id.clone()))
                && let (Some(first), Some(last)) = (cue.words.first(), cue.words.last())
            {
                spans.push((first.output.start().0, last.output.end().0));
            }
        }
        spans.sort_unstable();
        let mut total = 0_i64;
        let mut reach = i64::MIN;
        for (start, end) in spans {
            let start = start.max(reach);
            if end > start {
                total = total.saturating_add(end - start);
            }
            reach = reach.max(end);
        }
        total
    }

    fn any_checked(&self) -> bool {
        !(self.gaps.is_empty() && self.repeats.is_empty() && self.cues.is_empty())
    }

    /// Tick or untick every row of the current tab.
    fn set_all(&mut self, on: bool, no_text: &str) {
        for row in self.rows(no_text) {
            self.set(&row.kind, on);
        }
    }

    fn set(&mut self, kind: &Kind, on: bool) {
        match kind {
            Kind::Cue(item, cue) => {
                let key = (*item, cue.clone());
                if on {
                    self.cues.insert(key);
                } else {
                    self.cues.remove(&key);
                }
            }
            Kind::Pause(index) => {
                if on {
                    self.gaps.insert(*index);
                } else {
                    self.gaps.remove(index);
                }
            }
            Kind::Repeat(index) => {
                if on {
                    self.repeats.insert(*index);
                } else {
                    self.repeats.remove(index);
                }
            }
        }
    }
}

/// The silence length of `tick` milliseconds.
fn gap_ticks(ms: i64) -> Tick {
    Tick(ms.saturating_mul(TICKS_PER_SECOND) / 1000)
}


impl Subtitles {
    /// Whether no analysis is running.
    pub(super) fn review_idle(&self) -> bool {
        self.review.run.is_none()
    }

    pub(super) fn review_deleted_shown(&self) -> bool {
        self.review.show_deleted
    }

    pub(super) fn review_toggle_deleted(&mut self) {
        self.review.show_deleted = !self.review.show_deleted;
    }

    /// Cancel a running analysis. Its late result has no receiver and is dropped.
    pub(super) fn review_cancel(&mut self) {
        self.review.run = None;
    }

    pub(super) fn review_busy(&self) -> bool {
        self.review.run.is_some()
    }

    /// Read the transcript and analyze it on a worker; the analyzing modal covers the run.
    pub(super) fn review_start(&mut self, session: &Session) {
        self.review.problem = None;
        self.review.open = None;
        let Ok(view) = session.transcript() else {
            self.review.problem = Some(Text::SubReviewFailed);
            return;
        };
        let view = Arc::new(view);
        let worker = view.clone();
        let min_gap = gap_ticks(self.review.min_gap_ms);
        let (send, receive) = mpsc::sync_channel(1);
        wake::spawn(move || {
            let _ = send.send(engine::analyze_transcript(&worker, min_gap));
        });
        self.review.run = Some(Run {
            receive,
            project: session.project().clone(),
            view,
        });
    }

    /// Take a finished analysis and drop the cuts of an undone history. Called every tick.
    pub(super) fn review_tick(&mut self, session: &Session) {
        let depth = session.history_len().0;
        self.review.deleted.retain(|deleted| deleted.depth <= depth);
        let Some(run) = &self.review.run else {
            return;
        };
        match run.receive.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.review.run = None;
                self.review.problem = Some(Text::SubReviewFailed);
            }
            Ok(analysis) => {
                if let Some(run) = self.review.run.take()
                    && Arc::ptr_eq(&run.project, session.project())
                {
                    self.review.open = Some(Open::new(run.project, run.view, analysis));
                }
            }
        }
    }

    /// Read the transcript again with the current minimum, after an edit this review made.
    fn review_reopen(&mut self, session: &Session) {
        self.review.open = None;
        let Ok(view) = session.transcript() else {
            self.review.problem = Some(Text::SubReviewFailed);
            return;
        };
        let analysis = engine::analyze_transcript(&view, gap_ticks(self.review.min_gap_ms));
        self.review.open = Some(Open::new(
            session.project().clone(),
            Arc::new(view),
            analysis,
        ));
    }

    /// The analyzing modal: the same wording and one Cancel as the reference.
    pub(super) fn analyzing_ui(&mut self, ctx: &egui::Context, c: &mut Controls) {
        if self.review.run.is_none() {
            return;
        }
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("subtitles.review.analyzing"))
            .frame(crate::kit::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(MODAL_BUTTON.x);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    crate::kit::dialog_title(ui, c.tr(Text::SubReviewAnalyzing));
                    ui.label(c.tr(Text::SubReviewAnalyzingRepeats));
                    let r = ui.add(egui::Button::new(c.tr(Text::Cancel)).min_size(MODAL_BUTTON));
                    c.remember("subtitles.review.cancel", &r);
                    cancel = r.clicked();
                });
            });
        if cancel {
            self.review_cancel();
        }
    }

    /// The deleted-words list, the gap card, the tabbed review list, the mini timeline and the
    /// footer. Everything here edits through engine commands.
    pub(super) fn review_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        locale: Locale,
        dark: bool,
        widgets: &mut Vec<(String, egui::Response)>,
    ) {
        if self
            .review
            .open
            .as_ref()
            .is_some_and(|open| !Arc::ptr_eq(&open.project, session.project()))
        {
            self.review.open = None;
        }
        if let Some(problem) = self.review.problem {
            ui.colored_label(theme::error_text(dark), i18n::text(locale, problem));
        }
        if self.review.show_deleted {
            self.deleted_ui(ui, session, locale, widgets);
        }
        let ready = self.edit_ready(session);
        let rate = session.project().data().canvas.fps;
        let code = self.time_code;
        let history = session.history_len();
        let scene_end = session.project().data().scene().duration();
        let no_text = i18n::text(locale, Text::SubTranscriptNoText);
        let mut set_tab = None;
        let mut toggled: Vec<(Kind, bool)> = Vec::new();
        let mut select_all = None;
        let mut seek = None;
        let mut gap_ms = None;
        let mut remove_gaps = false;
        let mut delete = false;
        let mut close = false;
        let mut undo = false;
        let mut redo = false;
        let Some(open) = self.review.open.as_ref() else {
            return;
        };
        let rows = open.rows(no_text);
        let gaps = open.analysis.gaps.len();
        let gap_total: i64 = open
            .analysis
            .gaps
            .iter()
            .map(|gap| gap.span.end.0 - gap.span.start.0)
            .sum();
        let seconds = self.review.min_gap_ms as f64 / 1000.0;
        // The gap card.
        egui::Frame::new()
            .fill(theme::strip(dark))
            .corner_radius(theme::radius::CONTROL)
            .inner_margin(theme::space::MD)
            .show(ui, |ui| {
                let sentence = if gaps == 0 {
                    i18n::format(
                        locale,
                        Text::SubReviewGapNone,
                        &[("seconds", &i18n::decimal(locale, seconds, 1))],
                    )
                } else {
                    i18n::format(
                        locale,
                        Text::SubReviewGapCard,
                        &[
                            ("seconds", &i18n::decimal(locale, seconds, 1)),
                            ("count", &i18n::number(locale, gaps as u64)),
                            (
                                "total",
                                &i18n::decimal(
                                    locale,
                                    gap_total as f64 / TICKS_PER_SECOND as f64,
                                    1,
                                ),
                            ),
                        ],
                    )
                };
                let r = ui.label(sentence);
                remember(widgets, "subtitles.review.gap_card".into(), &r);
                ui.horizontal_wrapped(|ui| {
                    ui.label(i18n::text(locale, Text::SubReviewGapDuration));
                    let mut value = seconds;
                    let r = ui.add(
                        egui::DragValue::new(&mut value)
                            .range(0.1..=10.0)
                            .speed(0.1)
                            .fixed_decimals(1)
                            .suffix(" s"),
                    );
                    remember(widgets, "subtitles.review.gap_duration".into(), &r);
                    if r.changed() {
                        gap_ms = Some((value * 1000.0).round() as i64);
                    }
                    ui.add_enabled_ui(ready && gaps > 0, |ui| {
                        let r = ui.button(i18n::text(locale, Text::SubReviewGapRemove));
                        remember(widgets, "subtitles.review.gap_remove".into(), &r);
                        remove_gaps = r.clicked();
                    });
                });
            });
        // The tabs.
        let ids = [
            "subtitles.review.tab.all".to_owned(),
            "subtitles.review.tab.pauses".to_owned(),
            "subtitles.review.tab.repeats".to_owned(),
        ];
        let labels = [
            i18n::text(locale, Text::SubReviewAll),
            i18n::text(locale, Text::SubReviewPauses),
            i18n::text(locale, Text::SubReviewRepeats),
        ];
        let selected = match open.tab {
            Tab::All => 0,
            Tab::Pauses => 1,
            Tab::Repeats => 2,
        };
        for (index, r) in theme::text_tabs(ui, &ids, &labels, selected)
            .into_iter()
            .enumerate()
        {
            if r.clicked() {
                set_tab = Some(match index {
                    0 => Tab::All,
                    1 => Tab::Pauses,
                    _ => Tab::Repeats,
                });
            }
        }
        // Select all, then the rows.
        let mut all = !rows.is_empty() && rows.iter().all(|row| row.checked);
        let r = theme::checkbox(
            ui,
            "subtitles.review.select_all",
            &mut all,
            i18n::text(locale, Text::SubSelectAll),
        );
        if r.changed() {
            select_all = Some(all);
        }
        if rows.is_empty() {
            let text = match open.tab {
                Tab::Repeats => Text::SubReviewNoneRepeats,
                Tab::Pauses => Text::SubReviewNonePauses,
                Tab::All => Text::SubTranscriptEmpty,
            };
            ui.label(i18n::text(locale, text));
        }
        egui::ScrollArea::vertical()
            .id_salt("subtitles.review.rows")
            .max_height(ROW_HEIGHT * LIST_ROWS)
            .show(ui, |ui| {
                for row in &rows {
                    let id = row.id();
                    let width = ui.available_width();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
                    let mut cell = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let shown = match &row.kind {
                        Kind::Pause(index) => {
                            let length = open
                                .analysis
                                .gaps
                                .get(*index)
                                .map_or(0, |gap| gap.span.end.0 - gap.span.start.0);
                            i18n::format(
                                locale,
                                Text::SubReviewPause,
                                &[(
                                    "seconds",
                                    &i18n::decimal(
                                        locale,
                                        length as f64 / TICKS_PER_SECOND as f64,
                                        2,
                                    ),
                                )],
                            )
                        }
                        _ => row.label.clone(),
                    };
                    let mut on = row.checked;
                    let r = theme::checkbox_box(&mut cell, &id, &mut on, &shown);
                    if r.changed() {
                        let kind = match &row.kind {
                            Kind::Cue(item, cue) => Kind::Cue(*item, cue.clone()),
                            Kind::Pause(index) => Kind::Pause(*index),
                            Kind::Repeat(index) => Kind::Repeat(*index),
                        };
                        toggled.push((kind, on));
                    }
                    remember(widgets, id.clone(), &r);
                    // The start time sits at the right edge; the sentence takes the rest.
                    let stamp = cell.painter().text(
                        egui::pos2(rect.right(), rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        time_text(row.start, rate, code),
                        theme::label_font(),
                        theme::text(dark),
                    );
                    cell.set_max_width((rect.width() - stamp.width() - theme::space::MD).max(0.0));
                    let text_response = match &row.kind {
                        Kind::Pause(_) => {
                            let mut text = egui::RichText::new(&shown);
                            if row.checked {
                                text = text.strikethrough();
                            }
                            egui::Frame::new()
                                .stroke(egui::Stroke::new(1.0, theme::border(dark)))
                                .corner_radius((PILL_HEIGHT / 2.0) as u8)
                                .inner_margin(egui::Margin::symmetric(8, 0))
                                .show(&mut cell, |ui| {
                                    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
                                })
                                .inner
                        }
                        _ => {
                            let mut text = egui::RichText::new(&shown);
                            if row.checked {
                                text = text.strikethrough();
                            }
                            cell.add(egui::Label::new(text).truncate().sense(egui::Sense::click()))
                        }
                    };
                    remember(widgets, format!("{id}.text"), &text_response);
                    if text_response.clicked() {
                        seek = Some(row.start);
                    }
                    ui.painter().hline(
                        rect.x_range(),
                        rect.bottom(),
                        egui::Stroke::new(1.0, theme::separator(dark)),
                    );
                }
            });
        // The mini timeline with its undo and redo.
        ui.horizontal(|ui| {
            undo = icon_button(
                ui,
                widgets,
                "subtitles.review.undo",
                Text::SubReviewUndo,
                Icon::ArrowCounterClockwise,
                ready && history.0 > 0,
                locale,
            );
            redo = icon_button(
                ui,
                widgets,
                "subtitles.review.redo",
                Text::SubReviewRedo,
                Icon::ArrowClockwise,
                ready && history.1 > 0,
                locale,
            );
        });
        if let Some(at) = mini_timeline(ui, open, scene_end, dark, widgets) {
            seek = Some(at);
        }
        // The footer.
        let after = Tick(scene_end.0.saturating_sub(open.removed()));
        let total = i18n::format(
            locale,
            Text::SubReviewTotal,
            &[("time", &time_text(after, rate, code))],
        );
        let r = ui.label(total);
        remember(widgets, "subtitles.review.total".into(), &r);
        let can_delete = ready && open.any_checked();
        ui.horizontal_wrapped(|ui| {
            // Compound clips are not built yet: the option stays off and says why.
            ui.add_enabled_ui(false, |ui| {
                let mut off = false;
                let r = theme::checkbox(
                    ui,
                    "subtitles.review.compound",
                    &mut off,
                    i18n::text(locale, Text::SubReviewCompound),
                );
                let _tooltip =
                    r.on_disabled_hover_text(i18n::text(locale, Text::SubReviewCompoundWait));
            });
            let r = theme::dialog_button(
                ui,
                "subtitles.review.close",
                i18n::text(locale, Text::Cancel),
                false,
            );
            close = r.clicked();
            ui.add_enabled_ui(can_delete, |ui| {
                let r = theme::dialog_button(
                    ui,
                    "subtitles.review.delete",
                    i18n::text(locale, Text::Delete),
                    true,
                );
                delete = r.clicked();
            });
        });
        // Gestures apply after every row is drawn.
        if let Some(tab) = set_tab
            && let Some(open) = self.review.open.as_mut()
        {
            open.tab = tab;
        }
        if let Some(on) = select_all
            && let Some(open) = self.review.open.as_mut()
        {
            open.set_all(on, no_text);
        }
        if let Some(open) = self.review.open.as_mut() {
            for (kind, on) in &toggled {
                open.set(kind, *on);
            }
        }
        if let Some(at) = seek {
            self.requested_seek = Some(at);
        }
        if let Some(ms) = gap_ms {
            self.review.min_gap_ms = ms.clamp(100, 10_000);
            self.review_reopen(session);
        }
        if close {
            self.review.open = None;
        }
        if undo || redo {
            let result = if undo { session.undo() } else { session.redo() };
            if result.is_err() {
                self.review.problem = Some(Text::SubEditFailed);
            }
        }
        if remove_gaps {
            self.review_apply(session, true);
        } else if delete {
            self.review_apply(session, false);
        }
        if !toggled.is_empty() || remove_gaps || delete || undo || redo {
            ui.ctx().request_repaint();
        }
    }

    /// Cut what the review ticked as one `batch`, so it is one undo step: the ticked repeat
    /// words and sentences as `transcript.delete_range`, then the ticked pauses as
    /// `timeline.remove_pauses`. Every range is computed here against the timeline the review
    /// was read from: the word cut closes exactly the merged runs of its words, so a pause span
    /// moves back by the length of the runs that end before it. A refusal changes nothing and
    /// shows its reason. The list is read again afterwards.
    fn review_apply(&mut self, session: &mut Session, every_gap: bool) {
        let Some(open) = self.review.open.as_ref() else {
            return;
        };
        let mut ids: BTreeSet<WordId> = BTreeSet::new();
        if !every_gap {
            for repeat in open
                .repeats
                .iter()
                .filter_map(|index| open.analysis.repeats.get(*index))
            {
                ids.extend(repeat.words.iter().cloned());
            }
            for cue in &open.view.cues {
                if open.cues.contains(&(cue.item, cue.id.clone())) {
                    ids.extend(cue.words.iter().map(|word| word.id.clone()));
                }
            }
        }
        let mut ranges: Vec<(i64, i64)> = open
            .view
            .cues
            .iter()
            .flat_map(|cue| cue.words.iter())
            .filter(|word| ids.contains(&word.id))
            .map(|word| (word.output.start().0, word.output.end().0))
            .collect();
        ranges.sort_unstable();
        let mut runs: Vec<(i64, i64)> = Vec::new();
        for (start, end) in ranges {
            match runs.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => runs.push((start, end)),
            }
        }
        let closed = |tick: Tick| -> Tick {
            let length: i64 = runs
                .iter()
                .filter(|run| run.1 <= tick.0)
                .map(|run| run.1 - run.0)
                .sum();
            Tick(tick.0 - length)
        };
        let spans: Vec<OutputSpan> = open
            .analysis
            .gaps
            .iter()
            .enumerate()
            .filter(|(index, _)| every_gap || open.gaps.contains(index))
            .map(|(_, gap)| OutputSpan {
                start: closed(gap.span.start),
                end: closed(gap.span.end),
            })
            .collect();
        let earliest = open
            .analysis
            .gaps
            .iter()
            .enumerate()
            .filter(|(index, _)| every_gap || open.gaps.contains(index))
            .map(|(_, gap)| gap.span.start)
            .chain(runs.first().map(|run| Tick(run.0)))
            .min();
        let before = open.view.clone();
        let mut commands = Vec::new();
        if !ids.is_empty() {
            commands.push(Command::TranscriptDeleteRange {
                word_ids: ids.iter().cloned().collect(),
            });
        }
        if !spans.is_empty() {
            commands.push(Command::RemovePauses {
                spans,
                crossfade_ms: PAUSE_CROSSFADE_MS,
            });
        }
        let command = match commands.len() {
            0 => return,
            1 => commands.remove(0),
            _ => Command::Batch { commands },
        };
        if self.flush_drafts(session).is_err() {
            self.review.problem = Some(Text::SubEditFailed);
            return;
        }
        self.review.problem = None;
        match session.apply(command) {
            Ok(outcome) => {
                if outcome.changed && !ids.is_empty() {
                    self.review
                        .record_deleted(&before, ids.iter(), session.history_len().0);
                }
            }
            Err(error) => {
                self.review.problem = Some(refusal(&error));
                return;
            }
        }
        if let Some(at) = earliest {
            self.requested_seek = Some(at);
        }
        self.review_reopen(session);
    }

    /// Words cut through this pane, newest last, while their cut is still in the history.
    fn deleted_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        locale: Locale,
        widgets: &mut Vec<(String, egui::Response)>,
    ) {
        let rate = session.project().data().canvas.fps;
        let code = self.time_code;
        if self.review.deleted.is_empty() {
            let r = ui.label(i18n::text(locale, Text::SubReviewDeletedNone));
            remember(widgets, "subtitles.review.deleted.none".into(), &r);
            return;
        }
        for (index, deleted) in self.review.deleted.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(time_text(deleted.at, rate, code));
                let r = ui.label(egui::RichText::new(&deleted.text).strikethrough());
                remember(widgets, format!("subtitles.review.deleted.{index}"), &r);
            });
        }
    }
}

/// The strip under the list: sentences, pauses and ticked ranges over the scene, and a click
/// moves the playhead. Ours, unmeasured: the reference strip shows only a ruler at frame zoom.
fn mini_timeline(
    ui: &mut egui::Ui,
    open: &Open,
    end: Tick,
    dark: bool,
    widgets: &mut Vec<(String, egui::Response)>,
) -> Option<Tick> {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TIMELINE_HEIGHT),
        egui::Sense::click(),
    );
    let span = end.0.max(1);
    let x = |tick: Tick| {
        rect.left() + rect.width() * (tick.0.clamp(0, span) as f64 / span as f64) as f32
    };
    let band = |start: Tick, finish: Tick| {
        egui::Rect::from_min_max(
            egui::pos2(x(start), rect.top() + 6.0),
            egui::pos2(x(finish).max(x(start) + 1.0), rect.bottom() - 6.0),
        )
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::radius::CONTROL, theme::strip(dark));
    for cue in &open.view.cues {
        painter.rect_filled(
            band(cue.output.start(), cue.output.end()),
            0.0,
            theme::accent(dark).gamma_multiply(0.45),
        );
    }
    for (index, gap) in open.analysis.gaps.iter().enumerate() {
        let color = if open.gaps.contains(&index) {
            theme::error_text(dark)
        } else {
            theme::warning(dark)
        };
        painter.rect_filled(band(gap.span.start, gap.span.end), 0.0, color);
    }
    for index in &open.repeats {
        if let Some(repeat) = open.analysis.repeats.get(*index) {
            painter.rect_stroke(
                band(repeat.span.start, repeat.span.end),
                0.0,
                egui::Stroke::new(1.0, theme::error_text(dark)),
                egui::StrokeKind::Inside,
            );
        }
    }
    remember(widgets, "subtitles.review.timeline".into(), &response);
    let at = response.interact_pointer_pos().filter(|_| response.clicked())?;
    let ratio = f64::from(((at.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0));
    Some(Tick((ratio * span as f64) as i64))
}
