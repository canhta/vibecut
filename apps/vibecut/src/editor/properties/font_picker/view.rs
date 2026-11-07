//! Draws the font picker popover and turns its events into picker actions.
use super::state::{Category, Download, State, Status};
use crate::egui::{self, Align, Align2, Color32, Id, Rect, Sense, Stroke, Vec2};
use crate::i18n::{self, Locale, Text};
use crate::kit as theme;
use crate::preferences::{FontPickerPreferences, FontTab};
use engine::control::{FontEntry, FontState};
use std::sync::Arc;

/// Popover width and list ceiling; the reference's own list is 288 px.
const WIDTH: f32 = 320.0;
const LIST_HEIGHT: f32 = 288.0;
const ROW_HEIGHT: f32 = 34.0;
const STAR_WIDTH: f32 = 24.0;
const STATE_WIDTH: f32 = 96.0;
const ACTION_WIDTH: f32 = 84.0;

/// What the picker asks the properties owner to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Close,
    /// Ask the backend for the catalog through `fonts.list`.
    List,
    Tab(FontTab),
    /// Toggle one family in the persisted favourites.
    Favorite {
        family: String,
        favorite: bool,
    },
    /// Load and apply an available family through `fonts.load`.
    Select(String),
    /// Start one explicit catalog download through `fonts.download`.
    Download(String),
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
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(anchor)
        .constrain_to(ctx.content_rect().shrink(8.0));
    window.show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr(Text::TextFont))
                    .strong()
                    .size(theme::type_size::BODY),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let close = theme::icon_button(
                    ui,
                    "fonts.close",
                    theme::Icon::X,
                    tr(Text::FontClose),
                    false,
                );
                record(&mut event.widgets, "fonts.close", close.clone());
                if close.clicked() {
                    event.actions.push(Action::Close);
                }
            });
        });
        let search = ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .id(Id::unique("fonts.search"))
                .hint_text(tr(Text::FontsSearch))
                .desired_width(f32::INFINITY),
        );
        record(&mut event.widgets, "fonts.search", search);
        ui.add_space(theme::space::XS);

        let tabs = state.tabs();
        let ids: Vec<String> = tabs
            .iter()
            .map(|tab| format!("fonts.tab.{}", tab.id()))
            .collect();
        let labels: Vec<&str> = tabs.iter().map(|tab| tab_label(*tab, locale)).collect();
        let selected = tabs
            .iter()
            .position(|tab| *tab == state.tab(preferences))
            .unwrap_or(0);
        let responses = theme::text_tabs(ui, &ids, &labels, selected);
        for ((id, response), tab) in ids.iter().zip(&responses).zip(&tabs) {
            record(&mut event.widgets, id, response.clone());
            if response.clicked() {
                event.actions.push(Action::Tab(*tab));
            }
        }
        if state.tab(preferences) == FontTab::All {
            ui.add_space(theme::space::XS);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = theme::space::XS;
                for category in Category::ALL {
                    let enabled = category != Category::Custom;
                    let id = format!("fonts.category.{}", category.id());
                    let label = tr(category_label(category));
                    let selected = state.category == category;
                    let button = egui::Button::new(
                        egui::RichText::new(label).color(chip_ink(selected, enabled, dark)),
                    )
                    .fill(chip_fill(selected, enabled, dark))
                    .stroke(Stroke::new(theme::space::HAIRLINE, theme::border(dark)))
                    .min_size(Vec2::new(0.0, theme::size::FIELD));
                    let response = ui.push_id(&id, |ui| ui.add_enabled(enabled, button)).inner;
                    record(&mut event.widgets, &id, response.clone());
                    if enabled && response.clicked() {
                        state.category = category;
                    }
                }
            });
        }
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
                let rows: Vec<FontEntry> =
                    state.visible(preferences).into_iter().cloned().collect();
                if rows.is_empty() {
                    ui.add_space(theme::space::LG);
                    let message =
                        if state.tab(preferences) == FontTab::MyFonts && !state.has_imports {
                            Text::FontUploadHint
                        } else {
                            Text::FontsNone
                        };
                    let label =
                        ui.label(egui::RichText::new(tr(message)).color(theme::secondary(dark)));
                    record(
                        &mut event.widgets,
                        if message == Text::FontUploadHint {
                            "fonts.my_empty"
                        } else {
                            "fonts.empty"
                        },
                        label,
                    );
                    ui.add_space(theme::space::LG);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("fonts.list")
                        .max_height(LIST_HEIGHT)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = theme::space::XXS;
                            for entry in rows {
                                font_row(ui, &entry, state, preferences, locale, dark, &mut event);
                            }
                        });
                }
            }
        }
        ui.separator();
        let upload = ui.add_enabled(
            false,
            egui::Button::new(tr(Text::FontUpload)).min_size(Vec2::new(0.0, theme::size::FIELD)),
        );
        upload.clone().on_hover_text(tr(Text::FontUploadHint));
        record(&mut event.widgets, "fonts.upload", upload);
    });
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
    }
    let star_rect = Rect::from_min_size(row.min, Vec2::new(STAR_WIDTH, ROW_HEIGHT));
    let action_left = row.right() - ACTION_WIDTH;
    let state_left = action_left - STATE_WIDTH;
    let name_rect = Rect::from_min_max(
        egui::pos2(row.left() + STAR_WIDTH, row.top()),
        egui::pos2(state_left.max(row.left() + STAR_WIDTH), row.bottom()),
    );
    let state_rect = Rect::from_min_max(
        egui::pos2(state_left, row.top()),
        egui::pos2(action_left, row.bottom()),
    );
    let action_rect = Rect::from_min_max(
        egui::pos2(action_left, row.top()),
        egui::pos2(row.right(), row.bottom()),
    );

    let mut star_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(star_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let star = star_ui.add_sized(
        [STAR_WIDTH, ROW_HEIGHT],
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
                let download = action_ui.add_sized(
                    [ACTION_WIDTH, theme::size::FIELD],
                    egui::Button::new(tr(Text::FontDownload)),
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

fn chip_ink(selected: bool, enabled: bool, dark: bool) -> Color32 {
    if !enabled {
        theme::disabled(dark)
    } else if selected {
        theme::on_accent(dark)
    } else {
        theme::text(dark)
    }
}

fn chip_fill(selected: bool, enabled: bool, dark: bool) -> Color32 {
    if selected && enabled {
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

fn row_family(state: &State, family: &str) -> egui::FontFamily {
    if state.preview_fonts.contains(family) {
        egui::FontFamily::Name(preview_key(family).into())
    } else {
        egui::FontFamily::Proportional
    }
}

fn category_label(category: Category) -> Text {
    match category {
        Category::All => Text::FontsAll,
        Category::System => Text::FontsSystem,
        Category::Google => Text::FontsGoogle,
        Category::Custom => Text::FontsCustom,
    }
}

fn tab_label(tab: FontTab, locale: Locale) -> &'static str {
    i18n::text(
        locale,
        match tab {
            FontTab::All => Text::FontsAll,
            FontTab::MyFonts => Text::FontsMine,
            FontTab::Favorites => Text::FontsFavorites,
        },
    )
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
            assert_eq!(chip_fill(true, true, dark), theme::accent(dark));
            assert_eq!(chip_ink(true, true, dark), theme::on_accent(dark));
            assert_eq!(chip_fill(false, true, dark), theme::field(dark));
            assert_eq!(chip_ink(false, true, dark), theme::text(dark));
            assert_eq!(chip_fill(false, false, dark), theme::field(dark));
            assert_eq!(chip_ink(false, false, dark), theme::disabled(dark));
        }
    }
}
