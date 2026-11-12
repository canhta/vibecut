//! Renders the Audio tab's Music section: chips, results and saved tracks.
use super::super::AudioSection;
use super::super::music::{MusicMode, MusicProblem, MusicView};
use super::{
    Action, Align2, Color32, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons, theme,
};
use crate::i18n;

impl View<'_> {
    /// Section switch shared by the Music and Sound-effects pages.
    pub(super) fn audio_nav(&mut self, ui: &mut egui::Ui) {
        for (section, label, id) in [
            (AudioSection::Music, Text::Music, "audio.section.music"),
            (
                AudioSection::Sounds,
                Text::MusicSoundEffects,
                "audio.section.sounds",
            ),
        ] {
            let selected = self.state.audio_section == section;
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
            self.remember(id, &response, None);
            if response.clicked() && !selected {
                self.state.audio_section = section;
            }
        }
        ui.add_space(8.0);
        match self.state.audio_section {
            AudioSection::Music => self.music_nav(ui),
            AudioSection::Sounds => self.sounds_nav(ui),
        }
    }

    fn music_nav(&mut self, ui: &mut egui::Ui) {
        let saved = self.tr(Text::MusicSaved).to_owned();
        if self.nav_button(
            ui,
            "music.tab.saved",
            &saved,
            self.state.music_mode == MusicMode::Saved,
        ) {
            self.state.music_mode = MusicMode::Saved;
            self.actions.push(Action::MusicMode(MusicMode::Saved));
        }
        for chip in libraries::music::CHIPS {
            if self.nav_button(
                ui,
                &format!("music.chip.{}", chip.replace(' ', "-")),
                chip,
                self.state.music_mode == MusicMode::Browse && self.state.music_chip == chip,
            ) {
                self.state.music_chip = chip;
                self.state.music_mode = MusicMode::Browse;
                self.state.music_search.clear();
                self.actions.push(Action::MusicChip(chip));
            }
        }
    }

    fn nav_button(&mut self, ui: &mut egui::Ui, id: &str, label: &str, selected: bool) -> bool {
        let response = ui.add_sized(
            [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
            egui::Button::new(
                egui::RichText::new(label)
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
        self.remember(id, &response, None);
        response.clicked()
    }

    pub(super) fn music(&mut self, ui: &mut egui::Ui) {
        let credits = self.cx.music_credits.clone();
        let credits_failed = self.cx.music_credits_failed;
        let Some(view) = self.cx.music.clone() else {
            self.music_credits(ui, &credits, credits_failed);
            self.empty(ui, Icon::MusicNotes, Text::MusicUnavailable);
            return;
        };
        let dark = self.cx.preferences.dark;
        let area = ui.available_rect_before_wrap();
        let width = area.width();
        let row = Rect::from_min_size(area.min, Vec2::new(width, theme::size::CONTROL));
        let field_width = (width - 64.0).max(72.0);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("music.search.row")
                .max_rect(Rect::from_min_size(
                    row.min,
                    Vec2::new(field_width, row.height()),
                )),
            |ui| self.music_search(ui, field_width),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("music.search.go")
                .max_rect(Rect::from_min_size(
                    row.min + Vec2::new(field_width + 4.0, 0.0),
                    Vec2::splat(theme::size::CONTROL),
                )),
            |ui| {
                if self.icon(
                    ui,
                    "music.search.go",
                    Icon::MagnifyingGlass,
                    Text::MusicSearch,
                    true,
                ) {
                    self.actions.push(Action::MusicSearch { more: false });
                }
            },
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("music.settings.toggle")
                .max_rect(Rect::from_min_size(
                    row.min + Vec2::new(field_width + 32.0, 0.0),
                    Vec2::splat(theme::size::CONTROL),
                )),
            |ui| {
                if self.icon(ui, "music.settings", Icon::GearSix, Text::MusicSettings, true) {
                    self.state.music_settings = !self.state.music_settings;
                }
            },
        );
        ui.advance_cursor_after_rect(row);
        ui.add_space(8.0);

        if self.state.music_settings {
            self.music_settings(ui, &view);
        }
        self.music_credits(ui, &credits, credits_failed);

        self.music_problem_banner(ui, view.search_problem, dark, Some("music.retry"), false);
        self.music_problem_banner(
            ui,
            view.cache_problem,
            dark,
            Some("music.retry.cache"),
            true,
        );
        self.music_problem_banner(ui, view.preview_problem, dark, None, false);
        self.music_problem_banner(ui, view.saved_problem, dark, None, false);
        if view.searching {
            ui.label(
                egui::RichText::new(self.tr(Text::MusicSearching))
                    .size(theme::type_size::BODY)
                    .color(theme::secondary(dark)),
            );
        }
        if let Some(cache) = &view.cache {
            let percent = (cache.progress * 100.0).round().clamp(0.0, 100.0) as u32;
            let text = i18n::format(
                self.cx.preferences.locale,
                Text::MusicCaching,
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
                    "music.cancel_cache.banner",
                    Icon::X,
                    Text::MusicCancelCache,
                    cache.cancelling,
                ) {
                    self.actions.push(Action::MusicCancelCache);
                }
            });
        }

        let list = view.results.clone();
        if list.is_empty() && !view.searching {
            self.empty(
                ui,
                Icon::MusicNotes,
                if view.mode == MusicMode::Saved {
                    Text::MusicNoSaved
                } else {
                    Text::MusicNoResults
                },
            );
            return;
        }
        ui.horizontal(|ui| {
            let count = i18n::format(
                self.cx.preferences.locale,
                Text::MusicResultCount,
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
        });
        let viewport = ui.available_height().max(48.0);
        egui::ScrollArea::vertical()
            .id_salt("music.list")
            .max_height(viewport)
            .show(ui, |ui| {
                // Measured audio rows are 48 px on an explicit 60 px pitch.
                ui.spacing_mut().item_spacing.y = 12.0;
                for music in &list {
                    self.music_row(ui, music, &view);
                }
                if view.has_next
                    && self.icon(ui, "music.more", Icon::Plus, Text::MusicLoadMore, true)
                {
                    self.actions.push(Action::MusicSearch { more: true });
                }
            });
    }

    fn music_settings(&mut self, ui: &mut egui::Ui, view: &MusicView) {
        let dark = self.cx.preferences.dark;
        ui.horizontal(|ui| {
            let hint = self.tr(Text::MusicKey);
            let width = ui.available_width().clamp(96.0, 240.0);
            let response = ui.add_sized(
                [width, theme::size::CONTROL],
                egui::TextEdit::singleline(&mut self.state.music_key)
                    .password(true)
                    .char_limit(4096)
                    .hint_text(hint)
                    .font(egui::FontId::proportional(theme::type_size::BODY)),
            );
            self.remember("music.key", &response, None);
            if self.icon(
                ui,
                "music.key.apply",
                Icon::FloppyDisk,
                Text::MusicKeyApply,
                !self.state.music_key.is_empty(),
            ) {
                self.actions.push(Action::MusicKeyApply);
            }
            if self.icon(
                ui,
                "music.key.remove",
                Icon::X,
                Text::MusicKeyRemove,
                view.key_set,
            ) {
                self.actions.push(Action::MusicKeyRemove);
            }
        });
        ui.label(
            egui::RichText::new(self.tr(Text::MusicKeyScope))
                .size(theme::type_size::SMALL_LABEL)
                .color(theme::secondary(dark)),
        );
        ui.label(
            egui::RichText::new(self.tr(if view.key_set {
                Text::MusicKeySet
            } else {
                Text::MusicKeyNotSet
            }))
            .size(theme::type_size::SMALL_LABEL)
            .color(theme::secondary(dark)),
        );
        let terms = ui.hyperlink_to(
            self.tr(Text::MusicTerms),
            "https://docs.openverse.org/terms_of_service.html",
        );
        self.remember("music.terms", &terms, Some(Icon::LinkSimple));
        ui.add_space(4.0);
    }

    /// Project-derived credits, shown before any empty or unavailable list.
    fn music_credits(&mut self, ui: &mut egui::Ui, credits: &[String], failed: bool) {
        let dark = self.cx.preferences.dark;
        ui.horizontal(|ui| {
            let glyph = if self.state.music_credits_open {
                Icon::CaretDown
            } else {
                Icon::CaretRight
            };
            if self.icon(ui, "music.credits.toggle", glyph, Text::MusicCredits, true) {
                self.state.music_credits_open = !self.state.music_credits_open;
            }
            if self.icon(
                ui,
                "music.credits",
                Icon::Copy,
                Text::MusicCopyCredits,
                !credits.is_empty() && !failed,
            ) {
                ui.ctx().copy_text(credits.join("\n"));
                self.actions.push(Action::MusicCredits);
            }
            let note = if failed {
                Some(Text::MusicCreditsFailed)
            } else if credits.is_empty() {
                Some(Text::MusicNoCredits)
            } else {
                None
            };
            if let Some(note) = note {
                ui.label(
                    egui::RichText::new(self.tr(note))
                        .size(theme::type_size::SMALL_LABEL)
                        .color(theme::secondary(dark)),
                );
            }
        });
        if self.state.music_credits_open {
            egui::ScrollArea::vertical()
                .id_salt("music.credits.list")
                .max_height(120.0)
                .show(ui, |ui| {
                    for record in credits {
                        ui.label(
                            egui::RichText::new(record.as_str())
                                .size(theme::type_size::SMALL_LABEL)
                                .color(theme::secondary(dark)),
                        );
                    }
                });
        }
    }

    fn music_search(&mut self, ui: &mut egui::Ui, width: f32) {
        let hint = self.tr(Text::MusicSearch);
        let draft = &mut self.state.music_search;
        let response = ui.add_sized(
            [width.max(24.0), theme::size::CONTROL],
            egui::TextEdit::singleline(draft)
                .char_limit(200)
                .hint_text(hint)
                .font(egui::FontId::proportional(theme::type_size::BODY)),
        );
        self.remember("music.search", &response, None);
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.actions.push(Action::MusicSearch { more: false });
        }
    }

    fn music_row(
        &mut self,
        ui: &mut egui::Ui,
        music: &libraries::music::MusicResult,
        view: &MusicView,
    ) {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(48.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(width, theme::size::AUDIO_ROW),
            egui::Sense::hover(),
        );
        let dark = self.cx.preferences.dark;
        let cached = view
            .cache
            .as_ref()
            .is_some_and(|cache| cache.id == music.id);
        let saved = view.saved_ids.contains(&music.id);
        let previewing = view.previewing.as_deref() == Some(music.id.as_str());
        ui.painter().rect_filled(
            rect,
            theme::radius::CONTROL,
            if response.hovered() || previewing {
                theme::hover(dark)
            } else {
                theme::field(dark)
            },
        );
        self.remember(
            format!("music.result.{}", music.id),
            &response,
            Some(Icon::MusicNotes),
        );
        let label = format!(
            "{} — {} — {} — {} — {}",
            music.title, music.author, music.licence, music.attribution, music.source_url
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
        let _ = response.on_hover_text(label.as_str());
        let thumb = Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::splat(32.0));
        ui.painter().rect_filled(thumb, 4.0, theme::field(dark));
        icons::tinted(Icon::MusicNotes, 20.0, theme::secondary(dark)).paint_at(
            ui,
            Rect::from_center_size(thumb.center(), Vec2::splat(20.0)),
        );
        let overlay = Rect::from_center_size(thumb.center(), Vec2::splat(24.0));
        if self.music_button(
            ui,
            overlay,
            &format!("music.preview.{}", music.id),
            if previewing { Icon::Pause } else { Icon::Play },
            if previewing {
                Text::MusicStopPreview
            } else {
                Text::MusicPreview
            },
            previewing,
        ) {
            self.actions.push(Action::MusicPreview(music.id.clone()));
        }
        let buttons = 24.0 * 2.0 + 8.0;
        let text_width = (rect.width() - 8.0 - 32.0 - 8.0 - buttons - 8.0).max(24.0);
        let clip = Rect::from_min_size(
            egui::pos2(thumb.right() + 8.0, rect.top() + 3.0),
            Vec2::new(text_width, rect.height() - 6.0),
        );
        let painter = ui.painter().with_clip_rect(clip);
        painter.text(
            clip.min,
            Align2::LEFT_TOP,
            &music.title,
            theme::label_font(),
            theme::text(dark),
        );
        let meta = i18n::format(
            self.cx.preferences.locale,
            Text::SoundsMeta,
            &[
                ("author", &music.author),
                ("duration", &duration(music.duration_ms)),
                ("licence", &music.licence),
            ],
        );
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
            &music.attribution,
            FontId::proportional(theme::type_size::SMALL_LABEL),
            theme::secondary(dark),
        );
        if cached && let Some(cache) = &view.cache {
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
        let start = rect.right() - 8.0 - 24.0 * 2.0 - 8.0;
        let save =
            Rect::from_min_size(egui::pos2(start, rect.center().y - 12.0), Vec2::splat(24.0));
        if self.music_button(
            ui,
            save,
            &format!("music.save.{}", music.id),
            Icon::BookmarkSimple,
            if saved {
                Text::MusicUnsave
            } else {
                Text::MusicSave
            },
            saved,
        ) {
            self.actions
                .push(Action::MusicToggleSaved(music.id.clone()));
        }
        let add = Rect::from_min_size(
            egui::pos2(start + 28.0, rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        if cached {
            if self.music_button(
                ui,
                add,
                "music.cancel_cache",
                Icon::X,
                Text::MusicCancelCache,
                true,
            ) {
                self.actions.push(Action::MusicCancelCache);
            }
        } else if self.music_button(
            ui,
            add,
            &format!("music.add.{}", music.id),
            Icon::Plus,
            Text::MusicAdd,
            false,
        ) {
            self.actions.push(Action::MusicAdd(music.id.clone()));
        }
    }

    fn music_button(
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

    fn music_problem_banner(
        &mut self,
        ui: &mut egui::Ui,
        problem: Option<MusicProblem>,
        dark: bool,
        retry: Option<&str>,
        retry_add: bool,
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
                && self.icon(ui, id, Icon::ArrowClockwise, Text::MusicRetry, true)
            {
                self.actions.push(if retry_add {
                    Action::MusicRetryAdd
                } else {
                    Action::MusicRetry
                });
            }
        });
    }
}

fn duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn problem_text(locale: i18n::Locale, problem: MusicProblem) -> String {
    match problem {
        MusicProblem::Auth => i18n::text(locale, Text::MusicAuth).to_owned(),
        MusicProblem::Quota => i18n::text(locale, Text::MusicQuota).to_owned(),
        MusicProblem::RateLimit(Some(seconds)) => i18n::format(
            locale,
            Text::MusicRateLimit,
            &[("seconds", &i18n::number(locale, u64::from(seconds)))],
        ),
        MusicProblem::RateLimit(None) => i18n::text(locale, Text::MusicRateLimitSoon).to_owned(),
        MusicProblem::Network => i18n::text(locale, Text::MusicNetwork).to_owned(),
        MusicProblem::Server => i18n::text(locale, Text::MusicServer).to_owned(),
        MusicProblem::Invalid => i18n::text(locale, Text::MusicInvalid).to_owned(),
        MusicProblem::Limit => i18n::text(locale, Text::MusicLimit).to_owned(),
        MusicProblem::NoAudioDevice => i18n::text(locale, Text::MusicNoDevice).to_owned(),
        MusicProblem::NotCached
        | MusicProblem::Library
        | MusicProblem::CacheBusy
        | MusicProblem::CacheFailed
        | MusicProblem::Publication => i18n::text(locale, Text::MusicSaveFailed).to_owned(),
        MusicProblem::Cancelled => i18n::text(locale, Text::MusicCacheFailed).to_owned(),
    }
}
