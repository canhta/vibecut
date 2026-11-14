//! Presents the About dialog: app icon, version, copyright, terms links and the licence list.
use crate::editor::Editor;
use crate::egui;
use crate::i18n::{self, Locale, Text};
use crate::kit::{self, Icon, radius, size, space, type_size};
use crate::services::about;

/// Which page of the About dialog is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Page {
    Main,
    Licences,
}

/// Controls painted this frame: automation id, icon name and response.
type Painted = Vec<(&'static str, Option<&'static str>, egui::Response)>;

/// The app tile the About dialog shows, decoded once per context.
const ICON_PNG: &[u8] = include_bytes!("../../../../../assets/app-icon/vibecut-1024.png");

impl Editor {
    pub(in crate::editor) fn show_about_dialog(&mut self, ctx: &egui::Context) {
        let Some(page) = self.frame.about else {
            return;
        };
        let locale = self.preferences.locale;
        let dark = self.preferences.dark;
        let texture = app_icon(ctx);
        let mut painted: Painted = Vec::new();
        egui::Modal::new(egui::Id::new("about.dialog"))
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
        for (id, icon, response) in painted {
            self.remember_with_icon(id, icon, &response);
            if response.clicked() {
                self.action(id);
            }
        }
    }
}

/// The 36 px title bar: the dialog title centred, Close at the right and, on the licence
/// page, a way back to the main page at the left.
fn title_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    page: Page,
    locale: Locale,
    dark: bool,
    painted: &mut Painted,
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
    painted: &mut Painted,
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
    let version = i18n::format(locale, Text::AboutVersion, &[("version", about::VERSION)]);
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
    painted: &mut Painted,
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

/// The third-party licence list: one row per author and licence, scrolling inside the dialog.
fn licence_page(ui: &mut egui::Ui, rect: egui::Rect, dark: bool) {
    let body = egui::Rect::from_min_max(
        egui::pos2(
            rect.left() + size::ABOUT_INSET,
            rect.top() + size::ABOUT_TITLE_BAR + space::LG,
        ),
        egui::pos2(rect.right() - size::ABOUT_INSET, rect.bottom() - space::LG),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("about.licences")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for notice in about::notices() {
                    ui.label(
                        egui::RichText::new(notice.author)
                            .font(kit::label_font())
                            .color(kit::text(dark)),
                    );
                    ui.label(
                        egui::RichText::new(notice.licence)
                            .size(type_size::BODY)
                            .color(kit::secondary(dark)),
                    );
                    ui.label(
                        egui::RichText::new(notice.source)
                            .size(type_size::SMALL_LABEL)
                            .color(kit::secondary(dark)),
                    );
                    ui.add_space(space::LG);
                }
            });
    });
}

/// The app tile as a texture, decoded on first use and kept in the context.
fn app_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let key = egui::Id::new("about.icon");
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
