//! Renders the Sounds tab's provider results, saved library and states.
use super::super::sounds::{LibraryProblem, LibraryStore, SoundsMode, SoundsProblem, SoundsView};
use super::{
    Action, Align2, AssetsTab, Color32, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons,
    theme,
};
use crate::i18n;
use crate::preferences::SoundFavorite;

impl View<'_> {
    pub(super) fn sounds_nav(&mut self, ui: &mut egui::Ui) {
        // `None` is the Favorites entry; it lists starred sounds without changing the library mode.
        for (mode, label) in [
            (None, Text::AssetsFavorites),
            (Some(SoundsMode::Discover), Text::SoundsDiscover),
            (Some(SoundsMode::Saved), Text::SoundsSaved),
        ] {
            let selected = match mode {
                None => self.state.sounds_favorites,
                Some(mode) => !self.state.sounds_favorites && self.state.sounds_mode == mode,
            };
            let response = ui.add_sized(
                [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
                egui::Button::new(
                    egui::RichText::new(self.tr(label))
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL)
                        .color(if selected {
                            theme::accent(self.cx.preferences.dark)
                        } else {
                            theme::text(self.cx.preferences.dark)
                        }),
                )
                .fill(if selected {
                    theme::hover(self.cx.preferences.dark)
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(egui::Stroke::NONE)
                .right_text(""),
            );
            self.remember(
                format!("sounds.tab.{}", mode.map_or("favorites", SoundsMode::id)),
                &response,
                None,
            );
            if response.clicked() && !selected {
                match mode {
                    None => self.state.sounds_favorites = true,
                    Some(mode) => {
                        self.state.sounds_favorites = false;
                        self.state.sounds_mode = mode;
                        self.actions.push(Action::SoundsMode(mode));
                    }
                }
            }
        }
    }

    /// Starred sounds in star order, from the metadata stored with each star. No provider
    /// lookup, so a starred sound outside the current results is still listed. Author and
    /// licence are not stored; those rows show the duration alone.
    fn favorite_sounds(&self) -> Vec<libraries::sounds::SoundResult> {
        self.cx
            .preferences
            .catalog_favorites
            .sounds()
            .iter()
            .map(|sound| libraries::sounds::SoundResult {
                id: sound.id,
                name: sound.title.clone(),
                author: String::new(),
                licence: String::new(),
                attribution: String::new(),
                source_url: String::new(),
                audio_url: sound.preview_url.clone(),
                duration_ms: sound.duration_ms,
                downloads: 0,
            })
            .collect()
    }

    pub(super) fn sounds(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.cx.sounds.clone() else {
            match self.state.sounds_open {
                Some(problem) => self.library_open_failure(ui, LibraryStore::Sounds, problem),
                None => self.empty(
                    ui,
                    Icon::MusicNotes,
                    Text::SoundsUnavailable,
                    crate::kit::Tone::Quiet,
                ),
            }
            return;
        };
        let area = ui.available_rect_before_wrap();
        let dark = self.cx.preferences.dark;
        let width = area.width();
        let commercial = self.state.sounds_commercial;
        let row = Rect::from_min_size(area.min, Vec2::new(width, theme::size::CONTROL));
        let toggle_width = 36.0 + 8.0 + self.text_width(Text::SoundsCommercial);
        let field_width = (width - 24.0 - 8.0 - toggle_width - 8.0).max(72.0);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("sounds.search.row")
                .max_rect(Rect::from_min_size(
                    row.min,
                    Vec2::new(field_width, row.height()),
                )),
            |ui| self.sounds_search(ui, field_width),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("sounds.search.go")
                .max_rect(Rect::from_min_size(
                    row.min + Vec2::new(field_width + 4.0, 0.0),
                    Vec2::splat(theme::size::CONTROL),
                )),
            |ui| {
                if self.icon(
                    ui,
                    "sounds.search.go",
                    Icon::MagnifyingGlass,
                    Text::SoundsSearch,
                    true,
                ) {
                    self.actions.push(Action::SoundsSearch { more: false });
                }
            },
        );
        let toggle_x = row.right() - toggle_width;
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("sounds.commercial")
                .max_rect(Rect::from_min_size(
                    egui::pos2(toggle_x, row.top()),
                    Vec2::new(toggle_width, row.height()),
                )),
            |ui| {
                let mut on = commercial;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let response = crate::kit::toggle(
                        ui,
                        "sounds.commercial",
                        &mut on,
                        self.tr(Text::SoundsCommercial),
                    );
                    self.remember("sounds.commercial", &response, None);
                    ui.label(
                        egui::RichText::new(self.tr(Text::SoundsCommercial))
                            .size(theme::type_size::BODY)
                            .color(theme::secondary(dark)),
                    );
                    if response.changed() {
                        self.state.sounds_commercial = on;
                        self.actions.push(Action::SoundsCommercial(on));
                    }
                });
            },
        );
        ui.advance_cursor_after_rect(row);
        ui.add_space(8.0);

        if !view.has_token && view.mode == SoundsMode::Discover {
            self.notice(ui, Text::SoundsNoToken, dark, false);
        }
        self.problem_banner(ui, view.search_problem, dark, Some("sounds.retry"));
        self.problem_banner(ui, view.cache_problem, dark, Some("sounds.retry.cache"));
        self.problem_banner(ui, view.preview_problem, dark, None);
        self.problem_banner(ui, view.saved_problem, dark, None);
        if view.searching {
            ui.label(
                egui::RichText::new(self.tr(Text::SoundsSearching))
                    .size(theme::type_size::BODY)
                    .color(theme::secondary(dark)),
            );
        }
        if let Some(cache) = view.cache {
            let percent = (cache.progress * 100.0).round().clamp(0.0, 100.0) as u32;
            let text = i18n::format(
                self.cx.preferences.locale,
                Text::SoundsCaching,
                &[(
                    "percent",
                    &i18n::number(self.cx.preferences.locale, percent as u64),
                )],
            );
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(text)
                        .size(theme::type_size::BODY)
                        .color(theme::secondary(dark)),
                );
                if self.icon(
                    ui,
                    "sounds.cancel_cache.banner",
                    Icon::X,
                    Text::SoundsCancelCache,
                    cache.cancelling,
                ) {
                    self.actions.push(Action::SoundsCancelCache);
                }
            });
        }

        let favorites = self.state.sounds_favorites;
        let list = if favorites {
            self.favorite_sounds()
        } else {
            view.results.clone()
        };
        if list.is_empty() && !view.searching {
            self.empty(
                ui,
                Icon::MusicNotes,
                if favorites {
                    Text::AssetsFavoritesEmpty
                } else if view.mode == SoundsMode::Saved {
                    Text::SoundsNoSaved
                } else {
                    Text::SoundsNoResults
                },
                crate::kit::Tone::Quiet,
            );
            return;
        }
        if !list.is_empty() {
            let count = i18n::format(
                self.cx.preferences.locale,
                Text::SoundsResultCount,
                &[(
                    "count",
                    &i18n::number(self.cx.preferences.locale, list.len() as u64),
                )],
            );
            ui.label(
                egui::RichText::new(count)
                    .size(theme::type_size::SMALL_LABEL)
                    .color(theme::secondary(dark)),
            );
        }
        let viewport = ui.available_height().max(48.0);
        egui::ScrollArea::vertical()
            .id_salt("sounds.list")
            .max_height(viewport)
            .show(ui, |ui| {
                for sound in &list {
                    self.sound_row(ui, sound, &view);
                }
                if !favorites
                    && view.mode == SoundsMode::Discover
                    && view.has_next
                    && self.icon(ui, "sounds.more", Icon::Plus, Text::SoundsLoadMore, true)
                {
                    self.actions.push(Action::SoundsSearch { more: true });
                }
            });
    }

    fn sounds_search(&mut self, ui: &mut egui::Ui, width: f32) {
        let hint = self.tr(Text::SoundsSearch);
        let draft = self.state.searches.entry(AssetsTab::Sounds).or_default();
        let response = ui.add_sized(
            [width.max(24.0), theme::size::CONTROL],
            egui::TextEdit::singleline(draft)
                .char_limit(256)
                .hint_text(hint)
                .font(egui::FontId::proportional(theme::type_size::BODY)),
        );
        self.remember("sounds.search", &response, None);
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.actions.push(Action::SoundsSearch { more: false });
        }
    }

    fn sound_row(
        &mut self,
        ui: &mut egui::Ui,
        sound: &libraries::sounds::SoundResult,
        view: &SoundsView,
    ) {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(48.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(width, theme::size::AUDIO_ROW),
            egui::Sense::hover(),
        );
        let dark = self.cx.preferences.dark;
        let cached = view.cache.is_some_and(|cache| cache.sound == sound.id);
        let saved = view.saved_ids.contains(&sound.id);
        // Save and Add resolve the sound from the results or the saved library.
        let listed = saved || view.results.iter().any(|known| known.id == sound.id);
        ui.painter().rect_filled(
            rect,
            theme::radius::CONTROL,
            if response.hovered() {
                theme::hover(dark)
            } else {
                theme::field(dark)
            },
        );
        self.remember(
            format!("sounds.result.{}", sound.id),
            &response,
            Some(Icon::MusicNotes),
        );
        let label = format!("{} — {}", sound.name, sound.attribution);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
        let thumb = Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::splat(32.0));
        ui.painter().rect_filled(thumb, 4.0, theme::field(dark));
        icons::tinted(Icon::MusicNotes, 20.0, theme::secondary(dark)).paint_at(
            ui,
            Rect::from_center_size(thumb.center(), Vec2::splat(20.0)),
        );
        // Three buttons, the star slot and the gaps between them.
        let buttons = 24.0 * 3.0 + 8.0 + 28.0;
        let text_width = (rect.width() - 8.0 - 32.0 - 8.0 - buttons - 8.0).max(24.0);
        let clip = Rect::from_min_size(
            egui::pos2(thumb.right() + 8.0, rect.top() + 3.0),
            Vec2::new(text_width, rect.height() - 6.0),
        );
        let painter = ui.painter().with_clip_rect(clip);
        painter.text(
            clip.min,
            Align2::LEFT_TOP,
            &sound.name,
            theme::label_font(),
            theme::text(dark),
        );
        let meta = if sound.author.is_empty() && sound.licence.is_empty() {
            duration(sound.duration_ms)
        } else {
            i18n::format(
                self.cx.preferences.locale,
                Text::SoundsMeta,
                &[
                    ("author", &sound.author),
                    ("duration", &duration(sound.duration_ms)),
                    ("licence", &sound.licence),
                ],
            )
        };
        painter.text(
            clip.min + Vec2::new(0.0, 15.0),
            Align2::LEFT_TOP,
            meta,
            FontId::proportional(theme::type_size::SMALL_LABEL),
            theme::secondary(dark),
        );
        painter.text(
            clip.min + Vec2::new(0.0, 29.0),
            Align2::LEFT_TOP,
            &sound.attribution,
            FontId::proportional(theme::type_size::SMALL_LABEL),
            theme::secondary(dark),
        );
        if cached && let Some(cache) = view.cache {
            let bar = Rect::from_min_max(
                egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0),
                egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0),
            );
            ui.painter().rect_filled(bar, 0.0, theme::track_rest(dark));
            let filled = Rect::from_min_size(
                bar.min,
                Vec2::new(bar.width() * cache.progress.clamp(0.0, 1.0), bar.height()),
            );
            ui.painter().rect_filled(filled, 0.0, theme::accent(dark));
        }
        let start = rect.right() - 8.0 - 24.0 * 3.0 - 8.0;
        let star = Rect::from_min_size(
            egui::pos2(start - 28.0, rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        let key = sound.id.to_string();
        let starred = self
            .cx
            .preferences
            .catalog_favorites
            .contains("sounds", &key);
        let star_label = self.tr(if starred {
            Text::AssetsFavoriteRemove
        } else {
            Text::AssetsFavoriteAdd
        });
        let star_response = ui
            .interact(
                star,
                Id::unique(("sounds.favorite", sound.id)),
                egui::Sense::click(),
            )
            .on_hover_text(star_label);
        self.remember(
            format!("sounds.favorite.{}", sound.id),
            &star_response,
            None,
        );
        ui.painter().text(
            star.center(),
            Align2::CENTER_CENTER,
            if starred { "★" } else { "☆" },
            FontId::proportional(theme::type_size::BODY),
            theme::text(dark),
        );
        if star_response.clicked() {
            self.actions.push(Action::ToggleSoundFavorite {
                sound: SoundFavorite::from_result(sound),
                favorite: !starred,
            });
        }
        let previewing = view.previewing == Some(sound.id);
        let preview =
            Rect::from_min_size(egui::pos2(start, rect.center().y - 12.0), Vec2::splat(24.0));
        if self.sound_button(
            ui,
            preview,
            &format!("sounds.preview.{}", sound.id),
            if previewing { Icon::Pause } else { Icon::Play },
            if previewing {
                Text::SoundsStopPreview
            } else {
                Text::SoundsPreview
            },
            previewing,
        ) {
            // A listed sound previews by id; a favourite outside the results plays from
            // the URL its star stored.
            if listed {
                self.actions.push(Action::SoundsPreview(sound.id));
            } else {
                self.actions.push(Action::SoundsPlay(sound.clone()));
            }
        }
        let save = Rect::from_min_size(
            egui::pos2(start + 28.0, rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        if !listed {
            self.sound_unavailable(
                ui,
                save,
                &format!("sounds.save.unlisted.{}", sound.id),
                Icon::FloppyDisk,
            );
        } else if self.sound_button(
            ui,
            save,
            &format!("sounds.save.{}", sound.id),
            Icon::FloppyDisk,
            if saved {
                Text::SoundsUnsave
            } else {
                Text::SoundsSave
            },
            saved,
        ) {
            self.actions.push(Action::SoundsToggleSaved(sound.id));
        }
        let add = Rect::from_min_size(
            egui::pos2(start + 56.0, rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        if cached {
            if self.sound_button(
                ui,
                add,
                "sounds.cancel_cache",
                Icon::X,
                Text::SoundsCancelCache,
                true,
            ) {
                self.actions.push(Action::SoundsCancelCache);
            }
        } else if !listed {
            self.sound_unavailable(
                ui,
                add,
                &format!("sounds.add.unlisted.{}", sound.id),
                Icon::Plus,
            );
        } else if self.sound_button(
            ui,
            add,
            &format!("sounds.add.{}", sound.id),
            Icon::Plus,
            Text::SoundsAdd,
            false,
        ) {
            self.actions.push(Action::SoundsAdd(sound.id));
        }
    }

    /// A Save or Add slot that cannot act for a sound outside the results: a dimmed glyph
    /// whose hover text gives the reason.
    fn sound_unavailable(&mut self, ui: &mut egui::Ui, rect: Rect, id: &str, glyph: Icon) {
        let dark = self.cx.preferences.dark;
        let reason = self.tr(Text::SoundsNotListed).to_owned();
        let response = ui
            .interact(rect, Id::unique(id), egui::Sense::hover())
            .on_hover_text(reason);
        icons::tinted(glyph, theme::icon::DENSE, theme::secondary(dark))
            .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)));
        self.remember(id, &response, Some(glyph));
    }

    fn sound_button(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        id: &str,
        glyph: Icon,
        label: Text,
        selected: bool,
    ) -> bool {
        let dark = self.cx.preferences.dark;
        let response = ui
            .interact(rect, Id::unique(id), egui::Sense::click())
            .on_hover_text(self.tr(label));
        if response.hovered() || response.has_focus() || selected {
            ui.painter()
                .rect_filled(rect, theme::radius::CONTROL, theme::hover(dark));
        }
        icons::tinted(
            glyph,
            theme::icon::DENSE,
            if selected {
                theme::accent(dark)
            } else {
                theme::text(dark)
            },
        )
        .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)));
        self.remember(id, &response, Some(glyph));
        response.clicked()
    }

    fn notice(&mut self, ui: &mut egui::Ui, text: Text, dark: bool, retry: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                egui::RichText::new(self.tr(text))
                    .size(theme::type_size::BODY)
                    .color(theme::secondary(dark)),
            );
            if retry
                && self.icon(
                    ui,
                    "sounds.retry",
                    Icon::ArrowClockwise,
                    Text::SoundsRetry,
                    false,
                )
            {
                self.actions.push(Action::SoundsRetry);
            }
        });
    }

    fn problem_banner(
        &mut self,
        ui: &mut egui::Ui,
        problem: Option<SoundsProblem>,
        dark: bool,
        retry: Option<&str>,
    ) {
        let Some(problem) = problem else {
            return;
        };
        let text = problem_text(self.cx.preferences.locale, problem);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                egui::RichText::new(text)
                    .size(theme::type_size::BODY)
                    .color(theme::error_text(dark)),
            );
            if let Some(id) = retry
                && self.icon(ui, id, Icon::ArrowClockwise, Text::SoundsRetry, false)
            {
                self.actions.push(Action::SoundsRetry);
            }
        });
    }

    /// A library store that did not open: its reason and a Retry row, in place of
    /// the section's list. Retry asks the lending owner to open the store again.
    pub(super) fn library_open_failure(
        &mut self,
        ui: &mut egui::Ui,
        store: LibraryStore,
        problem: LibraryProblem,
    ) {
        let dark = self.cx.preferences.dark;
        let (retry_id, retry_label) = match store {
            LibraryStore::Sounds => ("sounds.open.retry", Text::SoundsRetry),
            LibraryStore::Music => ("music.open.retry", Text::MusicRetry),
        };
        let reason = match problem {
            LibraryProblem::Busy => Text::LibraryBusy,
            LibraryProblem::Corrupt => Text::LibraryCorrupt,
            LibraryProblem::Io => Text::LibraryIo,
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                egui::RichText::new(self.tr(reason))
                    .size(theme::type_size::BODY)
                    .color(theme::error_text(dark)),
            );
            if self.icon(ui, retry_id, Icon::ArrowClockwise, retry_label, true) {
                self.actions.push(Action::LibraryRetry(store));
            }
        });
    }

    fn text_width(&self, text: Text) -> f32 {
        let characters = i18n::text(self.cx.preferences.locale, text).chars().count() as f32;
        characters * 6.0 + 8.0
    }
}

fn duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn problem_text(locale: i18n::Locale, problem: SoundsProblem) -> String {
    match problem {
        SoundsProblem::Auth => i18n::text(locale, Text::SoundsAuth).to_owned(),
        SoundsProblem::Quota => i18n::text(locale, Text::SoundsQuota).to_owned(),
        SoundsProblem::RateLimit(Some(seconds)) => i18n::format(
            locale,
            Text::SoundsRateLimit,
            &[("seconds", &i18n::number(locale, u64::from(seconds)))],
        ),
        SoundsProblem::RateLimit(None) => i18n::text(locale, Text::SoundsRateLimitSoon).to_owned(),
        SoundsProblem::Network => i18n::text(locale, Text::SoundsNetwork).to_owned(),
        SoundsProblem::Server => i18n::text(locale, Text::SoundsServer).to_owned(),
        SoundsProblem::Invalid => i18n::text(locale, Text::SoundsInvalid).to_owned(),
        SoundsProblem::Limit => i18n::text(locale, Text::SoundsLimit).to_owned(),
        SoundsProblem::NoAudioDevice => i18n::text(locale, Text::SoundsNoDevice).to_owned(),
        SoundsProblem::NotCached
        | SoundsProblem::Library
        | SoundsProblem::CacheBusy
        | SoundsProblem::CacheFailed
        | SoundsProblem::Publication => i18n::text(locale, Text::SoundsSaveFailed).to_owned(),
        SoundsProblem::Cancelled => i18n::text(locale, Text::SoundsCacheFailed).to_owned(),
    }
}
