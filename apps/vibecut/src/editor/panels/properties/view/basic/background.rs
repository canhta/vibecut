//! Draws the Background sub-tab of a video clip: the kind row (None, Colour,
//! Blur), then the colour row for Colour or the three blur presets for
//! Blur, then Apply to all. The tab is listed only while the clip leaves canvas open.
use super::super::Panel;
use crate::editor::properties::{Action, ColorTarget, Icon, ItemId, Text, egui, read, theme};
use crate::kit;
use project::ClipBackground;

impl Panel<'_, '_> {
    pub(in super::super) fn background_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let project = self.cx.session.project();
        let Some(current) = read::clip_background(project, item) else {
            return;
        };
        let editable = !read::item_locked(project, item);
        self.state.background_memory.remember(current);
        ui.add_enabled_ui(editable, |ui| {
            self.background_kind_row(ui, item, current);
            ui.add_space(theme::space::MD);
            match current {
                ClipBackground::None => {}
                ClipBackground::Solid { .. } => {
                    self.color_row(
                        ui,
                        "inspector.clip_background.color",
                        Text::BackgroundColor,
                        ColorTarget::ClipBackground(item),
                    );
                }
                ClipBackground::Blur { intensity } => self.blur_presets_row(ui, item, intensity),
            }
            ui.add_space(theme::space::MD);
            self.apply_all_row(ui, current);
        });
    }

    /// None, Colour or Blur. Choosing a kind starts from the value it last showed this session.
    fn background_kind_row(&mut self, ui: &mut egui::Ui, item: ItemId, current: ClipBackground) {
        let index = match current {
            ClipBackground::None => 0,
            ClipBackground::Solid { .. } => 1,
            ClipBackground::Blur { .. } => 2,
        };
        let ids: Vec<String> = ["none", "color", "blur"]
            .iter()
            .map(|kind| format!("inspector.clip_background.kind.{kind}"))
            .collect();
        let labels = [
            self.tr(Text::ClipKindNone),
            self.tr(Text::ClipKindColor),
            self.tr(Text::ClipKindBlur),
        ];
        let cells = kit::segmented(ui, &ids, &labels, index);
        for (position, (id, response)) in ids.iter().zip(&cells).enumerate() {
            self.remember(id.clone(), response);
            if !response.clicked() || position == index {
                continue;
            }
            let memory = self.state.background_memory;
            let background = match position {
                0 => ClipBackground::None,
                1 => ClipBackground::Solid {
                    color: memory.colour_or_black(),
                },
                _ => ClipBackground::Blur {
                    intensity: memory.blur_or_default(),
                },
            };
            self.actions.push(Action::SetClipBackground {
                items: vec![item],
                background,
            });
        }
    }

    /// Light, Medium and Heavy; the preset the intensity equals is selected.
    fn blur_presets_row(&mut self, ui: &mut egui::Ui, item: ItemId, intensity: u32) {
        let index = ClipBackground::BLUR_PRESETS
            .iter()
            .position(|preset| *preset == intensity);
        let ids: Vec<String> = (0..ClipBackground::BLUR_PRESETS.len())
            .map(|position| format!("inspector.clip_background.blur.{position}"))
            .collect();
        let labels = [
            self.tr(Text::ClipBlurLight),
            self.tr(Text::ClipBlurMedium),
            self.tr(Text::ClipBlurHeavy),
        ];
        let cells = kit::segmented(ui, &ids, &labels, index.unwrap_or(usize::MAX));
        for (position, (id, response)) in ids.iter().zip(&cells).enumerate() {
            self.remember(id.clone(), response);
            if response.clicked()
                && Some(position) != index
                && let Some(preset) = ClipBackground::BLUR_PRESETS.get(position)
            {
                self.actions.push(Action::SetClipBackground {
                    items: vec![item],
                    background: ClipBackground::Blur { intensity: *preset },
                });
            }
        }
    }

    /// Copies this clip's background to every unlocked video item as one undo step.
    fn apply_all_row(&mut self, ui: &mut egui::Ui, current: ClipBackground) {
        let items = read::unlocked_video_items(self.cx.session.project());
        if items.is_empty() {
            return;
        }
        let width = ui.available_width();
        let apply = kit::action_button(
            ui,
            "inspector.clip_background.apply_all",
            Icon::Copy,
            self.tr(Text::ApplyToAll),
            width,
        );
        self.remember("inspector.clip_background.apply_all", &apply);
        if apply.clicked() {
            self.actions.push(Action::SetClipBackground {
                items,
                background: current,
            });
        }
    }
}
