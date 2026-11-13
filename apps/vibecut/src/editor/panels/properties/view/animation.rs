//! Draws the Animation control every visual element shares.
use super::Panel;
use crate::editor::properties::{
    ANIMATION_DRAFT as DRAFT, Action, ItemId, Text, egui, read, theme,
};
use crate::kit;
use engine::animation_previews::PREVIEW_FRAMES;
use engine::{AnimationPreset as Look, AnimationSlot as Slot};
use time::{Rounding, Tick};

mod previews;
pub(in crate::editor) use previews::Previews;

/// Automation marker the frame sets while the element cannot take an edit.
pub(super) const DISABLED: &str = "animation.edit";
/// Measured grid (capture 221759, logical px): six columns of 58 px square tiles, 15 px apart.
const COLUMNS: usize = 6;
const TILE: f32 = 58.0;
const GUTTER: f32 = 15.0;
/// Name under a tile (capture 221759: a 12 px line), and the gap that brings the row pitch
/// to the measured 82 px (58 + 12 + 12).
const NAME_HEIGHT: f32 = 12.0;
const ROW_GAP: f32 = 12.0;
/// The frame a resting tile shows, a third of the way into its motion: In and Combo near
/// the start, Out near the end, where the picture is still moving.
const fn rest_frame(slot: Slot) -> u32 {
    match slot {
        Slot::In => 2,
        Slot::Out => 8,
        Slot::Combo => 3,
    }
}
/// A hovered tile plays its whole strip once a second, on a loop.
const LOOP_SECONDS: f64 = 1.0;
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
            self.animation_shelf(ui);
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

    /// The Favorites shelf: a toggle above the grid that filters it to the starred looks.
    fn animation_shelf(&mut self, ui: &mut egui::Ui) {
        let selected = self.state.animation_favorites;
        let response = ui.selectable_label(selected, self.tr(Text::AssetsFavorites));
        self.remember("inspector.animation.favorites".to_owned(), &response);
        if response.clicked() {
            self.state.animation_favorites = !selected;
        }
        ui.add_space(theme::space::XS);
    }

    /// The None tile and one tile per look, six to a row. Returns the clicked entry:
    /// `Some(None)` removes the slot. The Favorites shelf shows only starred looks.
    fn animation_tiles(
        &mut self,
        ui: &mut egui::Ui,
        slot: Slot,
        held: Option<Look>,
    ) -> Option<Option<Look>> {
        let favorites = self.state.animation_favorites;
        let mut entries: Vec<Option<Look>> = if favorites { Vec::new() } else { vec![None] };
        entries.extend(
            Look::for_slot(slot)
                .iter()
                .copied()
                .filter(|look| {
                    !favorites
                        || self
                            .cx
                            .preferences
                            .catalog_favorites
                            .contains("animations", look.word())
                })
                .map(Some),
        );
        if entries.is_empty() {
            ui.label(
                egui::RichText::new(self.tr(Text::AssetsFavoritesEmpty))
                    .font(theme::label_font())
                    .size(theme::type_size::SMALL_LABEL)
                    .color(self.text_color()),
            );
            return None;
        }
        if self.state.animation_previews.is_none() {
            self.state.animation_previews = Previews::new();
        }
        let ctx = ui.ctx().clone();
        let busy = self
            .state
            .animation_previews
            .as_mut()
            .is_some_and(|previews| previews.update(&ctx, slot, Look::for_slot(slot)));
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let mut chosen = None;
        for row in entries.chunks(COLUMNS) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GUTTER;
                for entry in row {
                    if let Some(clicked) = self.animation_tile(ui, slot, *entry, held) {
                        chosen = Some(clicked);
                    }
                }
            });
            ui.add_space(ROW_GAP);
        }
        chosen
    }

    /// One tile: the look's preview with its name beneath; the None tile is empty. Returns
    /// the entry when it is clicked and was not already held.
    fn animation_tile(
        &mut self,
        ui: &mut egui::Ui,
        slot: Slot,
        entry: Option<Look>,
        held: Option<Look>,
    ) -> Option<Option<Look>> {
        let dark = self.cx.dark;
        let label = entry.map_or(self.tr(Text::AnimationNone), |look| {
            self.tr(look_text(look))
        });
        let word = entry.map_or("none", Look::word);
        let selected = entry == held;
        let mut clicked = None;
        ui.vertical(|ui| {
            ui.set_width(TILE);
            ui.spacing_mut().item_spacing.y = 0.0;
            let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::splat(TILE), egui::Sense::click());
            let radius = theme::radius::UPLOAD;
            ui.painter().rect_filled(rect, radius, theme::field(dark));
            let strip = entry.and_then(|look| {
                self.state
                    .animation_previews
                    .as_ref()
                    .and_then(|previews| previews.strip(slot, look))
            });
            if let Some(texture) = strip {
                let frames = f64::from(PREVIEW_FRAMES);
                let frame = if response.hovered() {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(16));
                    let phase = (ui.input(|input| input.time) / LOOP_SECONDS).fract();
                    ((phase * frames) as u32).min(PREVIEW_FRAMES - 1)
                } else {
                    rest_frame(slot)
                };
                let left = (f64::from(frame) / frames) as f32;
                let right = (f64::from(frame + 1) / frames) as f32;
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(right, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            if selected || response.hovered() || response.has_focus() {
                ui.painter().rect_stroke(
                    rect,
                    radius,
                    egui::Stroke::new(1.0, theme::accent(dark)),
                    egui::StrokeKind::Inside,
                );
            }
            let response = response.on_hover_text(self.tr(Text::TextAnimationTip));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            self.remember(
                format!("inspector.animation.{}.{word}", slot_word(slot)),
                &response,
            );
            if let Some(look) = entry {
                let starred = self
                    .cx
                    .preferences
                    .catalog_favorites
                    .contains("animations", look.word());
                let star = egui::Rect::from_min_size(
                    rect.right_top() - egui::vec2(24.0, 0.0),
                    egui::Vec2::splat(24.0),
                );
                let star_label = self.tr(if starred {
                    Text::AssetsFavoriteRemove
                } else {
                    Text::AssetsFavoriteAdd
                });
                let star_response = ui
                    .interact(
                        star,
                        egui::Id::new(("inspector.animation.favorite", look.word())),
                        egui::Sense::click(),
                    )
                    .on_hover_text(star_label);
                self.remember(
                    format!("inspector.animation.favorite.{}", look.word()),
                    &star_response,
                );
                ui.painter().text(
                    star.center(),
                    egui::Align2::CENTER_CENTER,
                    if starred { "★" } else { "☆" },
                    theme::label_font(),
                    self.text_color(),
                );
                if star_response.clicked() {
                    self.actions.push(Action::ToggleAnimationFavorite {
                        preset: look,
                        favorite: !starred,
                    });
                }
            }
            // The name line is exactly NAME_HEIGHT; an 11 px label's natural row is taller,
            // so it sits in a child that does not grow the tile and stretch the pitch.
            let (name, _) =
                ui.allocate_exact_size(egui::vec2(TILE, NAME_HEIGHT), egui::Sense::hover());
            ui.new_child(egui::UiBuilder::new().max_rect(name)).add(
                egui::Label::new(
                    egui::RichText::new(label)
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL)
                        .color(self.text_color()),
                )
                .truncate(),
            );
            if response.clicked() && !selected {
                clicked = Some(entry);
            }
        });
        clicked
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
