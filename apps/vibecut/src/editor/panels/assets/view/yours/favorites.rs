//! Renders the Media tab's Favorites page: the starred catalogue items and their empty state.
use super::{Action, AssetsTab, Icon, Id, Rect, Text, Vec2, View, egui, icons, theme};
use crate::i18n;

/// The catalogue families a star can sit in: the stored id and the label.
const FAMILIES: [(&str, Text); 6] = [
    ("effects", Text::Effects),
    ("transitions", Text::Transitions),
    ("filters", Text::Filters),
    ("animations", Text::MediaFavoritesAnimations),
    ("stickers", Text::Stickers),
    ("sounds", Text::Sounds),
];

/// What a click on a starred item does: the same action its own tab's tile pushes.
enum Apply {
    Effect(&'static str),
    Filter(&'static str),
    Transition(&'static str),
    Sticker(String),
    Sound(u64),
}

/// One starred item listed on the page. Animations are starred and applied from the
/// properties panel, so their rows carry no action.
struct Starred {
    key: String,
    name: String,
    apply: Option<Apply>,
}

impl View<'_> {
    /// The starred items of one family, oldest star first. A star whose catalogue entry is
    /// gone is skipped.
    fn starred(&self, family: &str) -> Vec<Starred> {
        let favorites = &self.cx.preferences.catalog_favorites;
        let mut out = Vec::new();
        if family == "sounds" {
            for sound in favorites.sounds() {
                out.push(Starred {
                    key: sound.id.to_string(),
                    name: sound.title.clone(),
                    apply: Some(Apply::Sound(sound.id)),
                });
            }
            return out;
        }
        for id in favorites.list(family) {
            let (name, apply) = match family {
                "effects" => {
                    let Some(definition) = project::effect_definitions()
                        .iter()
                        .find(|definition| definition.id == id.as_str())
                    else {
                        continue;
                    };
                    let name = crate::i18n::effect_label(definition.id, None)
                        .map_or(definition.name, |key| self.tr(key));
                    (name.to_owned(), Some(Apply::Effect(definition.id)))
                }
                "filters" => {
                    let Some(definition) = project::filter_definition(id) else {
                        continue;
                    };
                    (
                        definition.name.to_owned(),
                        Some(Apply::Filter(definition.id)),
                    )
                }
                "transitions" => {
                    let Some(definition) = project::TRANSITION_DEFINITIONS
                        .iter()
                        .find(|definition| definition.kind == id.as_str())
                    else {
                        continue;
                    };
                    (
                        self.tr(i18n::transition_label(definition.kind)).to_owned(),
                        Some(Apply::Transition(definition.kind)),
                    )
                }
                "stickers" => (id.clone(), Some(Apply::Sticker(id.clone()))),
                _ => (id.clone(), None),
            };
            out.push(Starred {
                key: id.clone(),
                name,
                apply,
            });
        }
        out
    }

    pub(in crate::editor::panels::assets::view) fn favorites(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let tool = Rect::from_min_size(
            egui::pos2(area.right() - 36.0, area.top()),
            Vec2::new(36.0, theme::size::CONTROL),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.favorites.filter.tool")
                .max_rect(tool),
            |ui| self.favorites_filter(ui),
        );
        let chosen = self.state.media_favorites_family;
        let groups: Vec<_> = FAMILIES
            .iter()
            .filter(|(family, _)| chosen.is_none_or(|chosen| chosen == *family))
            .map(|(family, label)| (*family, *label, self.starred(family)))
            .filter(|(_, _, items)| !items.is_empty())
            .collect();
        let body = Rect::from_min_max(
            egui::pos2(area.left(), tool.bottom() + theme::space::MD),
            area.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.favorites.body")
                .max_rect(body),
            |ui| {
                if groups.is_empty() {
                    self.favorites_empty(ui, body);
                    return;
                }
                let dark = self.cx.preferences.dark;
                egui::ScrollArea::vertical()
                    .id_salt("media.favorites.list")
                    .show(ui, |ui| {
                        for (family, label, items) in groups {
                            let count =
                                i18n::number(self.cx.preferences.locale, items.len() as u64);
                            let header = format!("{}  {count}", self.tr(label));
                            let response = ui.label(
                                egui::RichText::new(header)
                                    .font(theme::label_font())
                                    .size(theme::type_size::BODY)
                                    .color(theme::text(dark)),
                            );
                            self.remember(
                                format!("media.favorites.family.{family}"),
                                &response,
                                None,
                            );
                            for item in items {
                                let response = ui.add_enabled(
                                    item.apply.is_some(),
                                    egui::Button::new(
                                        egui::RichText::new(&item.name)
                                            .size(theme::type_size::BODY)
                                            .color(theme::text(dark)),
                                    )
                                    .fill(theme::hover(dark))
                                    .stroke(egui::Stroke::NONE)
                                    .truncate()
                                    .min_size(Vec2::new(
                                        ui.available_width(),
                                        theme::size::CONTROL,
                                    )),
                                );
                                self.remember(
                                    format!("media.favorites.{family}.{}", item.key),
                                    &response,
                                    None,
                                );
                                if response.clicked()
                                    && let Some(apply) = item.apply
                                {
                                    self.actions.push(match apply {
                                        Apply::Effect(kind) => Action::AddEffect(kind),
                                        Apply::Filter(id) => Action::AddFilter(id),
                                        Apply::Transition(kind) => Action::AddTransition(kind),
                                        Apply::Sticker(id) => Action::InsertSticker(id),
                                        Apply::Sound(id) => Action::SoundsAdd(id),
                                    });
                                }
                            }
                            ui.add_space(theme::space::MD);
                        }
                    });
            },
        );
    }

    /// The filter at the top right: one catalogue family, or all of them.
    fn favorites_filter(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = theme::size::CONTROL;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
        let menu = egui::ComboBox::from_id_salt(Id::unique("media.favorites.filter"))
            .selected_text("")
            .width(32.0)
            .show_ui(ui, |ui| {
                let all = ui.selectable_label(
                    self.state.media_favorites_family.is_none(),
                    self.tr(Text::AssetsAll),
                );
                self.remember("media.favorites.filter.all", &all, None);
                if all.clicked() {
                    self.state.media_favorites_family = None;
                    ui.close();
                }
                for (family, label) in FAMILIES {
                    let response = ui.selectable_label(
                        self.state.media_favorites_family == Some(family),
                        self.tr(label),
                    );
                    self.remember(format!("media.favorites.filter.{family}"), &response, None);
                    if response.clicked() {
                        self.state.media_favorites_family = Some(family);
                        ui.close();
                    }
                }
            });
        let response = menu
            .response
            .on_hover_text(self.tr(Text::MediaFavoritesFilter));
        icons::tinted(
            Icon::Funnel,
            theme::icon::DENSE,
            theme::text(self.cx.preferences.dark),
        )
        .paint_at(
            ui,
            Rect::from_center_size(
                egui::pos2(response.rect.left() + 10.0, response.rect.center().y),
                Vec2::splat(theme::icon::DENSE),
            ),
        );
        self.remember("media.favorites.filter", &response, Some(Icon::Funnel));
    }

    /// The empty state: the star, one line, and the link to where stars are made. The link
    /// goes to the Effects tab, the nearest catalogue (the reference's Library is online).
    fn favorites_empty(&mut self, ui: &mut egui::Ui, body: Rect) {
        let link = Some((Text::MediaFavoritesLink, "media.favorites.link"));
        if self.empty_page(ui, body, Icon::Star, Text::MediaFavoritesEmpty, link) {
            self.actions.push(Action::SelectTab(AssetsTab::Effects));
        }
    }
}
