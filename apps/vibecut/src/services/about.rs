//! The About dialog: its facts (version, terms links, licence list) and its one shared view.
use crate::egui;
use crate::i18n::{self, Locale, Text};
use crate::kit::{self, Icon, radius, size, space, type_size};
use std::sync::OnceLock;

/// The release version, from the workspace package.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Terms of use: the project licence text.
pub(crate) const TERMS_URL: &str = "https://github.com/canhta/vibecut/blob/main/LICENSE";
/// Privacy: the open-source policy's privacy section.
pub(crate) const PRIVACY_URL: &str =
    "https://github.com/canhta/vibecut/blob/main/docs/engineering/open-source.md#privacy";

/// One bundled asset licence row, generated at build time from the `.attribution` sidecars.
#[derive(Clone, Copy)]
struct Notice {
    author: &'static str,
    licence: &'static str,
    source: &'static str,
}

/// Every bundled asset's author and licence, one row per pair.
const ASSETS: &[Notice] = include!(concat!(env!("OUT_DIR"), "/licences.rs"));

/// Every shipped Rust crate: `name<TAB>version<TAB>licence` per line, from `cargo xtask licences`.
const CRATES: &str = include_str!("../../third-party-crates.txt");

/// One row of the licence list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) licence: &'static str,
    pub(crate) source: Option<&'static str>,
}

/// Every third-party entry: FFmpeg, then the bundled assets by author, then the Rust crates.
/// FFmpeg is linked dynamically; the open-source policy names it with its licence.
pub(crate) fn entries() -> &'static [Entry] {
    static LIST: OnceLock<Vec<Entry>> = OnceLock::new();
    LIST.get_or_init(|| {
        let ffmpeg = Entry {
            name: "FFmpeg developers".to_owned(),
            licence: "LGPL-2.1-or-later (GPL-3.0 with libx264 on Linux)",
            source: Some("https://ffmpeg.org"),
        };
        let assets = ASSETS.iter().map(|notice| Entry {
            name: notice.author.to_owned(),
            licence: notice.licence,
            source: Some(notice.source),
        });
        let crates = CRATES.lines().filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (name, version, licence) = (parts.next()?, parts.next()?, parts.next()?);
            Some(Entry {
                name: format!("{name} {version}"),
                licence,
                source: None,
            })
        });
        std::iter::once(ffmpeg)
            .chain(assets)
            .chain(crates)
            .collect()
    })
}

/// Which page of the dialog is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Main,
    Licences,
}

/// A control painted this frame: automation id, icon name and response.
pub(crate) type Click = (&'static str, Option<&'static str>, egui::Response);

/// The app tile the dialog shows, decoded once per context.
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/app-icon/vibecut-1024.png");

/// Paint the dialog and return the controls it drew; the host routes their clicks.
pub(crate) fn show(ctx: &egui::Context, page: Page, locale: Locale, dark: bool) -> Vec<Click> {
    let texture = app_icon(ctx);
    let mut painted: Vec<Click> = Vec::new();
    egui::Modal::new(egui::Id::unique("about.dialog"))
        .frame(kit::dialog_frame(ctx).inner_margin(egui::Margin::ZERO))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(size::ABOUT_WIDTH, size::ABOUT_HEIGHT),
                egui::Sense::hover(),
            );
            title_bar(ui, rect, page, locale, dark, &mut painted);
            match page {
                Page::Main => main_page(ui, rect, texture.as_ref(), locale, dark, &mut painted),
                Page::Licences => licence_page(ui, rect, dark),
            }
        });
    painted
}

/// The 36 px title bar: the dialog title centred, Close at the right and, on the licence
/// page, a way back to the main page at the left.
fn title_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    page: Page,
    locale: Locale,
    dark: bool,
    painted: &mut Vec<Click>,
) {
    let bar = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), size::ABOUT_TITLE_BAR));
    ui.painter().rect_filled(
        bar,
        egui::CornerRadius {
            nw: radius::DIALOG,
            ne: radius::DIALOG,
            sw: 0,
            se: 0,
        },
        kit::window(dark),
    );
    ui.painter().hline(
        bar.x_range(),
        bar.bottom(),
        egui::Stroke::new(space::HAIRLINE, kit::separator(dark)),
    );
    let title = match page {
        Page::Main => Text::ActionAppAbout,
        Page::Licences => Text::ActionAppLicences,
    };
    centred(
        ui,
        egui::Rect::from_center_size(
            bar.center(),
            egui::vec2(
                bar.width() - 2.0 * (space::LG + size::CONTROL),
                size::CONTROL,
            ),
        ),
        i18n::text(locale, title),
        kit::label_font(),
        kit::text(dark),
    );
    let close = ui
        .put(
            egui::Rect::from_center_size(
                egui::pos2(
                    bar.right() - space::LG - size::CONTROL / 2.0,
                    bar.center().y,
                ),
                egui::Vec2::splat(size::CONTROL),
            ),
            egui::Button::image(kit::tinted(Icon::X, 16.0, kit::secondary(dark))).frame(false),
        )
        .on_hover_text(i18n::text(locale, Text::ActionAppAboutClose));
    painted.push(("app.about.close", Some(Icon::X.name()), close));
    if page == Page::Licences {
        let back = ui
            .put(
                egui::Rect::from_center_size(
                    egui::pos2(bar.left() + space::LG + size::CONTROL / 2.0, bar.center().y),
                    egui::Vec2::splat(size::CONTROL),
                ),
                egui::Button::image(kit::tinted(Icon::ArrowLeft, 16.0, kit::secondary(dark)))
                    .frame(false),
            )
            .on_hover_text(i18n::text(locale, Text::ActionAppAbout));
        painted.push(("app.about", Some(Icon::ArrowLeft.name()), back));
    }
}

/// Icon with the version pill at its top right, the name, a divider, the links and the
/// copyright line.
fn main_page(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    texture: Option<&egui::TextureHandle>,
    locale: Locale,
    dark: bool,
    painted: &mut Vec<Click>,
) {
    let top = rect.top() + size::ABOUT_TITLE_BAR;
    let centre = rect.center().x;
    let icon = egui::Rect::from_min_size(
        egui::pos2(centre - size::ABOUT_ICON / 2.0, top + size::ABOUT_ICON_TOP),
        egui::Vec2::splat(size::ABOUT_ICON),
    );
    if let Some(texture) = texture {
        ui.put(
            icon,
            egui::Image::new((texture.id(), icon.size())).corner_radius(radius::ABOUT_ICON),
        );
    }
    let version = i18n::format(locale, Text::AboutVersion, &[("version", VERSION)]);
    let pill_font = egui::FontId::proportional(type_size::BODY);
    let text_width = ui
        .painter()
        .layout_no_wrap(version.clone(), pill_font.clone(), kit::on_accent(dark))
        .size()
        .x;
    let pill = egui::Rect::from_min_size(
        icon.right_top(),
        egui::vec2(
            text_width + 2.0 * space::ICON_LABEL,
            size::ABOUT_BADGE_HEIGHT,
        ),
    );
    ui.painter().rect_filled(
        pill,
        egui::CornerRadius::same((size::ABOUT_BADGE_HEIGHT / 2.0) as u8),
        kit::accent(dark),
    );
    centred(ui, pill, &version, pill_font, kit::on_accent(dark));
    centred(
        ui,
        egui::Rect::from_center_size(
            egui::pos2(centre, icon.bottom() + size::ABOUT_NAME_CENTRE),
            egui::vec2(rect.width() - 2.0 * size::ABOUT_INSET, size::CONTROL),
        ),
        i18n::text(locale, Text::AppName),
        egui::FontId::new(type_size::ABOUT_NAME, kit::label_font().family),
        kit::text(dark),
    );
    let divider = top + size::ABOUT_DIVIDER_TOP;
    ui.painter().hline(
        egui::Rangef::new(
            rect.left() + size::ABOUT_INSET,
            rect.right() - size::ABOUT_INSET,
        ),
        divider,
        egui::Stroke::new(space::HAIRLINE, kit::separator(dark)),
    );
    let copyright = rect.bottom() - size::ABOUT_COPYRIGHT_BOTTOM;
    links(
        ui,
        centre,
        (divider + copyright) / 2.0,
        locale,
        dark,
        painted,
    );
    centred(
        ui,
        egui::Rect::from_center_size(
            egui::pos2(centre, copyright),
            egui::vec2(rect.width() - 2.0 * size::ABOUT_INSET, size::CONTROL),
        ),
        i18n::text(locale, Text::AboutCopyright),
        egui::FontId::proportional(type_size::BODY),
        kit::secondary(dark),
    );
}

/// A non-selectable label centred in `rect`.
fn centred(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
) {
    ui.put(
        rect,
        egui::Label::new(egui::RichText::new(text).font(font).color(color)).selectable(false),
    );
}

/// The Terms, Privacy and Licences links, centred on one row.
fn links(
    ui: &mut egui::Ui,
    centre: f32,
    middle: f32,
    locale: Locale,
    dark: bool,
    painted: &mut Vec<Click>,
) {
    let font = egui::FontId::proportional(type_size::BODY);
    let pad = space::XS;
    let gap = space::MD;
    let items = [
        ("app.terms", Text::ActionAppTerms),
        ("app.privacy", Text::ActionAppPrivacy),
        ("app.licences", Text::AboutLicencesLink),
    ];
    let widths: Vec<f32> = items
        .iter()
        .map(|(_, text)| {
            ui.painter()
                .layout_no_wrap(
                    i18n::text(locale, *text).to_owned(),
                    font.clone(),
                    kit::text(dark),
                )
                .size()
                .x
                + 2.0 * pad
        })
        .collect();
    let total = widths.iter().sum::<f32>() + gap * items.len().saturating_sub(1) as f32;
    let mut x = centre - total / 2.0;
    for ((id, text), width) in items.into_iter().zip(widths) {
        let response = ui.put(
            egui::Rect::from_min_size(
                egui::pos2(x, middle - size::CONTROL / 2.0),
                egui::vec2(width, size::CONTROL),
            ),
            egui::Button::new(
                egui::RichText::new(i18n::text(locale, text))
                    .font(font.clone())
                    .color(kit::text(dark)),
            )
            .frame(false),
        );
        painted.push((id, None, response));
        x += width + gap;
    }
}

/// The third-party list: one truncated line per entry, scrolling inside the dialog; the hover
/// text holds the full line and the source.
fn licence_page(ui: &mut egui::Ui, rect: egui::Rect, dark: bool) {
    let body = egui::Rect::from_min_max(
        egui::pos2(
            rect.left() + size::ABOUT_INSET,
            rect.top() + size::ABOUT_TITLE_BAR + space::LG,
        ),
        egui::pos2(rect.right() - size::ABOUT_INSET, rect.bottom() - space::LG),
    );
    let list = entries();
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("about.licences")
            .auto_shrink([false, false])
            .show_rows(ui, size::CONTROL, list.len(), |ui, range| {
                for entry in list.get(range).unwrap_or_default() {
                    let line = format!("{} · {}", entry.name, entry.licence);
                    let shown = ui
                        .allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), size::CONTROL),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&line)
                                            .size(type_size::BODY)
                                            .color(kit::text(dark)),
                                    )
                                    .truncate()
                                    .selectable(false),
                                )
                            },
                        )
                        .inner;
                    match entry.source {
                        Some(source) => shown.on_hover_text(format!("{line}\n{source}")),
                        None => shown.on_hover_text(&line),
                    };
                }
            });
    });
}

/// The app tile as a texture, decoded on first use and kept in the context.
fn app_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let key = egui::Id::unique("about.icon");
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(key)) {
        return Some(texture);
    }
    let image = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
    let pixels = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let texture = ctx.load_texture("about.icon", pixels, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(key, texture.clone()));
    Some(texture)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_names_ffmpeg_the_asset_licences_and_every_shipped_crate_licence() {
        let list = entries();
        assert_eq!(
            list.first().map(|e| e.name.as_str()),
            Some("FFmpeg developers")
        );
        for licence in ["OFL-1.1", "MIT"] {
            assert!(
                list.iter().any(|entry| entry.licence == licence),
                "{licence}"
            );
        }
        let egui = list.iter().find(|entry| entry.name.starts_with("egui "));
        assert_eq!(egui.map(|entry| entry.licence), Some("MIT OR Apache-2.0"));
        assert!(
            list.iter()
                .all(|entry| !entry.name.is_empty() && !entry.licence.is_empty())
        );
    }
}
