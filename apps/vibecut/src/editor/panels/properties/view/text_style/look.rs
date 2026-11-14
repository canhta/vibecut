//! The Text tab's Basic, Bubble and Effects sub-tabs: the bubble and effect grids of a text or
//! caption item, the Apply to all box of a caption and Reset. Every edit is one engine command.
use super::Panel;
use crate::editor::properties::{Action, ItemId, Text, TextSub, egui, read, theme};
use crate::kit;
use project::{TextBubble, TextEffect};

/// Measured grid (capture 2026-11-14f): four columns of 62 px square tiles, 16 px apart, on a
/// 78 px pitch. The first tile is the empty choice.
const COLUMNS: usize = 4;
const TILE: f32 = 62.0;
const GUTTER: f32 = 16.0;
/// Texture side: the tile at twice its logical size.
const SAMPLE_SIDE: u32 = 124;

const fn bubble_text(bubble: TextBubble) -> Text {
    match bubble {
        TextBubble::Round => Text::TextBubbleRound,
        TextBubble::Pill => Text::TextBubblePill,
        TextBubble::Oval => Text::TextBubbleOval,
        TextBubble::SpeechDown => Text::TextBubbleSpeechDown,
        TextBubble::SpeechLeft => Text::TextBubbleSpeechLeft,
        TextBubble::Banner => Text::TextBubbleBanner,
        TextBubble::Tag => Text::TextBubbleTag,
        TextBubble::Burst => Text::TextBubbleBurst,
        TextBubble::Cloud => Text::TextBubbleCloud,
        TextBubble::Ticket => Text::TextBubbleTicket,
        TextBubble::Frame => Text::TextBubbleFrame,
    }
}

const fn effect_text(effect: TextEffect) -> Text {
    match effect {
        TextEffect::Neon => Text::TextEffectNeon,
        TextEffect::Pop => Text::TextEffectPop,
        TextEffect::Comic => Text::TextEffectComic,
        TextEffect::Hollow => Text::TextEffectHollow,
        TextEffect::Retro => Text::TextEffectRetro,
        TextEffect::Sticker => Text::TextEffectSticker,
        TextEffect::Fire => Text::TextEffectFire,
        TextEffect::Ice => Text::TextEffectIce,
        TextEffect::LongShadow => Text::TextEffectLongShadow,
        TextEffect::Gold => Text::TextEffectGold,
        TextEffect::Soft => Text::TextEffectSoft,
        TextEffect::Glitch => Text::TextEffectGlitch,
    }
}

/// The look a tile stands for; `None` is the empty choice.
#[derive(Clone, Copy)]
enum Look {
    Bubble(Option<TextBubble>),
    Effect(Option<TextEffect>),
}

impl Panel<'_, '_> {
    /// The Basic, Bubble and Effects segmented control at the top of the Text tab.
    pub(in crate::editor::panels::properties::view) fn text_sub_tabs(
        &mut self,
        ui: &mut egui::Ui,
    ) -> TextSub {
        let current = self.state.text_sub;
        let subs = [TextSub::Basic, TextSub::Bubble, TextSub::Effects];
        let ids: Vec<String> = subs
            .iter()
            .map(|sub| format!("inspector.text_sub.{}", sub.id()))
            .collect();
        let labels = [
            self.tr(Text::Basic),
            self.tr(Text::TextBubbleTab),
            self.tr(Text::Effects),
        ];
        let at = subs.iter().position(|sub| *sub == current).unwrap_or(0);
        ui.add_space(1.0);
        let cells = kit::segmented(ui, &ids, &labels, at);
        for ((id, response), sub) in ids.iter().zip(&cells).zip(subs) {
            self.remember(id.clone(), response);
            if response.clicked() {
                self.state.text_sub = sub;
            }
        }
        ui.add_space(12.0);
        self.state.text_sub
    }

    /// The Bubble or Effects body: for a caption the Apply to all box, then the grid and Reset.
    pub(in crate::editor::panels::properties::view) fn text_look_tab(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        sub: TextSub,
    ) {
        let Some(item_params) =
            read::item(self.cx.session.project(), item).map(|found| found.content.clone())
        else {
            return;
        };
        let (caption, bubble, effect) = match &item_params {
            project::ItemParams::Text(p) => (false, p.bubble, p.effect),
            project::ItemParams::Caption(p) => (true, p.bubble, p.effect),
            _ => return,
        };
        if caption {
            let mut all = self.state.caption_apply_all;
            let response = kit::checkbox(
                ui,
                "inspector.caption_apply_all",
                &mut all,
                self.tr(Text::TextApplyToAll),
            );
            self.remember("inspector.caption_apply_all", &response);
            self.state.caption_apply_all = all;
            ui.add_space(12.0);
        }
        let looks: Vec<(Look, &'static str, Text)> = match sub {
            TextSub::Bubble => std::iter::once((Look::Bubble(None), "none", Text::TextLookNone))
                .chain(
                    TextBubble::ALL
                        .into_iter()
                        .map(|b| (Look::Bubble(Some(b)), b.id(), bubble_text(b))),
                )
                .collect(),
            _ => std::iter::once((Look::Effect(None), "none", Text::TextLookNone))
                .chain(
                    TextEffect::ALL
                        .into_iter()
                        .map(|e| (Look::Effect(Some(e)), e.id(), effect_text(e))),
                )
                .collect(),
        };
        let family = if sub == TextSub::Bubble {
            "bubble"
        } else {
            "effect"
        };
        let mut chosen = None;
        for row in looks.chunks(COLUMNS) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GUTTER;
                for (look, word, label) in row {
                    let selected = match look {
                        Look::Bubble(value) => *value == bubble,
                        Look::Effect(value) => *value == effect,
                    };
                    if self.look_tile(ui, family, *look, word, *label, selected) {
                        chosen = Some(*look);
                    }
                }
            });
            ui.add_space(GUTTER);
        }
        let reset_on = match sub {
            TextSub::Bubble => bubble.is_some(),
            _ => effect.is_some(),
        };
        let reset = ui.add_enabled(reset_on, egui::Button::new(self.tr(Text::TextLookReset)));
        self.remember("inspector.text_look.reset", &reset);
        if reset.clicked() {
            chosen = Some(match sub {
                TextSub::Bubble => Look::Bubble(None),
                _ => Look::Effect(None),
            });
        }
        if let Some(look) = chosen {
            self.choose_look(item, caption, look);
        }
    }

    /// One tile: the sample, an accent border when it is the item's look.
    fn look_tile(
        &mut self,
        ui: &mut egui::Ui,
        family: &str,
        look: Look,
        word: &str,
        label: Text,
        selected: bool,
    ) -> bool {
        let dark = self.cx.dark;
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(TILE), egui::Sense::click());
        let radius = theme::radius::UPLOAD;
        ui.painter().rect_filled(rect, radius, theme::field(dark));
        let key = format!("{family}.{word}");
        let (bubble, effect) = match look {
            Look::Bubble(value) => (value, None),
            Look::Effect(value) => (None, value),
        };
        if bubble.is_some() || effect.is_some() {
            let texture = self
                .state
                .look_samples
                .entry(key.clone())
                .or_insert_with(|| {
                    let pixels = render::look_sample(bubble, effect, SAMPLE_SIDE)
                        .unwrap_or_else(|| vec![0; (SAMPLE_SIDE * SAMPLE_SIDE * 4) as usize]);
                    ui.ctx().load_texture(
                        format!("text_look.{key}"),
                        egui::ColorImage::from_rgba_premultiplied(
                            [SAMPLE_SIDE as usize; 2],
                            &pixels,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                });
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            // The empty choice: a ring with a slash.
            let c = rect.center();
            let stroke = egui::Stroke::new(1.5, theme::secondary(dark));
            ui.painter().circle_stroke(c, 12.0, stroke);
            ui.painter().line_segment(
                [c + egui::vec2(-8.5, 8.5), c + egui::vec2(8.5, -8.5)],
                stroke,
            );
        }
        if selected || response.hovered() || response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                radius,
                egui::Stroke::new(if selected { 2.0 } else { 1.0 }, theme::accent(dark)),
                egui::StrokeKind::Inside,
            );
        }
        let name = self.tr(label);
        let response = response.on_hover_text(name);
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
        self.remember(format!("inspector.text_{family}.{word}"), &response);
        response.clicked() && !selected
    }

    /// One command: a text item sets its field, a caption item its own look or, with Apply to
    /// all, every caption item's.
    fn choose_look(&mut self, item: ItemId, caption: bool, look: Look) {
        if !caption {
            let (field, word) = match look {
                Look::Bubble(value) => ("text_bubble", value.map_or("none", TextBubble::id)),
                Look::Effect(value) => ("text_effect", value.map_or("none", TextEffect::id)),
            };
            self.set_item_field(item, field, word);
            return;
        }
        let items: Vec<ItemId> = if self.state.caption_apply_all {
            self.cx
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .filter(|found| matches!(found.content, project::ItemParams::Caption(_)))
                .map(|found| found.id)
                .collect()
        } else {
            vec![item]
        };
        self.actions.push(match look {
            Look::Bubble(bubble) => Action::SetCaptionBubble { items, bubble },
            Look::Effect(effect) => Action::SetCaptionEffect { items, effect },
        });
    }
}
