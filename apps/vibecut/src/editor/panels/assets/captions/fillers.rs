//! Filler cut list: the reviewed lexicon candidates of the caption item, as a section of the Captions tab.
use super::Subtitles;
use super::view::{Controls, time_text};
use crate::{
    egui,
    i18n::{self, Locale, Text},
    kit::Icon,
    services::ai_access,
};
use captions::{CueWordTiming, FillerKind, FillerLexicon, NfcText};
use engine::{
    Session,
    control::{ControlResult, FillerCandidate},
};
use project::ItemId;
use std::collections::BTreeSet;

/// The cut list's state. Candidates belong to one caption item; a different item clears it.
#[derive(Default)]
pub(super) struct Cuts {
    item: Option<ItemId>,
    scanned: bool,
    candidates: Vec<FillerCandidate>,
    /// Candidate ids the next removal passes to `timeline.remove_fillers`.
    selected: BTreeSet<NfcText>,
    /// Ids waiting for the edited-cue confirmation row.
    confirm: Option<Vec<NfcText>>,
    problem: Option<Text>,
    removed: Option<usize>,
}

impl Subtitles {
    /// The cut list section under the cue list. Its state resets when the caption item changes.
    pub(super) fn fillers_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        c: &mut Controls,
        locale: Locale,
    ) {
        if self.fillers.item != self.item {
            self.fillers = Cuts {
                item: self.item,
                ..Cuts::default()
            };
        }
        let busy = self.busy();
        let rate = session.project().data().canvas.fps;
        let mut find = false;
        let mut apply = false;
        let mut answer = None;
        let header = egui::CollapsingHeader::new(c.tr(Text::FillersTitle))
            .id_salt("subtitles.fillers")
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    find = c.button(
                        ui,
                        "subtitles.fillers.find",
                        Text::FillersFind,
                        Icon::MagnifyingGlass,
                        true,
                    );
                    if self.fillers.scanned {
                        if let Some(problem) = self.fillers.problem {
                            ui.label(c.tr(problem));
                        } else if self.fillers.candidates.is_empty() {
                            ui.label(c.tr(Text::FillersNone));
                        } else {
                            for candidate in self.fillers.candidates.clone() {
                                let mut on = self.fillers.selected.contains(&candidate.id);
                                let kind = match candidate.kind {
                                    FillerKind::FilledPause => Text::FillersKindPause,
                                    FillerKind::DiscourseMarker => Text::FillersKindMarker,
                                };
                                let label = format!(
                                    "{} · {}",
                                    time_text(candidate.range.start, rate, locale),
                                    c.tr(kind)
                                );
                                let r = ui.checkbox(&mut on, label);
                                c.remember(
                                    format!("subtitles.fillers.toggle.{}", candidate.id.as_str()),
                                    &r,
                                );
                                if r.changed() {
                                    if on {
                                        self.fillers.selected.insert(candidate.id.clone());
                                    } else {
                                        self.fillers.selected.remove(&candidate.id);
                                    }
                                }
                            }
                            apply = c.button(
                                ui,
                                "subtitles.fillers.apply",
                                Text::FillersApply,
                                Icon::Scissors,
                                !self.fillers.selected.is_empty(),
                            );
                        }
                    }
                    if self.fillers.confirm.is_some() {
                        ui.label(c.tr(Text::FillersEdited));
                        if c.button(
                            ui,
                            "subtitles.fillers.confirm",
                            Text::FillersConfirm,
                            Icon::Scissors,
                            true,
                        ) {
                            answer = Some(true);
                        }
                        if c.button(
                            ui,
                            "subtitles.fillers.confirm.cancel",
                            Text::Cancel,
                            Icon::X,
                            true,
                        ) {
                            answer = Some(false);
                        }
                    }
                    if let Some(count) = self.fillers.removed {
                        ui.label(i18n::format(
                            locale,
                            Text::FillersRemoved,
                            &[("count", &i18n::number(locale, count as u64))],
                        ));
                    }
                });
            });
        c.remember("subtitles.fillers", &header.header_response);
        match answer {
            Some(true) => {
                if let Some(ids) = self.fillers.confirm.take() {
                    self.remove_fillers(session, ids, true);
                }
            }
            Some(false) => self.fillers.confirm = None,
            None => {}
        }
        if apply {
            let ids: Vec<NfcText> = self
                .fillers
                .candidates
                .iter()
                .filter(|candidate| self.fillers.selected.contains(&candidate.id))
                .map(|candidate| candidate.id.clone())
                .collect();
            let edited = self
                .fillers
                .candidates
                .iter()
                .any(|candidate| {
                    candidate.edited_cue && self.fillers.selected.contains(&candidate.id)
                });
            if edited {
                self.fillers.confirm = Some(ids);
            } else {
                self.remove_fillers(session, ids, false);
            }
        }
        if find {
            self.scan_fillers(session);
        }
    }

    /// Read the lexicon candidates of the selected caption item; every candidate starts checked.
    fn scan_fillers(&mut self, session: &mut Session) {
        self.fillers.scanned = true;
        self.fillers.candidates.clear();
        self.fillers.selected.clear();
        self.fillers.confirm = None;
        self.fillers.removed = None;
        let (language, timed) = match self.caption(session) {
            Some((_, params)) => (
                params.cues.cues.first().map(|cue| cue.source_lang.clone()),
                params
                    .cues
                    .cues
                    .iter()
                    .any(|cue| matches!(cue.word_timing, CueWordTiming::Words { .. })),
            ),
            None => (None, false),
        };
        let Some(language) = language.filter(|_| timed) else {
            self.fillers.problem = Some(Text::FillersNoTimings);
            return;
        };
        if FillerLexicon::builtin(&language).is_err() {
            self.fillers.problem = Some(Text::FillersUnsupported);
            return;
        }
        let result = ai_access::run(
            session,
            &mut self.runner,
            None,
            None,
            "timeline.filler_candidates",
            serde_json::json!({"language": language.as_str(), "include_markers": false}),
        );
        match result {
            Ok(ControlResult::Fillers { candidates }) => {
                self.fillers.selected = candidates
                    .iter()
                    .map(|candidate| candidate.id.clone())
                    .collect();
                self.fillers.candidates = candidates;
                self.fillers.problem = None;
            }
            _ => self.fillers.problem = Some(Text::FillersFailed),
        }
    }

    /// Remove the passed candidates in one registry command, one undo step, then re-read the list.
    fn remove_fillers(&mut self, session: &mut Session, ids: Vec<NfcText>, confirm_edited: bool) {
        let count = ids.len();
        if self.flush_drafts(session).is_err() {
            self.fillers.problem = Some(Text::FillersFailed);
            return;
        }
        let result = ai_access::run(
            session,
            &mut self.runner,
            None,
            None,
            "timeline.remove_fillers",
            serde_json::json!({"ids": ids, "confirm_edited": confirm_edited}),
        );
        self.scan_fillers(session);
        match result {
            Ok(_) => self.fillers.removed = Some(count),
            Err(_) => self.fillers.problem = Some(Text::FillersFailed),
        }
    }
}
