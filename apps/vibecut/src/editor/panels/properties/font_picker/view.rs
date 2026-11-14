//! Draws the font picker popover and turns its events into picker actions.
use super::state::{Download, Filter, State, Status};
use crate::egui::{self, Align, Align2, Color32, Id, Rect, Sense, Stroke, Vec2};
use crate::i18n::{self, Locale, Text};
use crate::kit as theme;
use crate::preferences::FontPickerPreferences;
use engine::control::{FontEntry, FontState};
use std::sync::Arc;

/// Measured from the reference dropdown (logical px, frame 103657 of the 2026-11-14b round):
/// popover 385 wide opening 8 below the field, search field 26 high, filter chips 24 high,
/// rows on a 28 pitch with the download and star icons in 28 px cells at the right edge, the
/// list running to about 448 under the chips. The section header height is read from the
/// Presets label's 24 px band and is [proposed]; the 10 px inset is the search field's.
const WIDTH: f32 = 385.0;
const LIST_HEIGHT: f32 = 448.0;
const SEARCH_HEIGHT: f32 = 26.0;
const CHIP_HEIGHT: f32 = 24.0;
const ROW_HEIGHT: f32 = 28.0;
const SECTION_HEIGHT: f32 = 24.0;
const CHECK_WIDTH: f32 = 20.0;
const ICON_CELL: f32 = 28.0;
const STATE_WIDTH: f32 = 96.0;
const ACTION_WIDTH: f32 = 84.0;

/// What the picker asks the properties owner to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Close,
    /// Ask the backend for the catalog through `fonts.list`.
    List,
    /// Toggle one family in the persisted favourites.
    Favorite {
        family: String,
        favorite: bool,
    },
    /// Load and apply an available family through `fonts.load`.
    Select(String),
    /// Start one explicit catalog download through `fonts.download`.
    Download(String),
    Import,
    CancelImport,
    Cancel,
}

/// Widgets for the frame's automation map and requests to route.
pub struct Event {
    pub widgets: Vec<(String, egui::Response)>,
    pub actions: Vec<Action>,
}

/// Draw the open picker below its field anchor, clamped to the window.
pub fn show(
    state: &mut State,
    ctx: &egui::Context,
    locale: Locale,
    dark: bool,
    preferences: &FontPickerPreferences,
    anchor: egui::Pos2,
) -> Event {
    let mut event = Event {
        widgets: Vec::new(),
        actions: Vec::new(),
    };
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        event.actions.push(Action::Close);
        return event;
    }
    install_previews(state, ctx);
    let tr = |key| i18n::text(locale, key);
    let window = egui::Window::new(tr(Text::TextFont))
        .id(Id::unique("fonts.picker"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(anchor)
        .constrain_to(ctx.content_rect().shrink(8.0));
    // The click that opened the picker lands outside it on its first pass.
    let fresh = !std::mem::replace(&mut state.shown, true);
    let shown = window.show(ctx, |ui| {
        ui.set_width(WIDTH);
        let search = ui.add_sized(
            [ui.available_width(), SEARCH_HEIGHT],
            egui::TextEdit::singleline(&mut state.query)
                .id(Id::unique("fonts.search"))
                .hint_text(tr(Text::FontsSearch)),
        );
        record(&mut event.widgets, "fonts.search", search);
        ui.add_space(theme::space::XS);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::space::XS;
            for filter in Filter::ALL {
                let id = format!("fonts.filter.{}", filter.id());
                let selected = state.filter == filter;
                let button = egui::Button::new(
                    egui::RichText::new(tr(filter_label(filter))).color(chip_ink(selected, dark)),
                )
                .fill(chip_fill(selected, dark))
                .stroke(Stroke::new(theme::space::HAIRLINE, theme::border(dark)))
                .min_size(Vec2::new(0.0, CHIP_HEIGHT));
                let response = ui.push_id(&id, |ui| ui.add(button)).inner;
                record(&mut event.widgets, &id, response.clone());
                if response.clicked() {
                    state.filter = filter;
                }
            }
        });
        ui.add_space(theme::space::XS);
        match state.status {
            Status::Loading => {
                ui.add_space(theme::space::LG);
                let label = ui.label(
                    egui::RichText::new(tr(Text::FontsLoading)).color(theme::secondary(dark)),
                );
                record(&mut event.widgets, "fonts.loading", label);
                ui.add_space(theme::space::LG);
                if !state.request_sent {
                    state.request_sent = true;
                    event.actions.push(Action::List);
                }
            }
            Status::Failed => {
                ui.add_space(theme::space::LG);
                let label = ui.label(
                    egui::RichText::new(tr(Text::FontsLoadFailed)).color(theme::error_text(dark)),
                );
                record(&mut event.widgets, "fonts.error", label);
                ui.add_space(theme::space::XS);
                let retry = ui.button(tr(Text::Retry));
                record(&mut event.widgets, "fonts.retry", retry.clone());
                if retry.clicked() {
                    state.request_sent = true;
                    event.actions.push(Action::List);
                }
                ui.add_space(theme::space::LG);
            }
            Status::Ready => {
                let (presets, system): (Vec<FontEntry>, Vec<FontEntry>) = {
                    let (presets, system) = state.sections(preferences);
                    (
                        presets.into_iter().cloned().collect(),
                        system.into_iter().cloned().collect(),
                    )
                };
                if presets.is_empty() && system.is_empty() {
                    ui.add_space(theme::space::LG);
                    let label = ui.label(
                        egui::RichText::new(tr(Text::FontsNone)).color(theme::secondary(dark)),
                    );
                    record(&mut event.widgets, "fonts.empty", label);
                    ui.add_space(theme::space::LG);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("fonts.list")
                        .max_height(LIST_HEIGHT)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if !presets.is_empty() {
                                let toggled = section_header(
                                    ui,
                                    "fonts.section.presets",
                                    tr(Text::FontsPresets),
                                    Some(state.presets_collapsed),
                                    dark,
                                    &mut event,
                                );
                                if toggled {
                                    state.presets_collapsed = !state.presets_collapsed;
                                }
                                if !state.presets_collapsed {
                                    for entry in &presets {
                                        font_row(
                                            ui,
                                            entry,
                                            state,
                                            preferences,
                                            locale,
                                            dark,
                                            &mut event,
                                        );
                                    }
                                }
                            }
                            if !system.is_empty() {
                                section_header(
                                    ui,
                                    "fonts.section.system",
                                    tr(Text::FontsSystem),
                                    None,
                                    dark,
                                    &mut event,
                                );
                                for entry in &system {
                                    font_row(
                                        ui,
                                        entry,
                                        state,
                                        preferences,
                                        locale,
                                        dark,
                                        &mut event,
                                    );
                                }
                            }
                        });
                }
            }
        }
        ui.separator();
        let upload = ui.add_enabled(
            !state.importing,
            egui::Button::image_and_text(
                theme::tinted(
                    theme::Icon::UploadSimple,
                    theme::icon::DENSE,
                    theme::text(dark),
                ),
                tr(Text::FontUpload),
            )
            .min_size(Vec2::new(0.0, theme::size::FIELD)),
        );
        upload.clone().on_hover_text(tr(Text::FontUploadHint));
        if upload.clicked() {
            event.actions.push(Action::Import);
        }
        record(&mut event.widgets, "fonts.upload", upload);
        if state.importing {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
            ui.horizontal(|ui| {
                ui.spinner();
                let label = ui.label(tr(Text::FontImporting));
                record(&mut event.widgets, "fonts.importing", label);
                let cancel = theme::icon_button(
                    ui,
                    "fonts.import.cancel",
                    theme::Icon::X,
                    tr(Text::Cancel),
                    false,
                );
                record(&mut event.widgets, "fonts.import.cancel", cancel.clone());
                if cancel.clicked() {
                    event.actions.push(Action::CancelImport);
                }
            });
        }
        if state.import_failed {
            let label = ui.colored_label(theme::error_text(dark), tr(Text::FontImportFailed));
            record(&mut event.widgets, "fonts.import.failed", label);
        }
    });
    // A dropdown, as the reference: no title or close button; a click outside closes it.
    if !fresh && shown.is_some_and(|window| window.response.clicked_elsewhere()) {
        event.actions.push(Action::Close);
    }
    event
}

fn font_row(
    ui: &mut egui::Ui,
    entry: &FontEntry,
    state: &State,
    preferences: &FontPickerPreferences,
    locale: Locale,
    dark: bool,
    event: &mut Event,
) {
    let tr = |key| i18n::text(locale, key);
    let family = entry.family.as_str();
    let favorite = preferences.favorites.iter().any(|name| name == family);
    let selected = state.current.as_deref() == Some(family);
    let active = state
        .download
        .as_ref()
        .filter(|download| download.family == family);
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::hover());
    if selected {
        ui.painter()
            .rect_filled(row, theme::radius::CONTROL, theme::hover(dark));
        paint_check(ui, row, dark);
    }
    // Right to left, as the reference: star cell, then the download or action cell, then the
    // state text, with the name taking what is left after the check column.
    let star_rect = Rect::from_min_max(egui::pos2(row.right() - ICON_CELL, row.top()), row.max);
    let action_width = if active.is_some() {
        ACTION_WIDTH
    } else {
        ICON_CELL
    };
    let action_left = star_rect.left() - action_width;
    let state_left = action_left - STATE_WIDTH;
    let name_rect = Rect::from_min_max(
        egui::pos2(row.left() + CHECK_WIDTH, row.top()),
        egui::pos2(state_left.max(row.left() + CHECK_WIDTH), row.bottom()),
    );
    let state_rect = Rect::from_min_max(
        egui::pos2(state_left, row.top()),
        egui::pos2(action_left, row.bottom()),
    );
    let action_rect = Rect::from_min_max(
        egui::pos2(action_left, row.top()),
        egui::pos2(star_rect.left(), row.bottom()),
    );

    let mut star_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(star_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let star = star_ui.add_sized(
        [ICON_CELL, ROW_HEIGHT],
        egui::Button::new(tr(if favorite {
            Text::FontFavoriteOn
        } else {
            Text::FontFavoriteOff
        }))
        .frame(false),
    );
    star.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::CheckBox,
            true,
            favorite,
            i18n::format(locale, Text::FontFavorite, &[("family", family)]),
        )
    });
    record(
        &mut event.widgets,
        &format!("fonts.favorite.{family}"),
        star.clone(),
    );
    if star.clicked() {
        event.actions.push(Action::Favorite {
            family: family.to_owned(),
            favorite: !favorite,
        });
    }

    // Add the label itself instead of sizing it into the column: add_sized
    // centres a narrow label in its box, which would centre the family name.
    // The whole column still takes the click through a separate response.
    let mut name_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(name_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let text = name_ui.add(
        egui::Label::new(
            egui::RichText::new(family)
                .font(egui::FontId::new(
                    theme::type_size::CHROME,
                    row_family(state, family),
                ))
                .color(theme::text(dark)),
        )
        .halign(Align::LEFT)
        .truncate(),
    );
    record(&mut event.widgets, &format!("fonts.name.{family}"), text);
    let name = name_ui.interact(
        name_rect,
        ui.scope_id().with(("fonts.name_row", family)),
        Sense::click(),
    );
    name.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, family));
    record(
        &mut event.widgets,
        &format!("inspector.text_font.{family}"),
        name.clone(),
    );
    if name.clicked() {
        match entry.state {
            FontState::Available => event.actions.push(Action::Select(family.to_owned())),
            FontState::NotDownloaded => {
                if active.is_none_or(|download| download.failed) {
                    event.actions.push(Action::Download(family.to_owned()));
                }
            }
        }
    }

    if let Some((status, detail, id)) = row_status(entry, active, locale) {
        // Stack the state lines on the row's centre line. top_down starts at
        // its rect's top, so the rect itself carries the vertical offset.
        let line = ui.text_style_height(&egui::TextStyle::Small);
        let gap = ui.spacing().item_spacing.y;
        let lines = if detail.is_some() { 2.0 } else { 1.0 };
        let height = line * lines + gap * (lines - 1.0);
        let state_rect = Rect::from_min_size(
            egui::pos2(state_rect.left(), state_rect.center().y - height * 0.5),
            Vec2::new(state_rect.width(), height),
        );
        let mut state_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(state_rect)
                .layout(egui::Layout::top_down(Align::LEFT)),
        );
        let status = state_ui.label(egui::RichText::new(status).small().color(
            if active.is_some_and(|download| download.failed) {
                theme::error_text(dark)
            } else {
                theme::secondary(dark)
            },
        ));
        record(
            &mut event.widgets,
            &format!("fonts.state.{family}"),
            status.clone(),
        );
        if id == "fonts.downloading" || id == "fonts.download_failed" {
            record(&mut event.widgets, id, status);
        }
        if let Some(detail) = detail {
            let size = state_ui.label(
                egui::RichText::new(detail)
                    .small()
                    .color(theme::secondary(dark)),
            );
            record(
                &mut event.widgets,
                &format!("fonts.state_size.{family}"),
                size,
            );
        }
    }

    let mut action_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(action_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    match entry.state {
        FontState::Available => {}
        FontState::NotDownloaded => match active {
            Some(download) if download.failed => {
                let retry = action_ui.add_sized(
                    [ACTION_WIDTH, theme::size::FIELD],
                    egui::Button::new(tr(Text::Retry)),
                );
                record(
                    &mut event.widgets,
                    &format!("fonts.download_retry.{family}"),
                    retry.clone(),
                );
                if retry.clicked() {
                    event.actions.push(Action::Download(family.to_owned()));
                }
            }
            Some(download) if download.job.is_some() => {
                let cancel = action_ui.add_enabled(
                    !download.cancelling,
                    egui::Button::new(tr(Text::Cancel))
                        .min_size(Vec2::new(ACTION_WIDTH, theme::size::FIELD)),
                );
                record(&mut event.widgets, "fonts.cancel", cancel.clone());
                if cancel.clicked() {
                    event.actions.push(Action::Cancel);
                }
            }
            _ => {
                let download = theme::icon_button(
                    &mut action_ui,
                    &format!("fonts.download.{family}"),
                    theme::Icon::DownloadSimple,
                    tr(Text::FontDownload),
                    false,
                );
                record(
                    &mut event.widgets,
                    &format!("fonts.download.{family}"),
                    download.clone(),
                );
                if download.clicked() {
                    event.actions.push(Action::Download(family.to_owned()));
                }
            }
        },
    }
}

/// The state cell's lines, or `None` when the row needs no state text.
fn row_status(
    entry: &FontEntry,
    active: Option<&Download>,
    locale: Locale,
) -> Option<(String, Option<String>, &'static str)> {
    let tr = |key| i18n::text(locale, key);
    if let Some(download) = active {
        if download.failed {
            return Some((
                tr(Text::FontDownloadFailed).to_owned(),
                None,
                "fonts.download_failed",
            ));
        }
        if download.cancelling {
            return Some((
                tr(Text::FontDownloading).to_owned(),
                None,
                "fonts.downloading",
            ));
        }
        let percent = i18n::number(locale, (download.progress * 100.0).round() as u64);
        return Some((
            i18n::format(
                locale,
                Text::FontDownloadingPercent,
                &[("state", tr(Text::FontDownloading)), ("percent", &percent)],
            ),
            None,
            "fonts.downloading",
        ));
    }
    match entry.state {
        // A ready row says everything through its name and typeface.
        FontState::Available => None,
        FontState::NotDownloaded => Some((
            tr(Text::FontNotDownloaded).to_owned(),
            entry.download_bytes.map(|bytes| human_size(locale, bytes)),
            "",
        )),
    }
}

fn chip_ink(selected: bool, dark: bool) -> Color32 {
    if selected {
        theme::on_accent(dark)
    } else {
        theme::text(dark)
    }
}

fn chip_fill(selected: bool, dark: bool) -> Color32 {
    if selected {
        theme::accent(dark)
    } else {
        theme::field(dark)
    }
}

fn preview_key(family: &str) -> String {
    format!("font-preview:{family}")
}

/// Install a typeface for every available family the shared text registry can
/// resolve. Not-yet-loaded and refused families keep the UI font.
fn install_previews(state: &mut State, ctx: &egui::Context) {
    // Fonts installed on the previous pass are usable from this one.
    state.preview_fonts.append(&mut state.preview_pending);
    if state.status != Status::Ready {
        return;
    }
    let mut pending = Vec::new();
    for entry in &state.entries {
        if entry.state != FontState::Available
            || state.preview_fonts.contains(&entry.family)
            || state.preview_pending.contains(&entry.family)
        {
            continue;
        }
        let resolved = text::resolve(&entry.family, "Regular");
        if resolved.missing {
            continue;
        }
        pending.push((entry.family.clone(), text::face(resolved.face).bytes()));
    }
    if pending.is_empty() {
        return;
    }
    let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    for (family, bytes) in pending {
        let key = preview_key(&family);
        definitions
            .font_data
            .insert(key.clone(), Arc::new(egui::FontData::from_static(bytes)));
        definitions.families.insert(
            egui::FontFamily::Name(key.as_str().into()),
            vec![key.clone(), "Inter".into(), "CJK".into()],
        );
        state.preview_pending.insert(family);
    }
    ctx.set_fonts(definitions);
}

/// The typeface a closed Font field draws its family in. The family is installed from the
/// shared text registry on first use; the frame that installs it still draws in the UI font,
/// because egui applies new definitions on the next pass. Unresolvable names keep the UI font.
pub(in crate::editor) fn label_family(ctx: &egui::Context, family: &str) -> egui::FontFamily {
    let key = preview_key(family);
    let face = egui::FontFamily::Name(key.as_str().into());
    if ctx.fonts(|fonts| fonts.definitions().families.contains_key(&face)) {
        return face;
    }
    let resolved = text::resolve(family, "Regular");
    if resolved.missing {
        return egui::FontFamily::Proportional;
    }
    let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    definitions.font_data.insert(
        key.clone(),
        Arc::new(egui::FontData::from_static(
            text::face(resolved.face).bytes(),
        )),
    );
    definitions
        .families
        .insert(face, vec![key, "Inter".into(), "CJK".into()]);
    ctx.set_fonts(definitions);
    egui::FontFamily::Proportional
}

fn row_family(state: &State, family: &str) -> egui::FontFamily {
    if state.preview_fonts.contains(family) {
        egui::FontFamily::Name(preview_key(family).into())
    } else {
        egui::FontFamily::Proportional
    }
}

fn filter_label(filter: Filter) -> Text {
    match filter {
        Filter::All => Text::FontsAll,
        Filter::Commercial => Text::FontsCommercial,
    }
}

/// One section header band. `collapsed` is `Some` for the collapsible Presets section, which
/// shows a caret and answers a click; the return value says the click happened.
fn section_header(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    collapsed: Option<bool>,
    dark: bool,
    event: &mut Event,
) -> bool {
    let sense = if collapsed.is_some() {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), SECTION_HEIGHT), sense);
    let text = ui.painter().text(
        egui::pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::type_size::CHROME),
        theme::secondary(dark),
    );
    if let Some(collapsed) = collapsed {
        let icon = if collapsed {
            theme::Icon::CaretRight
        } else {
            theme::Icon::CaretDown
        };
        ui.put(
            Rect::from_center_size(
                egui::pos2(text.right() + theme::space::SM, rect.center().y),
                Vec2::splat(theme::icon::DENSE),
            ),
            theme::tinted(icon, theme::icon::DENSE, theme::secondary(dark)),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, !collapsed, label)
        });
    }
    let clicked = response.clicked();
    record(&mut event.widgets, id, response);
    clicked
}

/// The selected row's check mark in the name column's gutter, drawn as two strokes so it needs
/// no glyph or icon asset.
fn paint_check(ui: &egui::Ui, row: Rect, dark: bool) {
    let center = egui::pos2(row.left() + CHECK_WIDTH * 0.5, row.center().y);
    ui.painter().add(egui::Shape::line(
        vec![
            center + Vec2::new(-4.0, 0.0),
            center + Vec2::new(-1.5, 3.0),
            center + Vec2::new(4.0, -3.0),
        ],
        Stroke::new(1.5, theme::accent(dark)),
    ));
}

fn record(widgets: &mut Vec<(String, egui::Response)>, id: &str, response: egui::Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
    widgets.push((id.to_owned(), response));
}

fn human_size(locale: Locale, bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let number = if unit == 0 {
        i18n::number(locale, bytes)
    } else {
        i18n::decimal(locale, value, 1)
    };
    format!("{number} {}", UNITS.get(unit).copied().unwrap_or("B"))
}

#[cfg(test)]
mod tests {
    use super::{Download, chip_fill, chip_ink, row_status};
    use crate::i18n::Locale;
    use crate::kit as theme;
    use engine::control::{FontEntry, FontOrigin, FontState};
    use engine::jobs::JobId;

    fn entry(state: FontState, download_bytes: Option<u64>) -> FontEntry {
        FontEntry {
            family: "Lobster".into(),
            origin: FontOrigin::Google,
            styles: vec!["Regular".into()],
            state,
            license: None,
            source: None,
            download_bytes,
        }
    }

    fn download(progress: f32, cancelling: bool, failed: bool) -> Download {
        Download {
            family: "Lobster".into(),
            job: Some(JobId(7)),
            progress,
            cancelling,
            failed,
        }
    }

    #[test]
    fn state_text_appears_only_when_it_adds_information() {
        assert!(
            row_status(&entry(FontState::Available, None), None, Locale::En).is_none(),
            "a ready row carries no state text"
        );
        let (status, size, _) = row_status(
            &entry(FontState::NotDownloaded, Some(61_000)),
            None,
            Locale::En,
        )
        .expect("a not-downloaded row shows its state");
        assert_eq!(status, "Not downloaded");
        assert!(size.expect("the size is shown").ends_with("KB"));
        let (status, size, _) =
            row_status(&entry(FontState::NotDownloaded, None), None, Locale::En)
                .expect("a not-downloaded row without a size still shows its state");
        assert_eq!(status, "Not downloaded");
        assert!(size.is_none());
        let not_downloaded = entry(FontState::NotDownloaded, None);
        let (status, size, id) = row_status(
            &not_downloaded,
            Some(&download(0.4, false, false)),
            Locale::En,
        )
        .expect("a download shows progress");
        assert!(status.starts_with("Downloading"));
        assert!(size.is_none());
        assert_eq!(id, "fonts.downloading");
        let (status, _, id) = row_status(
            &not_downloaded,
            Some(&download(0.4, true, false)),
            Locale::En,
        )
        .expect("a cancelling download still shows");
        assert_eq!(status, "Downloading…");
        assert_eq!(id, "fonts.downloading");
        let (status, _, id) = row_status(
            &not_downloaded,
            Some(&download(0.4, false, true)),
            Locale::En,
        )
        .expect("a failed download shows its failure");
        assert_eq!(status, "Download failed");
        assert_eq!(id, "fonts.download_failed");
    }

    #[test]
    fn selected_chip_uses_the_accent_fill_and_on_accent_ink() {
        for dark in [true, false] {
            assert_eq!(chip_fill(true, dark), theme::accent(dark));
            assert_eq!(chip_ink(true, dark), theme::on_accent(dark));
            assert_eq!(chip_fill(false, dark), theme::field(dark));
            assert_eq!(chip_ink(false, dark), theme::text(dark));
        }
    }
}
