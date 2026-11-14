//! Keyword emphasis of the caption cue under the playhead, drawn in the caption item's Text tab.
//! The playhead picks the cue and the Captions panel owns the suggestion run; this section draws
//! the read it gets and emits `KeywordAction`s, which the frame routes back to that panel.
use super::Panel;
use crate::{
    editor::{
        context::{KeywordAction, KeywordRun},
        properties::{Action, theme},
    },
    egui,
    i18n::Text,
    kit::{Icon, tinted},
};
use project::ItemId;

impl Panel<'_, '_> {
    /// The collapsed Keywords section. Its chips and buttons act on the caption cue under the
    /// playhead, and say so when no cue is there.
    pub(in super::super) fn keywords_section(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(keywords) = self.cx.keywords.clone() else {
            return;
        };
        if keywords.item != item {
            return;
        }
        let idle = !keywords.busy;
        let has_cue = keywords.cue.is_some();
        let header = egui::CollapsingHeader::new(self.tr(Text::EmphasisTitle))
            .id_salt("inspector.keywords")
            .show(ui, |ui| {
                // The properties body runs with zero item spacing; the section restores the
                // kit's density gap between its chips, actions and rows.
                ui.spacing_mut().item_spacing = egui::vec2(theme::space::MD, theme::space::XS);
                if !has_cue {
                    ui.label(self.tr(Text::EmphasisNoCue));
                }
                ui.horizontal_wrapped(|ui| {
                    if self.keyword_button(
                        ui,
                        "inspector.keywords.choose",
                        Text::EmphasisChoose,
                        Icon::TextT,
                        idle && has_cue && keywords.timed,
                    ) {
                        self.actions.push(Action::Keywords(KeywordAction::Choose));
                    }
                    if self.keyword_button(
                        ui,
                        "inspector.keywords.preview",
                        Text::EmphasisPreview,
                        Icon::Play,
                        idle && has_cue,
                    ) {
                        self.actions.push(Action::Keywords(KeywordAction::Preview));
                    }
                    if self.keyword_button(
                        ui,
                        "inspector.keywords.clear",
                        Text::EmphasisClear,
                        Icon::X,
                        idle && has_cue && (!keywords.shown.is_empty() || keywords.stale),
                    ) {
                        self.actions.push(Action::Keywords(KeywordAction::Clear));
                    }
                    if self.keyword_button(
                        ui,
                        "inspector.keywords.suggest",
                        Text::EmphasisAi,
                        Icon::Sparkle,
                        idle && has_cue,
                    ) {
                        self.actions.push(Action::Keywords(KeywordAction::Suggest));
                    }
                });
                if has_cue && !keywords.timed {
                    ui.label(self.tr(Text::EmphasisNoTimings));
                }
                if keywords.stale {
                    ui.label(self.tr(Text::EmphasisStale));
                } else if has_cue && !keywords.highlight {
                    ui.label(self.tr(Text::EmphasisHighlightOff));
                }
                if has_cue && !keywords.words.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for (id, text) in &keywords.words {
                            let on = keywords.shown.contains(id);
                            let full = !on && keywords.shown.len() >= keywords.limit;
                            let response = ui.add_enabled(
                                idle && !full,
                                chip(ui.visuals().dark_mode, text.as_str(), on),
                            );
                            self.remember(format!("inspector.keywords.word.{id}"), &response);
                            if response.clicked() {
                                let mut next = keywords.shown.clone();
                                if on {
                                    next.retain(|value| *value != *id);
                                } else {
                                    next.push(*id);
                                    next.sort_unstable();
                                }
                                self.actions
                                    .push(Action::Keywords(KeywordAction::Pick(next)));
                            }
                        }
                    });
                }
                if keywords.run != KeywordRun::Idle {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let waiting = if keywords.run == KeywordRun::AwaitingApproval {
                            Text::EmphasisAwaitingApproval
                        } else {
                            Text::EmphasisWorking
                        };
                        ui.label(self.tr(waiting));
                        if self.keyword_button(
                            ui,
                            "inspector.keywords.cancel",
                            Text::Cancel,
                            Icon::X,
                            true,
                        ) {
                            self.actions.push(Action::Keywords(KeywordAction::Cancel));
                        }
                    });
                }
                if let Some(problem) = keywords.problem {
                    ui.label(self.tr(problem));
                }
                if keywords.suggested > 0
                    && self.keyword_button(
                        ui,
                        "inspector.keywords.apply",
                        Text::EmphasisApply,
                        Icon::TextT,
                        idle,
                    )
                {
                    self.actions.push(Action::Keywords(KeywordAction::Apply));
                }
            });
        self.remember("inspector.keywords", &header.header_response);
    }

    /// An icon and label button; returns whether it was clicked this frame.
    fn keyword_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        text: Text,
        icon: Icon,
        enabled: bool,
    ) -> bool {
        let label = self.tr(text);
        let response = ui.add_enabled(
            enabled,
            egui::Button::image_and_text(tinted(icon, 16.0, ui.visuals().text_color()), label),
        );
        self.remember(id, &response);
        response.clicked()
    }
}

/// A keyword chip. The chosen chip uses the sub-navigation's selected style, as the open
/// scene row does: an accent label on the shared hover fill, with no outline. egui's
/// `selected` would fill it with the accent and put the same accent under its label.
fn chip(dark: bool, text: &str, on: bool) -> egui::Button<'_> {
    let label = egui::RichText::new(text).color(if on {
        theme::accent(dark)
    } else {
        theme::text(dark)
    });
    let button = egui::Button::new(label);
    if on {
        button.fill(theme::hover(dark)).stroke(egui::Stroke::NONE)
    } else {
        button
    }
}
