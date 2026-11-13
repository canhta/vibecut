//! Renders searchable media rows with stable interaction identities.
use super::{
    Action, Align2, AssetRow, AssetState, AssetsTab, Color32, Filter, FontId, Icon, Id, MediaKind,
    MediaSortKey, MediaSortOrder, MediaView, Rect, Row, Text, Vec2, View, asset_state_color,
    display_short_time, egui, icons, theme,
};
use crate::i18n;
use media::AssetId;

impl View<'_> {
    pub(super) fn browser(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let toolbar_height = self.toolbar(ui);
        let filter = Rect::from_min_size(
            area.min + Vec2::new(0.0, toolbar_height + 12.0),
            Vec2::new(area.width(), 12.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.filter.row")
                .max_rect(filter),
            |ui| {
                ui.spacing_mut().interact_size.y = 12.0;
                ui.spacing_mut().button_padding = Vec2::ZERO;
                if self.cx.preferences.assets_tab == AssetsTab::Sounds {
                    ui.label(
                        egui::RichText::new(self.tr(Text::AssetsAll)).size(theme::type_size::BODY),
                    );
                    return;
                }
                let menu = ui.menu_button(
                    egui::RichText::new(self.tr(self.state.filter.label()))
                        .size(theme::type_size::BODY),
                    |ui| {
                        for filter in Filter::ALL {
                            let response = ui.selectable_label(
                                self.state.filter == filter,
                                self.tr(filter.label()),
                            );
                            self.remember(
                                format!("assets.filter.{}", filter.id()),
                                &response,
                                None,
                            );
                            if response.clicked() {
                                self.state.filter = filter;
                                ui.close();
                            }
                        }
                    },
                );
                self.remember("assets.filter", &menu.response, None);
            },
        );
        let search = self
            .state
            .searches
            .get(&self.cx.preferences.assets_tab)
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        let cx = self.cx;
        let rows: Vec<_> = cx
            .rows
            .iter()
            .filter(|row| {
                let kind = row.kind;
                let filter = if self.cx.preferences.assets_tab == AssetsTab::Sounds {
                    Filter::Audio
                } else {
                    self.state.filter
                };
                let matches = match filter {
                    Filter::All => true,
                    Filter::Video => kind == MediaKind::Video,
                    Filter::Audio => kind == MediaKind::Audio,
                    Filter::Image => kind == MediaKind::Image,
                };
                matches && row.display.name.to_lowercase().contains(&search)
            })
            .collect();
        // The strip is one control row plus the grid gap, measured by the toolbar tokens.
        let export_height = if self.cx.clip_export.is_some() {
            theme::size::CONTROL + theme::space::MD
        } else {
            0.0
        };
        if let Some(status) = self.cx.clip_export {
            let strip = Rect::from_min_size(
                area.min + Vec2::new(0.0, toolbar_height + 36.0),
                Vec2::new(area.width(), theme::size::CONTROL),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt("media.export.strip")
                    .max_rect(strip),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::ClipExportRunning))
                                .size(theme::type_size::BODY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.icon(
                                ui,
                                "media.export.cancel",
                                Icon::X,
                                Text::Cancel,
                                !status.cancelling,
                            ) {
                                self.actions.push(Action::ClipExportCancel);
                            }
                            let width = ui.available_width();
                            ui.add(egui::ProgressBar::new(status.progress).desired_width(width));
                        });
                    });
                },
            );
        }
        let results = Rect::from_min_max(
            area.min + Vec2::new(0.0, toolbar_height + 36.0 + export_height),
            area.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.results")
                .max_rect(results),
            |ui| {
                if rows.is_empty() {
                    self.empty(
                        ui,
                        if self.cx.failed {
                            Icon::X
                        } else if self.cx.importing {
                            Icon::UploadSimple
                        } else {
                            Icon::FolderOpen
                        },
                        if self.cx.failed {
                            Text::AssetsImportError
                        } else if self.cx.importing {
                            Text::Importing
                        } else if search.is_empty() {
                            Text::AssetsNoMedia
                        } else {
                            Text::AssetsNoMatches
                        },
                    );
                    if !search.is_empty() && !self.cx.failed && !self.cx.importing {
                        self.clear_search(ui);
                    }
                } else {
                    self.grid(ui, &rows);
                }
                if self.cx.failed && !rows.is_empty() {
                    ui.label(
                        egui::RichText::new(self.tr(Text::AssetsImportError))
                            .size(theme::type_size::BODY)
                            .color(theme::error_text(self.cx.preferences.dark)),
                    );
                }
            },
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) -> f32 {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(24.0);
        let media = self.cx.preferences.assets_tab == AssetsTab::Media;
        if !media {
            let row = Rect::from_min_size(ui.cursor().min, Vec2::new(width, theme::size::CONTROL));
            let search = Rect::from_min_size(
                row.min,
                Vec2::new((width - 41.0).max(24.0), theme::size::CONTROL),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt("audio.search")
                    .max_rect(search),
                |ui| self.search(ui, search.width()),
            );
            let sort = Rect::from_min_size(
                egui::pos2(row.right() - 36.0, row.top()),
                Vec2::new(36.0, theme::size::CONTROL),
            );
            ui.scope_builder(
                egui::UiBuilder::new().id_salt("audio.sort").max_rect(sort),
                |ui| self.sort_menu(ui),
            );
            ui.advance_cursor_after_rect(row);
            return theme::size::CONTROL;
        }

        let compact = width < 404.0;
        let three_rows = width < 260.0;
        let height = if three_rows {
            88.0
        } else if compact {
            56.0
        } else {
            24.0
        };
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let importing = self.cx.importing;
        {
            let rect = Rect::from_min_size(row.min, Vec2::new(72.0, 24.0));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt("media.import.tool")
                    .max_rect(rect),
                |ui| {
                    let response = ui.add_enabled(
                        !importing,
                        egui::Button::image_and_text(
                            icons::tinted(
                                Icon::UploadSimple,
                                theme::icon::DENSE,
                                theme::text(self.cx.preferences.dark),
                            ),
                            egui::RichText::new(self.tr(Text::Import)).size(theme::type_size::BODY),
                        )
                        .gap(theme::space::ICON_LABEL)
                        .stroke(egui::Stroke::NONE)
                        .min_size(rect.size()),
                    );
                    crate::kit::dashed_outline(
                        ui,
                        response.rect,
                        theme::radius::CONTROL,
                        response.has_focus(),
                    );
                    self.remember("file.import", &response, Some(Icon::UploadSimple));
                    if response.clicked() {
                        self.actions.push(Action::Command("file.import".into()));
                    }
                },
            );
            let rect = Rect::from_min_size(
                row.min + Vec2::new(80.0, 0.0),
                Vec2::new(if width < 150.0 { 24.0 } else { 62.0 }, 24.0),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt("media.text.tool")
                    .max_rect(rect),
                |ui| {
                    let response = ui
                        .add_enabled(
                            !importing,
                            egui::Button::image(icons::tinted(
                                Icon::TextT,
                                16.0,
                                theme::text(self.cx.preferences.dark),
                            ))
                            .min_size(rect.size()),
                        )
                        .on_hover_text(self.tr(Text::TextItem));
                    self.remember("text.insert", &response, Some(Icon::TextT));
                    if response.clicked() {
                        self.actions.push(Action::Command("text.insert".into()));
                    }
                },
            );
        }
        let search = Rect::from_min_size(
            row.min
                + Vec2::new(
                    if compact { 0.0 } else { 150.0 },
                    if compact { 32.0 } else { 0.0 },
                ),
            Vec2::new(width.min(148.0), 24.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.search.tool")
                .max_rect(search),
            |ui| self.search(ui, search.width()),
        );
        let offset = if three_rows {
            0.0
        } else if compact {
            150.0
        } else {
            316.0
        };
        let positions = if compact {
            [(offset, 24.0), (offset + 32.0, 32.0), (offset + 72.0, 24.0)]
        } else {
            [(offset, 24.0), (offset + 34.0, 32.0), (offset + 79.0, 24.0)]
        };
        for (index, (x, width)) in positions.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                row.min + Vec2::new(x, if three_rows { 64.0 } else { 0.0 }),
                Vec2::new(width, 24.0),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt(("media.tool", index))
                    .max_rect(rect),
                |ui| match index {
                    0 => {
                        let (glyph, label) = if self.cx.preferences.media_view == MediaView::Grid {
                            (Icon::SquaresFour, Text::SwitchToList)
                        } else {
                            (Icon::ListBullets, Text::SwitchToGrid)
                        };
                        if self.icon(ui, "media.view", glyph, label, !importing) {
                            self.actions.push(Action::Command("media.view".into()));
                        }
                    }
                    1 => self.sort_menu(ui),
                    _ => {
                        if self.icon(
                            ui,
                            "media.insert",
                            Icon::Plus,
                            Text::AddToTimeline,
                            !importing && !self.cx.selection.is_empty(),
                        ) {
                            self.actions.push(Action::Command("media.insert".into()));
                        }
                    }
                },
            );
        }
        height
    }

    fn sort_menu(&mut self, ui: &mut egui::Ui) {
        let menu = ui
            .add_enabled_ui(!self.cx.importing, |ui| {
                ui.spacing_mut().interact_size.y = theme::size::CONTROL;
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                egui::ComboBox::from_id_salt(Id::unique("media.sort"))
                    .selected_text("")
                    .width(32.0)
                    .show_ui(ui, |ui| {
                        for key in MediaSortKey::ALL {
                            let text = match key {
                                MediaSortKey::Name => Text::SortName,
                                MediaSortKey::Type => Text::SortType,
                                MediaSortKey::Duration => Text::SortDuration,
                                MediaSortKey::Size => Text::SortSize,
                            };
                            let response = ui.selectable_label(
                                key == self.cx.preferences.media_sort.key,
                                self.tr(text),
                            );
                            self.remember(format!("media.sort.{}", key.id()), &response, None);
                            if response.clicked() {
                                self.actions
                                    .push(Action::Command(format!("media.sort.{}", key.id())));
                                ui.close();
                            }
                        }
                    })
            })
            .inner;
        let order = self.tr(match self.cx.preferences.media_sort.order {
            MediaSortOrder::Asc => Text::SortAscending,
            MediaSortOrder::Desc => Text::SortDescending,
        });
        let key = self.tr(match self.cx.preferences.media_sort.key {
            MediaSortKey::Name => Text::SortName,
            MediaSortKey::Type => Text::SortType,
            MediaSortKey::Duration => Text::SortDuration,
            MediaSortKey::Size => Text::SortSize,
        });
        let response = menu.response.on_hover_text(i18n::format(
            self.cx.preferences.locale,
            Text::SortTooltip,
            &[("key", key), ("order", order)],
        ));
        icons::tinted(
            Icon::ListBullets,
            16.0,
            theme::text(self.cx.preferences.dark),
        )
        .paint_at(
            ui,
            Rect::from_center_size(
                egui::pos2(response.rect.left() + 10.0, response.rect.center().y),
                Vec2::splat(16.0),
            ),
        );
        self.remember("media.sort", &response, Some(Icon::ListBullets));
    }

    fn grid(&mut self, ui: &mut egui::Ui, rows: &[&Row]) {
        let list = self.cx.preferences.media_view == MediaView::List
            || self.cx.preferences.assets_tab == AssetsTab::Sounds;
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(0.0);
        let columns = if list {
            1
        } else {
            ((width + theme::space::GRID) / (theme::size::MEDIA_THUMB_WIDTH + theme::space::GRID))
                .floor()
                .max(1.0) as usize
        };
        let cell = if list {
            Vec2::new(width, theme::size::AUDIO_ROW)
        } else {
            Vec2::new(
                theme::size::MEDIA_THUMB_WIDTH,
                theme::size::MEDIA_THUMB_HEIGHT + 6.0 + theme::size::MEDIA_LABEL_HEIGHT,
            )
        };
        let pitch = if list {
            60.0
        } else {
            cell.y + theme::space::GRID
        };
        let viewport = (ui.available_height() - if self.cx.failed { 30.0 } else { 0.0 }).max(32.0);
        egui::ScrollArea::vertical()
            .id_salt(if list { "media.list" } else { "media.grid" })
            .max_height(viewport)
            .show(ui, |ui| {
                let (content, _) = ui.allocate_exact_size(
                    Vec2::new(
                        width,
                        (rows.len().div_ceil(columns) as f32 * pitch).max(viewport),
                    ),
                    egui::Sense::hover(),
                );
                let background = ui.interact(
                    content,
                    Id::unique("media.marquee"),
                    egui::Sense::click_and_drag(),
                );
                self.remember("media.marquee", &background, None);
                let mut rects = Vec::new();
                for (index, row) in rows.iter().enumerate() {
                    let rect = Rect::from_min_size(
                        content.min
                            + Vec2::new(
                                (index % columns) as f32 * (cell.x + theme::space::GRID),
                                (index / columns) as f32 * pitch,
                            ),
                        cell,
                    );
                    self.asset(ui, &row.display, rect, list);
                    if self.cx.reveal == Some(row.display.id) {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    rects.push((row.display.id, rect));
                }
                if let Some(marquee) = &self.cx.marquee
                    && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
                {
                    let rect = Rect::from_two_pos(marquee.origin, pointer).intersect(content);
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        theme::accent(self.cx.preferences.dark).gamma_multiply(0.15),
                    );
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.0, theme::accent(self.cx.preferences.dark)),
                        egui::StrokeKind::Inside,
                    );
                }
                self.actions.push(Action::Marquee {
                    content,
                    background: Box::new(background),
                    rects,
                });
            });
    }

    /// The empty-results clear action: presentation only, the project is untouched.
    fn clear_search(&mut self, ui: &mut egui::Ui) {
        // Sits one gap below the empty state's label, which `View::empty` places at 36% + 32.
        let area = ui.available_rect_before_wrap();
        let top =
            area.top() + area.height() * 0.36 + 32.0 + theme::type_size::BODY + theme::space::MD;
        let row = Rect::from_min_size(
            egui::pos2(area.left(), top),
            Vec2::new(area.width(), theme::size::CONTROL),
        );
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
                ui.vertical_centered(|ui| {
                    ui.button(
                        egui::RichText::new(self.tr(Text::AssetsClearSearch))
                            .size(theme::type_size::BODY),
                    )
                })
                .inner
            })
            .inner;
        self.remember("media.search.clear", &response, None);
        if response.clicked() {
            self.state.searches.remove(&AssetsTab::Media);
        }
    }
    /// Why the context menu cannot export `targets`; `None` when it can.
    fn clip_export_refusal(&self, targets: &[AssetId]) -> Option<Text> {
        if self.cx.clip_export.is_some() {
            return Some(Text::ClipExportBusy);
        }
        let video_only = !targets.is_empty()
            && targets.iter().all(|id| {
                self.cx
                    .rows
                    .iter()
                    .any(|row| row.display.id == *id && row.kind == MediaKind::Video)
            });
        (!video_only).then_some(Text::ClipExportNeedsVideo)
    }
    fn asset(&mut self, ui: &mut egui::Ui, row: &AssetRow, rect: Rect, list: bool) {
        let response = ui.interact(
            rect,
            Id::unique(("media.asset", row.id.0)),
            egui::Sense::click(),
        );
        self.remember(
            format!("media.asset.{}", row.id.0),
            &response,
            Some(row.icon),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, !self.cx.importing, &row.name)
        });
        if response.clicked() {
            self.actions
                .push(Action::Select(row.id, ui.input(|input| input.modifiers)));
        }
        if response.secondary_clicked() && !self.cx.selection.contains(&row.id) {
            self.actions
                .push(Action::Select(row.id, egui::Modifiers::NONE));
        }
        let selected = self.cx.selection.contains(&row.id);
        let dark = self.cx.preferences.dark;
        if list {
            ui.painter().rect_filled(
                rect,
                theme::radius::UPLOAD,
                if selected || response.hovered() {
                    theme::hover(dark)
                } else {
                    theme::field(dark)
                },
            );
        }
        let thumb = if list {
            Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::splat(32.0))
        } else {
            Rect::from_min_size(
                rect.min,
                Vec2::new(rect.width(), theme::size::MEDIA_THUMB_HEIGHT),
            )
        };
        ui.painter().rect_filled(thumb, 4.0, theme::field(dark));
        if let Some(texture) = self.cx.thumbnails.get(&row.id) {
            let scale =
                (thumb.width() / texture.size_vec2().x).min(thumb.height() / texture.size_vec2().y);
            let image = Rect::from_center_size(thumb.center(), texture.size_vec2() * scale);
            let response = ui.put(
                image,
                egui::Image::new((texture.id(), image.size()))
                    .corner_radius(theme::radius::CONTROL),
            );
            self.remember(
                format!("media.asset.{}.thumbnail", row.id.0),
                &response,
                None,
            );
        } else {
            icons::tinted(row.icon, 24.0, theme::secondary(dark)).paint_at(
                ui,
                Rect::from_center_size(thumb.center(), Vec2::splat(24.0)),
            );
        }
        if selected || self.cx.reveal == Some(row.id) || response.hovered() {
            ui.painter().rect_stroke(
                thumb,
                4.0,
                egui::Stroke::new(1.0, theme::accent(dark)),
                egui::StrokeKind::Inside,
            );
        }
        if !list && let Some(duration) = row.duration {
            let galley = ui.painter().layout_no_wrap(
                display_short_time(duration),
                FontId::proportional(theme::type_size::BODY),
                theme::on_scrim(),
            );
            let origin = thumb.right_top() - Vec2::new(galley.size().x + 4.0, -2.0);
            ui.painter().rect_filled(
                Rect::from_min_size(origin, galley.size()),
                2.0,
                theme::scrim(),
            );
            ui.painter().galley(origin, galley, theme::on_scrim());
        }
        let name = if list {
            egui::pos2(thumb.right() + 8.0, rect.top() + 8.0)
        } else {
            rect.min + Vec2::new(0.0, theme::size::MEDIA_THUMB_HEIGHT + 6.0)
        };
        let painter = ui.painter().with_clip_rect(rect);
        painter.text(
            name,
            Align2::LEFT_TOP,
            &row.name,
            if list {
                theme::label_font()
            } else {
                FontId::proportional(theme::type_size::BODY)
            },
            theme::text(dark),
        );
        if list {
            painter.text(
                name + Vec2::new(0.0, 16.0),
                Align2::LEFT_TOP,
                row.duration
                    .map(display_short_time)
                    .unwrap_or_else(|| self.tr(Text::DurationUnknown).to_owned()),
                FontId::proportional(theme::type_size::BODY),
                theme::secondary(dark),
            );
        }
        let status_rect = Rect::from_min_size(thumb.min + Vec2::splat(2.0), Vec2::splat(16.0));
        let status = ui.interact(
            status_rect,
            Id::unique(("media.status", row.id.0)),
            egui::Sense::hover(),
        );
        self.remember(format!("media.asset.{}.status", row.id.0), &status, None);
        if row.state != AssetState::Ready {
            let glyph = if row.state == AssetState::Probing {
                Icon::ArrowClockwise
            } else {
                Icon::X
            };
            icons::tinted(glyph, 12.0, asset_state_color(row.state, dark))
                .paint_at(ui, status_rect);
            status.on_hover_text(if row.state == AssetState::Probing {
                self.tr(Text::Importing)
            } else {
                self.tr(Text::AssetsImportError)
            });
        }
        if self.cx.preferences.assets_tab == AssetsTab::Sounds {
            let add = Rect::from_center_size(
                egui::pos2(rect.right() - 16.0, rect.center().y),
                Vec2::splat(24.0),
            );
            let id = format!("media.asset.{}.add", row.id.0);
            let clicked = ui
                .scope_builder(egui::UiBuilder::new().id_salt(&id).max_rect(add), |ui| {
                    self.icon(ui, &id, Icon::Plus, Text::AddToTimeline, !self.cx.importing)
                })
                .inner;
            if clicked {
                self.actions
                    .push(Action::Select(row.id, egui::Modifiers::NONE));
                self.actions.push(Action::Command("media.insert".into()));
            }
        }
        response.on_hover_text(&row.info).context_menu(|ui| {
            let targets: Vec<AssetId> = if selected {
                self.cx.selection.iter().copied().collect()
            } else {
                vec![row.id]
            };
            let refusal = self.clip_export_refusal(&targets);
            let export = ui.add_enabled(
                refusal.is_none() && !self.cx.importing,
                egui::Button::image_and_text(
                    icons::tinted(Icon::Export, 16.0, theme::text(self.cx.preferences.dark)),
                    self.tr(Text::ExportClips),
                ),
            );
            let export = match refusal {
                Some(text) => export.on_disabled_hover_text(self.tr(text)),
                None => export,
            };
            self.remember("media.export_clip", &export, Some(Icon::Export));
            if export.clicked() {
                self.actions.push(Action::ExportClips(targets));
                ui.close();
            }
            let mut button = egui::Button::new(if selected && self.cx.selection.len() > 1 {
                i18n::format_count(
                    self.cx.preferences.locale,
                    self.cx.selection.len() as u64,
                    Text::DeleteItemsOther,
                    Text::DeleteItemsOther,
                )
            } else {
                self.tr(Text::DeleteItemOne).to_owned()
            });
            if let Some(shortcut) = self.cx.shortcut("media.delete") {
                button = button.shortcut_text(shortcut);
            }
            let response = ui.add_enabled(!self.cx.importing, button);
            self.remember("media.delete", &response, Some(Icon::Trash));
            if response.clicked() {
                let ids = if selected {
                    self.cx.selection.iter().copied().collect()
                } else {
                    vec![row.id]
                };
                self.actions.push(Action::Delete(ids));
                ui.close();
            }
        });
    }
}
