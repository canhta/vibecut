//! Draws the Animation control every visual element shares.
use super::Panel;
use crate::editor::properties::{Action, ItemId, Text, egui, read, theme};
use crate::kit;
use engine::{AnimationPreset as Look, AnimationSlot as Slot};
use time::{Rounding, Tick};

/// Draft key of the duration field.
const DRAFT: &str = "animation_duration";
/// Automation marker the frame sets while the element cannot take an edit.
pub(super) const DISABLED: &str = "animation.edit";
/// Measured grid (capture 221759, logical px): six columns of 58 px square tiles, 15 px apart.
const COLUMNS: usize = 6;
const TILE: f32 = 58.0;
const GUTTER: f32 = 15.0;
const FIELD_WIDTH: f32 = 68.0;
/// Stepper step in seconds; the engine clamps a duration to one frame..the clip.
const STEP_SECONDS: f64 = 0.1;

const fn slot_word(slot: Slot) -> &'static str {
    match slot {
        Slot::In => "in",
        Slot::Out => "out",
        Slot::Combo => "combo",
    }
}

fn look_text(look: Look) -> Text {
    match look {
        Look::Fade => Text::AnimationFade,
        Look::SlideLeft => Text::AnimationSlideLeft,
        Look::SlideRight => Text::AnimationSlideRight,
        Look::SlideUp => Text::AnimationSlideUp,
        Look::SlideDown => Text::AnimationSlideDown,
        Look::ZoomIn => Text::AnimationZoomIn,
        Look::ZoomOut => Text::AnimationZoomOut,
        Look::Spin => Text::AnimationSpin,
        Look::Drop => Text::AnimationDrop,
        Look::Rise => Text::AnimationRise,
        Look::Pop => Text::AnimationPop,
        Look::Tilt => Text::AnimationTilt,
        Look::Pulse => Text::AnimationPulse,
        Look::Heartbeat => Text::AnimationHeartbeat,
        Look::Sway => Text::AnimationSway,
        Look::Swing => Text::AnimationSwing,
        Look::Float => Text::AnimationFloat,
        Look::DriftLeft => Text::AnimationDriftLeft,
        Look::DriftRight => Text::AnimationDriftRight,
        Look::PushIn => Text::AnimationPushIn,
        Look::PullOut => Text::AnimationPullOut,
        Look::Shake => Text::AnimationShake,
        Look::Flicker => Text::AnimationFlicker,
        Look::Breathe => Text::AnimationBreathe,
        Look::Turn => Text::AnimationTurn,
    }
}

impl Panel<'_, '_> {
    /// In, Out and Combo sub-tabs, the looks of the open one (None first) and, for In and Out,
    /// the duration. Every change is one `clip.set_animation` request. The Text tab and the
    /// clip tabs both draw this control.
    pub(super) fn animation_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let cx = self.cx;
        let project = cx.session.project();
        let Some(found) = read::item(project, item) else {
            return;
        };
        let current = engine::ClipAnimations::of(found, &project.data().canvas);
        let snapshot = project.clone();
        let enabled = !cx.disabled.contains(DISABLED);
        ui.add_enabled_ui(enabled, |ui| {
            let slot = self.animation_tabs(ui);
            ui.add_space(theme::space::XS);
            let held = match slot {
                Slot::In => current.enter,
                Slot::Out => current.exit,
                Slot::Combo => current.combo.map(|look| (look, Tick::ZERO)),
            };
            let chosen = self.animation_tiles(ui, slot, held.map(|(look, _)| look));
            if let Some(preset) = chosen {
                self.actions.push(Action::SetClipAnimation {
                    item,
                    slot,
                    preset,
                    duration: None,
                    snapshot: snapshot.clone(),
                });
            }
            if let Some((look, duration)) = held.filter(|_| slot != Slot::Combo) {
                self.animation_duration(ui, item, (slot, look, duration), &snapshot);
            }
        });
    }

    /// The In / Out / Combo bar; returns the open sub-tab.
    fn animation_tabs(&mut self, ui: &mut egui::Ui) -> Slot {
        let slots = [Slot::In, Slot::Out, Slot::Combo];
        let ids: Vec<String> = slots
            .iter()
            .map(|slot| format!("inspector.animation.tab.{}", slot_word(*slot)))
            .collect();
        let labels = [
            self.tr(Text::AnimationIn),
            self.tr(Text::AnimationOut),
            self.tr(Text::AnimationCombo),
        ];
        let open = self.state.animation_slot;
        let selected = slots.iter().position(|slot| *slot == open).unwrap_or(0);
        let cells = kit::segmented(ui, &ids, &labels, selected);
        for ((id, cell), slot) in ids.iter().zip(&cells).zip(slots) {
            self.remember(id.clone(), cell);
            if cell.clicked() && slot != open {
                self.state.animation_slot = slot;
                self.state.drafts.remove(DRAFT);
                self.state.focus = None;
            }
        }
        self.state.animation_slot
    }

    /// The None tile and one tile per look, six to a row. Returns the clicked entry:
    /// `Some(None)` removes the slot.
    fn animation_tiles(
        &mut self,
        ui: &mut egui::Ui,
        slot: Slot,
        held: Option<Look>,
    ) -> Option<Option<Look>> {
        let mut entries: Vec<Option<Look>> = vec![None];
        entries.extend(Look::for_slot(slot).iter().copied().map(Some));
        let mut chosen = None;
        for row in entries.chunks(COLUMNS) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GUTTER;
                for entry in row {
                    let label = entry.map_or(self.tr(Text::AnimationNone), |look| {
                        self.tr(look_text(look))
                    });
                    let word = entry.map_or("none", Look::word);
                    let selected = *entry == held;
                    let response = ui
                        .add_sized([TILE, TILE], egui::Button::new(label).selected(selected))
                        .on_hover_text(self.tr(Text::TextAnimationTip));
                    self.remember(
                        format!("inspector.animation.{}.{word}", slot_word(slot)),
                        &response,
                    );
                    if response.clicked() && !selected {
                        chosen = Some(*entry);
                    }
                }
            });
            ui.add_space(GUTTER);
        }
        chosen
    }

    /// Draft field (commit on Enter or blur, Escape cancels) with its stepper.
    fn animation_duration(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        (slot, look, current): (Slot, Look, Tick),
        snapshot: &std::sync::Arc<project::Project>,
    ) {
        let label = self.tr(Text::AnimationDuration);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, label);
        value_ui.spacing_mut().item_spacing.x = theme::space::XXS;
        let mut next = self
            .state
            .drafts
            .get(DRAFT)
            .cloned()
            .unwrap_or_else(|| format!("{:.1}s", current.seconds()));
        let response = kit::value_field(
            &mut value_ui,
            "inspector.animation.duration",
            &mut next,
            FIELD_WIDTH,
            None,
            label,
        );
        self.remember("inspector.animation.duration", &response);
        if response.has_focus() {
            self.state.focus = Some(DRAFT.to_owned());
        }
        if response.changed() {
            self.state.drafts.insert(DRAFT.to_owned(), next);
        }
        value_ui.add_space(4.0);
        let (up, down) = kit::stepper(&mut value_ui, "inspector.animation.duration", label);
        self.remember("inspector.animation.duration.increase", &up);
        self.remember("inspector.animation.duration.decrease", &down);
        let change = |seconds: f64| (slot, look, current, seconds);
        for (clicked, sign) in [(up.clicked(), 1.0), (down.clicked(), -1.0)] {
            if clicked {
                self.state.drafts.remove(DRAFT);
                self.set_animation_duration(
                    item,
                    change(current.seconds() + sign * STEP_SECONDS),
                    snapshot,
                );
            }
        }
        let editing = response.has_focus() || self.state.focus.as_deref() == Some(DRAFT);
        if editing && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(DRAFT);
            self.state.focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.state.focus = None;
            if let Some(draft) = self.state.drafts.remove(DRAFT) {
                match parse(&draft) {
                    Some(seconds) => self.set_animation_duration(item, change(seconds), snapshot),
                    None => self.fail(crate::editor::Error::Invalid("animation duration")),
                }
            }
        }
    }

    fn set_animation_duration(
        &mut self,
        item: ItemId,
        (slot, look, current, seconds): (Slot, Look, Tick, f64),
        snapshot: &std::sync::Arc<project::Project>,
    ) {
        let Ok(duration) = Tick::from_seconds_f64(seconds, Rounding::Nearest) else {
            return;
        };
        if duration != current {
            self.actions.push(Action::SetClipAnimation {
                item,
                slot,
                preset: Some(look),
                duration: Some(duration),
                snapshot: snapshot.clone(),
            });
        }
    }
}

fn parse(text: &str) -> Option<f64> {
    let value = text
        .trim()
        .trim_end_matches('s')
        .trim()
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value)
}
