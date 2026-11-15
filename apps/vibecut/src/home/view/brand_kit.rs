//! Draws the Home brand kit page with one section of tiles per kind.
//!
//! Geometry is the measured reference page (capture 2026-11-14b, frame 111111, logical px from
//! the page's left edge): a 712×100 intro, then sections whose label's top sits 11 px under the
//! block above, with 120×80 tiles 33 px under the label's top and the next label 33 px under
//! the tiles.

use super::{
    CARD_RADIUS,
    widgets::{action_button, line, menu_action, quiet_buttons, semibold, set_author_id},
};
use crate::{
    egui,
    home::{
        brand_kit::{Page, Rename, Section},
        state::{Action, State},
    },
    i18n::{self, Locale, Text},
    kit::{self, Icon},
};
use engine::control::BrandEntryState;

const INTRO: [f32; 2] = [712.0, 100.0];
/// Intro title 13 px down and 24 px in; three-line list 36 px down on an 18 px pitch, its mark
/// centred 26 px in and its text 36 px in; the close glyph centred 16 px from the right and top.
const INTRO_TITLE: [f32; 2] = [24.0, 13.0];
const INTRO_LINE_TOP: f32 = 36.0;
const INTRO_LINE_PITCH: f32 = 18.0;
const INTRO_MARK_X: f32 = 26.0;
const INTRO_TEXT_X: f32 = 36.0;
const INTRO_CLOSE: f32 = 16.0;
const INTRO_MARK: f32 = 10.0;
/// The page title row above the intro, and the 13 px under it.
const TITLE_GAP: f32 = 13.0;
/// A section label's top sits 11 px under the block above; tiles 33 px under the label's top,
/// and the next label's top 33 px under the tiles.
const LABEL_GAP: f32 = 11.0;
const TILE_GAP_Y: f32 = 33.0;
const TILE: [f32; 2] = [120.0, 80.0];
/// The add tile's section glyph (18.5 px measured) with its plus beside it.
const ADD_GLYPH: f32 = 20.0;

/// What a click on the page asked for, applied after drawing.
enum Click {
    Pick(engine::control::BrandMediaSection),
    Fonts,
    Rename(u64, String),
    RemoveMedia(u64),
    RemoveFont(String),
    Collapse(Section),
    CloseIntro,
}

/// The page body: drawn inside Home's scroll area at the page's left edge.
pub(super) fn page(ui: &mut egui::Ui, width: f32, state: &mut State, locale: Locale) {
    let dark = ui.visuals().dark_mode;
    let tr = |key: Text| i18n::text(locale, key);
    let mut clicks = Vec::new();
    let (title_rect, _) =
        ui.allocate_exact_size(egui::vec2(width, kit::size::CONTROL), egui::Sense::hover());
    let title = line(
        ui,
        tr(Text::BrandKit),
        semibold(kit::type_size::BODY),
        kit::text(dark),
        width,
    );
    ui.painter().galley(
        egui::pos2(
            title_rect.min.x,
            title_rect.center().y - title.size().y / 2.0,
        ),
        title,
        kit::text(dark),
    );
    ui.add_space(TITLE_GAP);
    if !state.brand.intro_closed {
        intro(ui, locale, &mut clicks);
    }
    if state.brand.unavailable {
        notice(ui, tr(Text::BrandKitUnavailable), kit::secondary(dark));
    } else if state.brand.error.is_some() {
        notice(ui, tr(Text::BrandKitFailed), kit::error_text(dark));
    }
    if !state.brand.unavailable {
        for section in Section::ALL {
            section_block(ui, width, section, &state.brand, locale, &mut clicks);
        }
    }
    for click in clicks {
        match click {
            Click::Pick(section) => state.action = Some(Action::PickBrandMedia(section)),
            Click::Fonts => state.brand.open_fonts(),
            Click::Rename(media, name) => state.brand.rename = Some(Rename { media, name }),
            Click::RemoveMedia(media) => state.brand.remove_media(media),
            Click::RemoveFont(family) => state.brand.remove_font(family),
            Click::Collapse(section) => {
                if !state.brand.collapsed.remove(&section) {
                    state.brand.collapsed.insert(section);
                }
            }
            Click::CloseIntro => state.brand.intro_closed = true,
        }
    }
}

/// The intro banner: title, two lines and the close glyph. The reference's third line and its
/// illustration advertise a paid trial and are not built.
fn intro(ui: &mut egui::Ui, locale: Locale, clicks: &mut Vec<Click>) {
    let dark = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(INTRO.into(), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, CARD_RADIUS, kit::panel(dark));
    let title = line(
        ui,
        i18n::text(locale, Text::BrandKitIntroTitle),
        semibold(kit::type_size::BODY),
        kit::text(dark),
        rect.width() - INTRO_TITLE[0] * 2.0,
    );
    ui.painter().galley(
        rect.min + egui::vec2(INTRO_TITLE[0], INTRO_TITLE[1]),
        title,
        kit::text(dark),
    );
    for (index, key) in [Text::BrandKitIntroManage, Text::BrandKitIntroUse]
        .into_iter()
        .enumerate()
    {
        let top = rect.min.y + INTRO_LINE_TOP + INTRO_LINE_PITCH * index as f32;
        let galley = line(
            ui,
            i18n::text(locale, key),
            egui::FontId::proportional(kit::type_size::SMALL_LABEL),
            kit::text(dark),
            rect.width() - INTRO_TEXT_X - INTRO_TITLE[0],
        );
        let center = top + galley.size().y / 2.0;
        kit::tinted(Icon::Sparkle, INTRO_MARK, kit::text(dark)).paint_at(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(rect.min.x + INTRO_MARK_X, center),
                egui::Vec2::splat(INTRO_MARK),
            ),
        );
        ui.painter().galley(
            egui::pos2(rect.min.x + INTRO_TEXT_X, top),
            galley,
            kit::text(dark),
        );
    }
    let close = egui::Rect::from_center_size(
        egui::pos2(rect.max.x - INTRO_CLOSE, rect.min.y + INTRO_CLOSE),
        egui::Vec2::splat(kit::size::CONTROL),
    );
    // A child ui, not a scope: a scope would move the page's cursor back up to the glyph.
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(close));
    quiet_buttons(ui);
    let response = kit::icon_button(
        ui,
        "brand_kit.intro.close",
        Icon::X,
        i18n::text(locale, Text::BrandKitIntroClose),
        false,
    );
    if response.clicked() {
        clicks.push(Click::CloseIntro);
    }
}

/// One quiet line under the intro: the store did not open, or the last edit failed.
fn notice(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.add_space(LABEL_GAP);
    ui.label(
        egui::RichText::new(text)
            .size(kit::type_size::BODY)
            .color(color),
    );
}

fn section_label(section: Section) -> Text {
    match section {
        Section::Videos => Text::BrandKitVideos,
        Section::Images => Text::BrandKitImages,
        Section::Stickers => Text::Stickers,
        Section::Fonts => Text::BrandKitFonts,
        Section::Backgrounds => Text::BrandKitBackgrounds,
        Section::Music => Text::Music,
    }
}

/// The section's glyph: the catalogue's existing icons, ours (the reference draws its own).
fn section_icon(section: Section) -> Icon {
    match section {
        Section::Videos => Icon::FilmStrip,
        Section::Images => Icon::File,
        Section::Stickers => Icon::Sticker,
        Section::Fonts => Icon::TextT,
        Section::Backgrounds => Icon::Layout,
        Section::Music => Icon::MusicNotes,
    }
}

/// The label with its collapse caret, then the section's tiles and its add tile.
fn section_block(
    ui: &mut egui::Ui,
    width: f32,
    section: Section,
    page: &Page,
    locale: Locale,
    clicks: &mut Vec<Click>,
) {
    let dark = ui.visuals().dark_mode;
    let collapsed = page.collapsed.contains(&section);
    let busy = page.busy() || !page.loaded;
    let view = page.kit.as_ref();
    ui.add_space(LABEL_GAP);
    let label = i18n::text(locale, section_label(section));
    let galley = line(
        ui,
        label,
        egui::FontId::proportional(kit::type_size::SMALL_LABEL),
        kit::text(dark),
        width,
    );
    let caret = kit::icon::PLAY;
    let size = egui::vec2(
        galley.size().x + kit::space::XS + caret,
        galley.size().y.max(caret),
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let id = format!("brand_kit.{}.collapse", section.id());
    set_author_id(&response, &id);
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, !collapsed, label));
    ui.painter().galley(rect.min, galley, kit::text(dark));
    kit::tinted(
        if collapsed {
            Icon::CaretRight
        } else {
            Icon::CaretDown
        },
        caret,
        kit::secondary(dark),
    )
    .paint_at(
        ui,
        egui::Rect::from_min_size(
            egui::pos2(rect.max.x - caret, rect.center().y - caret / 2.0),
            egui::Vec2::splat(caret),
        ),
    );
    if response.clicked() {
        clicks.push(Click::Collapse(section));
    }
    if collapsed {
        return;
    }
    ui.add_space(TILE_GAP_Y - rect.height());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing = egui::Vec2::splat(kit::space::LG);
        match section.media() {
            Some(media_section) => {
                for media in view
                    .iter()
                    .flat_map(|view| view.media.iter())
                    .filter(|media| media.section == media_section)
                {
                    let state = match media.state {
                        BrandEntryState::Available => None,
                        BrandEntryState::Missing => Some(Text::BrandKitEntryMissing),
                        BrandEntryState::Corrupt => Some(Text::BrandKitEntryCorrupt),
                    };
                    let prefix = format!("brand_kit.{}.{}", section.id(), media.id);
                    item_tile(
                        ui,
                        &prefix,
                        Some(section_icon(section)),
                        &media.name,
                        state,
                        locale,
                        |ui| {
                            if menu_action(
                                ui,
                                &format!("{prefix}.rename"),
                                Icon::PencilSimple,
                                i18n::text(locale, Text::MediaRename),
                            ) {
                                clicks.push(Click::Rename(media.id, media.name.clone()));
                                ui.close();
                            }
                            if menu_action(
                                ui,
                                &format!("{prefix}.remove"),
                                Icon::Trash,
                                i18n::text(locale, Text::BrandKitRemove),
                            ) {
                                clicks.push(Click::RemoveMedia(media.id));
                                ui.close();
                            }
                        },
                    );
                }
                if add_tile(ui, section, busy, locale) {
                    clicks.push(Click::Pick(media_section));
                }
            }
            None => {
                for (index, family) in view.iter().flat_map(|view| view.fonts.iter()).enumerate() {
                    let prefix = format!("brand_kit.fonts.{index}");
                    item_tile(ui, &prefix, None, family, None, locale, |ui| {
                        if menu_action(
                            ui,
                            &format!("{prefix}.remove"),
                            Icon::Trash,
                            i18n::text(locale, Text::BrandKitRemove),
                        ) {
                            clicks.push(Click::RemoveFont(family.clone()));
                            ui.close();
                        }
                    });
                }
                if add_tile(ui, section, busy, locale) {
                    clicks.push(Click::Fonts);
                }
            }
        }
    });
    ui.add_space(TILE_GAP_Y - LABEL_GAP);
}

/// The "+" tile: the section glyph and a plus, centred. A click opens the section's picker.
fn add_tile(ui: &mut egui::Ui, section: Section, busy: bool, locale: Locale) -> bool {
    let dark = ui.visuals().dark_mode;
    let id = format!("brand_kit.{}.add", section.id());
    let (rect, response) = ui.allocate_exact_size(TILE.into(), egui::Sense::click());
    set_author_id(&response, &id);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            !busy,
            i18n::text(locale, Text::BrandKitAdd),
        )
    });
    let fill = if response.hovered() && !busy {
        kit::hover(dark)
    } else {
        kit::panel(dark)
    };
    ui.painter().rect_filled(rect, CARD_RADIUS, fill);
    let plus = kit::icon::PLAY;
    let width = ADD_GLYPH + kit::space::XS + plus;
    let left = rect.center().x - width / 2.0;
    let ink = kit::secondary(dark);
    kit::tinted(section_icon(section), ADD_GLYPH, ink).paint_at(
        ui,
        egui::Rect::from_min_size(
            egui::pos2(left, rect.center().y - ADD_GLYPH / 2.0),
            egui::Vec2::splat(ADD_GLYPH),
        ),
    );
    kit::tinted(Icon::Plus, plus, ink).paint_at(
        ui,
        egui::Rect::from_min_size(
            egui::pos2(
                left + ADD_GLYPH + kit::space::XS,
                rect.center().y - plus / 2.0,
            ),
            egui::Vec2::splat(plus),
        ),
    );
    let response = response.on_hover_text(i18n::text(locale, Text::BrandKitAdd));
    response.clicked() && !busy
}

/// One kit entry: its glyph (none for a font, whose name is the face), its name and, for a
/// file that fails verification, why; the menu sits at the top right. Ours, unmeasured: the
/// reference capture shows no filled section.
fn item_tile(
    ui: &mut egui::Ui,
    prefix: &str,
    icon: Option<Icon>,
    name: &str,
    state: Option<Text>,
    locale: Locale,
    menu: impl FnOnce(&mut egui::Ui),
) {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(TILE.into(), egui::Sense::hover());
    set_author_id(&response, prefix);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, name));
    ui.painter()
        .rect_filled(rect, CARD_RADIUS, kit::panel(dark));
    let inset = kit::space::MD;
    let name_galley = line(
        ui,
        name,
        egui::FontId::proportional(kit::type_size::SMALL_LABEL),
        kit::text(dark),
        rect.width() - inset * 2.0,
    );
    match icon {
        Some(icon) => {
            kit::tinted(icon, kit::icon::STATE, kit::secondary(dark)).paint_at(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, rect.min.y + inset + kit::icon::STATE / 2.0),
                    egui::Vec2::splat(kit::icon::STATE),
                ),
            );
            ui.painter().galley(
                egui::pos2(
                    rect.min.x + inset,
                    rect.max.y - inset - name_galley.size().y,
                ),
                name_galley,
                kit::text(dark),
            );
        }
        None => {
            ui.painter().galley(
                rect.center() - name_galley.size() / 2.0,
                name_galley,
                kit::text(dark),
            );
        }
    }
    if let Some(state) = state {
        let galley = line(
            ui,
            i18n::text(locale, state),
            egui::FontId::proportional(kit::type_size::SMALL_LABEL),
            kit::error_text(dark),
            rect.width() - inset * 2.0,
        );
        ui.painter().galley(
            egui::pos2(rect.min.x + inset, rect.min.y + inset),
            galley,
            kit::error_text(dark),
        );
    }
    let button = egui::Rect::from_min_size(
        egui::pos2(
            rect.max.x - kit::space::XS - kit::size::CONTROL,
            rect.min.y + kit::space::XS,
        ),
        egui::Vec2::splat(kit::size::CONTROL),
    );
    // A child ui, not a scope: a scope would grow the row and shift the next tile down.
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(button));
    quiet_buttons(ui);
    let response = ui
        .menu_image_button(
            kit::tinted(Icon::DotsThree, kit::icon::DENSE, kit::text(dark)),
            menu,
        )
        .response
        .on_hover_text(i18n::text(locale, Text::BrandKitItemMenu));
    set_author_id(&response, &format!("{prefix}.menu"));
}

/// The rename dialog and the Fonts chooser, over the page.
pub(super) fn dialogs(state: &mut State, ctx: &egui::Context, locale: Locale) {
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if escape {
        state.brand.rename = None;
        state.brand.fonts = None;
    }
    let mut renamed = None;
    if let Some(rename) = &mut state.brand.rename {
        let mut close = false;
        egui::Modal::new(egui::Id::unique("brand_kit.rename.dialog"))
            .frame(kit::dialog_frame(ctx))
            .show(ctx, |ui| {
                kit::dialog_title(ui, i18n::text(locale, Text::MediaRename));
                let response =
                    ui.add(egui::TextEdit::singleline(&mut rename.name).desired_width(320.0));
                set_author_id(&response, "brand_kit.rename.input");
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        "brand_kit.rename.save",
                        Icon::FloppyDisk,
                        i18n::text(locale, Text::MediaRename),
                        !rename.name.trim().is_empty(),
                    ) || (enter && !rename.name.trim().is_empty())
                    {
                        renamed = Some((rename.media, rename.name.trim().to_owned()));
                    }
                    if action_button(
                        ui,
                        "brand_kit.rename.cancel",
                        Icon::X,
                        i18n::text(locale, Text::Cancel),
                        true,
                    ) {
                        close = true;
                    }
                });
            });
        if close {
            state.brand.rename = None;
        }
    }
    if let Some((media, name)) = renamed {
        state.brand.rename = None;
        state.brand.rename_media(media, name);
    }
    let mut chosen = None;
    if let Some(chooser) = &mut state.brand.fonts {
        let mut close = false;
        egui::Modal::new(egui::Id::unique("brand_kit.fonts.dialog"))
            .frame(kit::dialog_frame(ctx))
            .show(ctx, |ui| {
                kit::dialog_title(ui, i18n::text(locale, Text::BrandKitFonts));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut chooser.query)
                        .hint_text(i18n::text(locale, Text::BrandKitFontsSearch))
                        .desired_width(320.0),
                );
                set_author_id(&response, "brand_kit.fonts.search");
                match &chooser.families {
                    None => {
                        ui.label(i18n::text(locale, Text::Loading));
                    }
                    Some(Err(_)) => {
                        ui.label(
                            egui::RichText::new(i18n::text(locale, Text::BrandKitFailed))
                                .color(kit::error_text(ui.visuals().dark_mode)),
                        );
                    }
                    Some(Ok(families)) => {
                        let query = chooser.query.trim().to_lowercase();
                        let shown: Vec<&String> = families
                            .iter()
                            .filter(|family| {
                                query.is_empty() || family.to_lowercase().contains(&query)
                            })
                            .collect();
                        if shown.is_empty() {
                            ui.label(i18n::text(locale, Text::AssetsNoMatches));
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("brand_kit.fonts.list")
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for family in shown {
                                    let response = ui.selectable_label(false, family.as_str());
                                    set_author_id(
                                        &response,
                                        &format!("brand_kit.fonts.family.{family}"),
                                    );
                                    if response.clicked() {
                                        chosen = Some(family.clone());
                                    }
                                }
                            });
                    }
                }
                if action_button(
                    ui,
                    "brand_kit.fonts.cancel",
                    Icon::X,
                    i18n::text(locale, Text::Cancel),
                    true,
                ) {
                    close = true;
                }
            });
        if close {
            state.brand.fonts = None;
        }
    }
    if let Some(family) = chosen {
        state.brand.fonts = None;
        state
            .brand
            .add_font(family, i18n::text(locale, Text::BrandKit).to_owned());
    }
}
