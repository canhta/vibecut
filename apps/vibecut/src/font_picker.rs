//! Presents the engine font catalog with search, source tabs, favourites and
//! explicit downloads. Presentation only: every catalog operation is a
//! `fonts.*` engine command and project edits stay with the properties owner.
use crate::catalog::{self, FontPickerPreferences, FontTab, Locale, Text};
use crate::egui::{self, Align2, Id, Vec2};
use crate::kit as theme;
use engine::control::{
    API_VERSION, Caller, ControlResult, FontAccess, FontEntry, FontLoad, FontState, JobView,
    Policy, Request, ResponseValue, registry_hash,
};
use engine::jobs::{JobId, JobRunner, RunnerConfig, SystemClock};
use engine::{ByteBudget, EditError, Session};
use std::sync::Arc;
use std::time::Duration;

/// Which family group All fonts shows. System/Google is our filter, not a
/// source tab; Custom waits for font import (80).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    All,
    System,
    Google,
    Custom,
}
impl Category {
    pub const ALL: [Self; 4] = [Self::All, Self::System, Self::Google, Self::Custom];
    pub const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::System => "system",
            Self::Google => "google",
            Self::Custom => "custom",
        }
    }
    pub const fn label(self) -> Text {
        match self {
            Self::All => Text::FontsAll,
            Self::System => Text::FontsSystem,
            Self::Google => Text::FontsGoogle,
            Self::Custom => Text::FontsCustom,
        }
    }
}

/// Listing state of the picker: the previews are loading, ready or refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Loading,
    Ready,
    Failed,
}

/// One in-flight or failed catalog download.
#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub family: String,
    pub job: Option<JobId>,
    pub progress: f32,
    pub cancelling: bool,
    pub failed: bool,
}

/// Transient picker state; the tab and favourites live in preferences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub open: bool,
    pub anchor: Option<egui::Pos2>,
    pub query: String,
    pub category: Category,
    pub status: Status,
    pub entries: Vec<FontEntry>,
    pub loaded: bool,
    /// A `fonts.list` request has been emitted and no answer is in yet.
    pub request_sent: bool,
    /// Imported families become a My fonts source once 80 lands.
    pub has_imports: bool,
    pub current: Option<String>,
    pub download: Option<Download>,
}
impl State {
    pub fn open(&mut self, anchor: egui::Pos2, current: Option<String>) {
        self.open = true;
        self.anchor = Some(anchor);
        self.query.clear();
        self.category = Category::All;
        self.current = current;
        if !self.loaded {
            self.status = Status::Loading;
            self.request_sent = false;
        }
    }
    pub fn close(&mut self) {
        self.open = false;
    }
    /// My fonts stays hidden until imported families exist.
    pub fn tabs(&self) -> Vec<FontTab> {
        if self.has_imports {
            vec![FontTab::All, FontTab::MyFonts, FontTab::Favorites]
        } else {
            vec![FontTab::All, FontTab::Favorites]
        }
    }
    /// The persisted tab unless it is not currently available.
    pub fn tab(&self, preferences: &FontPickerPreferences) -> FontTab {
        let tabs = self.tabs();
        if tabs.contains(&preferences.tab) {
            preferences.tab
        } else {
            FontTab::All
        }
    }
    /// Entries the active tab, category and search show, in catalog order.
    pub fn visible(&self, preferences: &FontPickerPreferences) -> Vec<&FontEntry> {
        let query = self.query.trim().to_lowercase();
        let tab = self.tab(preferences);
        self.entries
            .iter()
            .filter(|entry| match tab {
                FontTab::All => match self.category {
                    Category::All => true,
                    Category::System => entry.origin == engine::fonts::FontOrigin::System,
                    Category::Google => entry.origin == engine::fonts::FontOrigin::Google,
                    Category::Custom => false,
                },
                FontTab::Favorites => preferences
                    .favorites
                    .iter()
                    .any(|family| family == &entry.family),
                // No imported origin exists until 80; the tab only shows then.
                FontTab::MyFonts => false,
            })
            .filter(|entry| query.is_empty() || entry.family.to_lowercase().contains(&query))
            .collect()
    }
    /// Next available family after `family` in the visible order, then before it.
    pub fn next_available(
        &self,
        preferences: &FontPickerPreferences,
        family: &str,
    ) -> Option<String> {
        let visible = self.visible(preferences);
        let at = visible.iter().position(|entry| entry.family == family)?;
        visible
            .iter()
            .cycle()
            .skip(at + 1)
            .take(visible.len().saturating_sub(1))
            .find(|entry| entry.state == FontState::Available)
            .map(|entry| entry.family.clone())
    }
    pub fn inspect(&self, preferences: &FontPickerPreferences) -> Inspection {
        Inspection {
            open: self.open,
            tab: self.tab(preferences),
            category: self.category,
            query: self.query.clone(),
            status: self.status,
            entries: self.entries.clone(),
            favorites: preferences.favorites.clone(),
            download: self.download.clone(),
            has_imports: self.has_imports,
        }
    }
}

/// Snapshot for tests and agents; it never carries file paths or bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    pub open: bool,
    pub tab: FontTab,
    pub category: Category,
    pub query: String,
    pub status: Status,
    pub entries: Vec<FontEntry>,
    pub favorites: Vec<String>,
    pub download: Option<Download>,
    pub has_imports: bool,
}

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

/// The three catalog operations the picker consumes. A second adapter is the
/// test host; production uses [`EngineBackend`].
pub trait Backend: Send {
    fn list(&mut self, session: &mut Session, query: Option<&str>)
    -> crate::Result<Vec<FontEntry>>;
    fn load(&mut self, session: &mut Session, family: &str) -> crate::Result<FontLoad>;
    fn download(&mut self, session: &mut Session, family: &str) -> crate::Result<JobId>;
    fn job(&mut self, job: JobId) -> Option<JobView>;
    fn cancel(&mut self, session: &mut Session, job: JobId) -> crate::Result<()>;
}

/// Production adapter: the shared engine font store behind the registered
/// `fonts.list`, `fonts.load`, `fonts.download` and `job_cancel` commands.
pub struct EngineBackend {
    store: Arc<engine::fonts::FontStore>,
    runner: JobRunner,
    sequence: u64,
}
impl EngineBackend {
    pub fn native() -> crate::Result<Self> {
        let store = Arc::new(engine::fonts::FontStore::native()?);
        Self::with_store(store)
    }
    pub fn with_store(store: Arc<engine::fonts::FontStore>) -> crate::Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(250),
                progress_interval: Duration::from_millis(40),
            },
            ByteBudget::new(64 << 20)?,
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            store,
            runner,
            sequence: 0,
        })
    }
    fn request(
        &mut self,
        session: &mut Session,
        command: &str,
        params: serde_json::Value,
    ) -> crate::Result<ControlResult> {
        let store = &self.store;
        let runner = &mut self.runner;
        let list = |query: Option<&str>| store.list(query);
        let load = |family: &str| store.load(family);
        let download = |runner: &mut JobRunner, family: &str, accepted: bool| {
            store
                .submit_download(runner, family, accepted)
                .map(|(id, _)| id)
        };
        let access = FontAccess::new(&list, &load, &download);
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        let request_id = format!("vibecut-font-{}-{sequence}", std::process::id());
        let request = Request {
            api_version: API_VERSION,
            registry_hash: Some(registry_hash().into()),
            session_id: session.session_id().into(),
            idempotency_key: Some(request_id.clone()),
            request_id,
            base_rev: Some(session.revision()),
            command: command.into(),
            params,
        };
        let response = session.control_request(
            request,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Allow,
                fonts: Some(&access),
            },
            runner,
        )?;
        match response.value {
            ResponseValue::Result(value) => Ok(value),
            ResponseValue::Error(code) => Err(EditError::Invalid(match code {
                engine::control::ErrorCode::Permission => "font command refused",
                engine::control::ErrorCode::QueueFull => "font queue full",
                engine::control::ErrorCode::Cancelled => "font command cancelled",
                _ => "font command failed",
            })
            .into()),
        }
    }
}
impl Backend for EngineBackend {
    fn list(
        &mut self,
        session: &mut Session,
        query: Option<&str>,
    ) -> crate::Result<Vec<FontEntry>> {
        let params = match query {
            Some(query) => serde_json::json!({"query": query}),
            None => serde_json::json!({}),
        };
        match self.request(session, "fonts.list", params)? {
            ControlResult::Fonts { entries } => Ok(entries),
            _ => Err(crate::Error::Invalid("fonts.list result")),
        }
    }
    fn load(&mut self, session: &mut Session, family: &str) -> crate::Result<FontLoad> {
        match self.request(session, "fonts.load", serde_json::json!({"family": family}))? {
            ControlResult::Font { font } => Ok(font),
            _ => Err(crate::Error::Invalid("fonts.load result")),
        }
    }
    fn download(&mut self, session: &mut Session, family: &str) -> crate::Result<JobId> {
        match self.request(
            session,
            "fonts.download",
            serde_json::json!({"family": family, "accept_license": true}),
        )? {
            ControlResult::JobAccepted { job, .. } => Ok(job),
            _ => Err(crate::Error::Invalid("fonts.download result")),
        }
    }
    fn job(&mut self, job: JobId) -> Option<JobView> {
        self.runner.snapshot(job).map(JobView::from)
    }
    fn cancel(&mut self, session: &mut Session, job: JobId) -> crate::Result<()> {
        match self.request(session, "job_cancel", serde_json::json!({"job": job}))? {
            ControlResult::CancelRequested { .. } => Ok(()),
            _ => Err(crate::Error::Invalid("job_cancel result")),
        }
    }
}
struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}

/// Draw the open picker as a popover right-aligned to `anchor`.
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
    let tr = |key| catalog::text(locale, key);
    let window = egui::Window::new(tr(Text::TextFont))
        .id(Id::unique("fonts.picker"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(anchor);
    window.show(ctx, |ui| {
        ui.set_width(320.0_f32.min(ctx.content_rect().width() - 48.0));
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr(Text::TextFont))
                    .strong()
                    .size(theme::type_size::BODY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                for category in Category::ALL {
                    let enabled = category != Category::Custom;
                    let id = format!("fonts.category.{}", category.id());
                    let response = ui
                        .push_id(&id, |ui| {
                            ui.add_enabled(
                                enabled,
                                egui::Button::selectable(
                                    state.category == category,
                                    tr(category.label()),
                                ),
                            )
                        })
                        .inner;
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
                    let label = ui.label(
                        egui::RichText::new(tr(Text::FontsNone)).color(theme::secondary(dark)),
                    );
                    record(&mut event.widgets, "fonts.empty", label);
                    ui.add_space(theme::space::LG);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("fonts.list")
                        .max_height(288.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
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

#[allow(clippy::too_many_arguments)]
fn font_row(
    ui: &mut egui::Ui,
    entry: &FontEntry,
    state: &State,
    preferences: &FontPickerPreferences,
    locale: Locale,
    dark: bool,
    event: &mut Event,
) {
    let tr = |key| catalog::text(locale, key);
    let family = entry.family.clone();
    let favorite = preferences.favorites.iter().any(|name| name == &family);
    ui.horizontal(|ui| {
        let star = ui
            .push_id(("fonts.favorite", family.as_str()), |ui| {
                ui.add(egui::Button::new(tr(if favorite {
                    Text::FontFavoriteOn
                } else {
                    Text::FontFavoriteOff
                })))
            })
            .inner;
        star.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                true,
                favorite,
                catalog::format(locale, Text::FontFavorite, &[("family", &family)]),
            )
        });
        record(
            &mut event.widgets,
            &format!("fonts.favorite.{family}"),
            star.clone(),
        );
        if star.clicked() {
            event.actions.push(Action::Favorite {
                family: family.clone(),
                favorite: !favorite,
            });
        }
        match entry.state {
            FontState::Available => {
                let selected = state.current.as_deref() == Some(family.as_str());
                let width = (ui.available_width() - 4.0).max(80.0);
                let height = theme::size::PROPERTY_ROW;
                let response = ui
                    .push_id(("inspector.text_font", family.as_str()), |ui| {
                        ui.add_sized(
                            [width, height],
                            egui::Button::selectable(
                                selected,
                                egui::RichText::new(&family).color(if selected {
                                    theme::on_accent(dark)
                                } else {
                                    theme::text(dark)
                                }),
                            ),
                        )
                    })
                    .inner;
                record(
                    &mut event.widgets,
                    &format!("inspector.text_font.{family}"),
                    response.clone(),
                );
                if response.clicked() {
                    event.actions.push(Action::Select(family.clone()));
                }
            }
            FontState::NotDownloaded => {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&family).color(theme::text(dark)));
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(tr(Text::FontNotDownloaded))
                                .small()
                                .color(theme::secondary(dark)),
                        );
                        if let Some(bytes) = entry.download_bytes {
                            ui.label(
                                egui::RichText::new(human_size(locale, bytes))
                                    .small()
                                    .color(theme::secondary(dark)),
                            );
                        }
                    });
                });
                let download = ui.add_enabled(
                    state
                        .download
                        .as_ref()
                        .is_none_or(|download| download.family != family),
                    egui::Button::new(tr(Text::FontDownload)),
                );
                record(
                    &mut event.widgets,
                    &format!("fonts.download.{family}"),
                    download.clone(),
                );
                if download.clicked() {
                    event.actions.push(Action::Download(family.clone()));
                }
            }
        }
    });
    if let Some(download) = state
        .download
        .as_ref()
        .filter(|download| download.family == family)
    {
        ui.horizontal(|ui| {
            if download.failed {
                let failed = ui.label(
                    egui::RichText::new(tr(Text::FontDownloadFailed))
                        .small()
                        .color(theme::error_text(dark)),
                );
                record(&mut event.widgets, "fonts.download_failed", failed);
                let retry = ui.button(tr(Text::Retry));
                record(
                    &mut event.widgets,
                    &format!("fonts.download_retry.{family}"),
                    retry.clone(),
                );
                if retry.clicked() {
                    event.actions.push(Action::Download(family.clone()));
                }
            } else if download.cancelling {
                let label = ui.label(
                    egui::RichText::new(tr(Text::FontDownloading))
                        .small()
                        .color(theme::secondary(dark)),
                );
                record(&mut event.widgets, "fonts.downloading", label);
            } else {
                let progress = ui.add(
                    egui::ProgressBar::new(download.progress)
                        .desired_width((ui.available_width() - 88.0).max(60.0))
                        .show_percentage(),
                );
                record(&mut event.widgets, "fonts.progress", progress);
                let cancel = ui.button(tr(Text::Cancel));
                record(&mut event.widgets, "fonts.cancel", cancel.clone());
                if cancel.clicked() {
                    event.actions.push(Action::Cancel);
                }
            }
        });
    }
}

fn tab_label(tab: FontTab, locale: Locale) -> &'static str {
    catalog::text(
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
        catalog::number(locale, bytes)
    } else {
        catalog::decimal(locale, value, 1)
    };
    format!("{number} {}", UNITS.get(unit).copied().unwrap_or("B"))
}

#[cfg(test)]
mod tests {
    use super::{Category, State, Status};
    use crate::catalog::{FontPickerPreferences, FontTab};
    use engine::control::{FontEntry, FontOrigin, FontState};

    fn entry(family: &str, origin: FontOrigin, state: FontState) -> FontEntry {
        FontEntry {
            family: family.into(),
            origin,
            styles: vec!["Regular".into()],
            state,
            license: None,
            source: None,
            download_bytes: None,
        }
    }
    fn state() -> State {
        State {
            open: true,
            status: Status::Ready,
            entries: vec![
                entry("Inter", FontOrigin::Bundled, FontState::Available),
                entry("Arial", FontOrigin::System, FontState::Available),
                entry("Lobster", FontOrigin::Google, FontState::NotDownloaded),
            ],
            loaded: true,
            ..State::default()
        }
    }

    #[test]
    fn my_fonts_stays_hidden_until_import_is_available() {
        let mut state = state();
        assert_eq!(state.tabs(), [FontTab::All, FontTab::Favorites]);
        let preferences = FontPickerPreferences {
            tab: FontTab::MyFonts,
            ..FontPickerPreferences::default()
        };
        assert_eq!(
            state.tab(&preferences),
            FontTab::All,
            "a hidden tab falls back"
        );
        state.has_imports = true;
        assert_eq!(
            state.tabs(),
            [FontTab::All, FontTab::MyFonts, FontTab::Favorites]
        );
        assert_eq!(state.tab(&preferences), FontTab::MyFonts);
    }

    #[test]
    fn search_category_and_favorites_filter_the_visible_catalog() {
        let state = state();
        let mut preferences = FontPickerPreferences::default();
        assert_eq!(state.visible(&preferences).len(), 3);
        let mut search = state.clone();
        search.query = "LO".into();
        let visible = search.visible(&preferences);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].family, "Lobster");
        let mut system = state.clone();
        system.category = Category::System;
        assert_eq!(system.visible(&preferences)[0].family, "Arial");
        let mut google = state.clone();
        google.category = Category::Google;
        assert_eq!(google.visible(&preferences)[0].family, "Lobster");
        let mut custom = state.clone();
        custom.category = Category::Custom;
        assert!(custom.visible(&preferences).is_empty());
        preferences.favorites = vec!["Inter".into(), "Lobster".into()];
        preferences.tab = FontTab::Favorites;
        let favorites: Vec<_> = state
            .visible(&preferences)
            .into_iter()
            .map(|entry| entry.family.as_str())
            .collect();
        assert_eq!(favorites, ["Inter", "Lobster"]);
        let mut mine = state.clone();
        mine.has_imports = true;
        preferences.tab = FontTab::MyFonts;
        assert!(
            mine.visible(&preferences).is_empty(),
            "no imports exist yet"
        );
    }

    #[test]
    fn a_failed_preview_falls_back_to_the_next_available_family() {
        let state = state();
        let preferences = FontPickerPreferences::default();
        assert_eq!(
            state.next_available(&preferences, "Arial").as_deref(),
            Some("Inter"),
            "the search wraps past a not-downloaded family"
        );
        let mut catalog = state;
        catalog.entries[1].state = FontState::NotDownloaded;
        catalog.entries[2].state = FontState::NotDownloaded;
        assert_eq!(
            catalog.next_available(&preferences, "Arial").as_deref(),
            Some("Inter"),
            "a not-downloaded neighbour is skipped"
        );
    }
}
