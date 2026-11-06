//! Renders the media browser.

use crate::editor::assets::{
    Align2, AssetId, AssetRow, AssetState, BTreeMap, BUTTON_ICON, Color32, Editor, FontId, Icon,
    Id, MEDIA_CARD_GAP, MEDIA_CARD_HEIGHT, MEDIA_CARD_WIDTH, MEDIA_ROW_HEIGHT, MediaMarquee,
    MediaSortKey, MediaSortOrder, MediaView, Rect, SourceAsset, Text, Vec2, asset_row,
    asset_state_color, asset_state_text, catalog, display_short_time, egui, file_name, icons,
    theme,
};

impl Editor {
    pub(in crate::editor) fn media_view(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Media))
                .size(16.0)
                .strong(),
        );
        ui.add_space(theme::space::MD);
        self.refresh_media_sizes();
        let importing = self.importing();
        self.media_toolbar(ui, importing);
        if self.button(ui, "text.insert", Icon::TextT, Text::TextItem, true) {
            self.action("text.insert");
        }
        // Reserve the line whether or not a selection exists so selecting an
        // asset never shifts the grid under a subsequent click.
        let count = if !importing && !self.media_selection.is_empty() {
            catalog::format_count(
                self.preferences.locale,
                self.media_selection.len() as u64,
                Text::SelectedCountOne,
                Text::SelectedCountOther,
            )
        } else {
            String::new()
        };
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::Label::new(
                egui::RichText::new(count)
                    .size(theme::type_size::CHROME)
                    .color(self.secondary()),
            ),
        );
        ui.add_space(theme::space::MD);
        let locale = self.preferences.locale;
        let order = self.media_order();
        let sizes = self.media_size_map();
        let by_id: BTreeMap<AssetId, &SourceAsset> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| (asset.id(), asset))
            .collect();
        let mut rows: Vec<AssetRow> = order
            .iter()
            .filter_map(|id| {
                by_id.get(id).map(|asset| {
                    let state = self
                        .asset_states
                        .get(id)
                        .copied()
                        .unwrap_or(AssetState::Probing);
                    asset_row(asset, locale, sizes.get(id).copied().flatten(), state)
                })
            })
            .collect();
        rows.extend(self.pending_imports.iter().map(|(id, path)| AssetRow {
            id: *id,
            name: file_name(path),
            info: asset_state_text(locale, AssetState::Probing),
            brief: asset_state_text(locale, AssetState::Probing),
            icon: Icon::UploadSimple,
            state: AssetState::Probing,
            duration: None,
        }));
        if rows.is_empty() {
            ui.label(self.tr(Text::EmptyMedia));
        } else {
            match self.preferences.media_view {
                MediaView::Grid => self.media_grid(ui, &rows, importing),
                MediaView::List => self.media_list(ui, &rows, importing),
            }
            if self.media_selection.is_empty() && !importing {
                ui.label(
                    egui::RichText::new(self.tr(Text::SelectAssets))
                        .small()
                        .color(self.secondary()),
                );
            }
        }
        if !self.import_failures.is_empty() {
            ui.separator();
            ui.label(
                egui::RichText::new(self.tr(Text::ImportIssues))
                    .size(theme::type_size::CHROME)
                    .strong(),
            );
            for failure in &self.import_failures {
                let name = file_name(&failure.path);
                ui.label(
                    egui::RichText::new(catalog::format(
                        self.preferences.locale,
                        Text::FailureDetail,
                        &[("name", &name), ("reason", &failure.reason)],
                    ))
                    .small()
                    .color(self.secondary()),
                );
            }
        }
        if let Some(notice) = &self.import_notice {
            ui.separator();
            ui.label(egui::RichText::new(notice).small().color(self.secondary()));
        }
    }
    pub(in crate::editor) fn media_toolbar(&mut self, ui: &mut egui::Ui, importing: bool) {
        ui.horizontal(|ui| {
            let (icon, label) = match self.preferences.media_view {
                MediaView::Grid => (Icon::ListBullets, Text::SwitchToList),
                MediaView::List => (Icon::SquaresFour, Text::SwitchToGrid),
            };
            if self.icon_only(
                ui,
                "media.view",
                icon,
                self.tr(label),
                None,
                None,
                !importing,
            ) {
                self.action("media.view");
            }
            ui.add_enabled_ui(!importing, |ui| self.media_sort_menu(ui));
            if self.button(
                ui,
                "file.import",
                Icon::UploadSimple,
                Text::Import,
                !importing,
            ) {
                self.action("file.import");
            }
            if self.icon_only(
                ui,
                "media.insert",
                Icon::Plus,
                self.tr(Text::AddToTimeline),
                None,
                None,
                !importing && !self.media_selection.is_empty(),
            ) {
                self.action("media.insert");
            }
        });
    }
    pub(in crate::editor) fn media_sort_label(&self, key: MediaSortKey) -> &'static str {
        self.tr(match key {
            MediaSortKey::Name => Text::SortName,
            MediaSortKey::Type => Text::SortType,
            MediaSortKey::Duration => Text::SortDuration,
            MediaSortKey::Size => Text::SortSize,
        })
    }
    pub(in crate::editor) fn media_sort_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.preferences.media_sort;
        let arrow = match current.order {
            MediaSortOrder::Asc => "↑",
            MediaSortOrder::Desc => "↓",
        };
        let selected = format!("{} {arrow}", self.media_sort_label(current.key));
        let combo = egui::ComboBox::from_id_salt(Id::unique("media.sort"))
            .selected_text(selected)
            .width(120.0)
            .show_ui(ui, |ui| {
                for key in MediaSortKey::ALL {
                    let active = current.key == key;
                    let arrow = if active { arrow } else { "" };
                    let response = ui.selectable_label(
                        active,
                        format!("{} {arrow}", self.media_sort_label(key)),
                    );
                    self.remember(format!("media.sort.{}", key.id()), &response);
                    if response.clicked() {
                        self.action(&format!("media.sort.{}", key.id()));
                        ui.close();
                    }
                }
            });
        let order = self.tr(match current.order {
            MediaSortOrder::Asc => Text::SortAscending,
            MediaSortOrder::Desc => Text::SortDescending,
        });
        let tooltip = catalog::format(
            self.preferences.locale,
            Text::SortTooltip,
            &[
                ("key", self.media_sort_label(current.key)),
                ("order", order),
            ],
        );
        let response = combo.response.on_hover_text(tooltip.clone());
        self.remember("media.sort", &response);
        if let Some(state) = self.widgets.get_mut("media.sort") {
            state.tooltip = Some(tooltip);
        }
    }
    pub(in crate::editor) fn media_viewport_height(&self, ui: &egui::Ui) -> f32 {
        let reserved = if self.import_failures.is_empty() && self.import_notice.is_none() {
            16.0
        } else {
            120.0
        };
        (ui.clip_rect().bottom() - ui.cursor().min.y - reserved).max(80.0)
    }
    pub(in crate::editor) fn media_grid(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[AssetRow],
        importing: bool,
    ) {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(0.0);
        let columns = ((width + MEDIA_CARD_GAP) / (MEDIA_CARD_WIDTH + MEDIA_CARD_GAP))
            .floor()
            .max(1.0) as usize;
        let content_height = rows.len().div_ceil(columns) as f32
            * (MEDIA_CARD_HEIGHT + MEDIA_CARD_GAP)
            - MEDIA_CARD_GAP;
        let viewport_height = self.media_viewport_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("media.grid")
            .max_height(viewport_height)
            .show(ui, |ui| {
                let (content, _) = ui.allocate_exact_size(
                    Vec2::new(width, content_height.max(viewport_height)),
                    egui::Sense::hover(),
                );
                let background = ui.interact(
                    content,
                    Id::unique("media.marquee"),
                    egui::Sense::click_and_drag(),
                );
                self.remember("media.marquee", &background);
                let mut rects = Vec::with_capacity(rows.len());
                for (index, row) in rows.iter().enumerate() {
                    let column = index % columns;
                    let line = index / columns;
                    let rect = Rect::from_min_size(
                        content.min
                            + Vec2::new(
                                column as f32 * (MEDIA_CARD_WIDTH + MEDIA_CARD_GAP),
                                line as f32 * (MEDIA_CARD_HEIGHT + MEDIA_CARD_GAP),
                            ),
                        Vec2::new(MEDIA_CARD_WIDTH, MEDIA_CARD_HEIGHT),
                    );
                    let response = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            self.media_asset(ui, row, importing, true)
                        })
                        .inner;
                    if self
                        .media_reveal
                        .is_some_and(|reveal| reveal.asset == row.id)
                    {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        let modifiers = ui.ctx().input(|input| input.modifiers);
                        self.select_asset(row.id, modifiers);
                    }
                    rects.push((row.id, rect));
                }
                self.media_marquee(ui, content, &background, &rects);
            });
    }
    pub(in crate::editor) fn media_list(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[AssetRow],
        importing: bool,
    ) {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(0.0);
        let viewport_height = self.media_viewport_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("media.list")
            .max_height(viewport_height)
            .show(ui, |ui| {
                let (content, _) = ui.allocate_exact_size(
                    Vec2::new(
                        width,
                        (rows.len() as f32 * MEDIA_ROW_HEIGHT).max(viewport_height),
                    ),
                    egui::Sense::hover(),
                );
                let background = ui.interact(
                    content,
                    Id::unique("media.marquee"),
                    egui::Sense::click_and_drag(),
                );
                self.remember("media.marquee", &background);
                let mut rects = Vec::with_capacity(rows.len());
                for (index, row) in rows.iter().enumerate() {
                    let rect = Rect::from_min_size(
                        content.min + Vec2::new(0.0, index as f32 * MEDIA_ROW_HEIGHT),
                        Vec2::new(content.width(), MEDIA_ROW_HEIGHT - 2.0),
                    );
                    let response = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            self.media_asset(ui, row, importing, false)
                        })
                        .inner;
                    if self
                        .media_reveal
                        .is_some_and(|reveal| reveal.asset == row.id)
                    {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        let modifiers = ui.ctx().input(|input| input.modifiers);
                        self.select_asset(row.id, modifiers);
                    }
                    rects.push((row.id, rect));
                }
                self.media_marquee(ui, content, &background, &rects);
            });
    }
    pub(in crate::editor) fn media_marquee(
        &mut self,
        ui: &mut egui::Ui,
        content: Rect,
        background: &egui::Response,
        rects: &[(AssetId, Rect)],
    ) {
        let modifiers = ui.ctx().input(|input| input.modifiers);
        if background.drag_started()
            && self.media_marquee.is_none()
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
            && !rects.iter().any(|(_, rect)| rect.contains(origin))
        {
            self.media_marquee = Some(MediaMarquee {
                origin,
                before: self.media_selection.clone(),
                additive: modifiers.command || modifiers.ctrl || modifiers.shift,
                moved: false,
            });
        }
        if let Some(marquee) = self.media_marquee.clone()
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(marquee.origin, pointer).intersect(content);
            let mut selected =
                crate::selection::intersections(rect.min, rect.max, rects.iter().copied());
            if marquee.additive {
                selected.extend(&marquee.before);
            }
            self.media_selection = selected;
            if let Some(state) = &mut self.media_marquee {
                state.moved |= marquee.origin.distance(pointer) > 4.0;
            }
            ui.painter().with_clip_rect(content).rect_filled(
                rect,
                0.0,
                self.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(content).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, self.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped()
            && let Some(marquee) = self.media_marquee.take()
        {
            if !marquee.moved {
                self.media_selection.clear();
                self.media_anchor = None;
            }
            self.session.end_merge();
        }
        if background.clicked() {
            self.media_selection.clear();
            self.media_anchor = None;
            self.session.end_merge();
        }
    }
    pub(in crate::editor) fn media_asset(
        &mut self,
        ui: &mut egui::Ui,
        row: &AssetRow,
        importing: bool,
        grid: bool,
    ) -> egui::Response {
        let rect = ui.max_rect();
        let response = ui.interact(
            rect,
            Id::unique(("media.asset", row.id.0)),
            egui::Sense::click(),
        );
        self.remember_with_icon(
            format!("media.asset.{}", row.id.0),
            Some(row.icon.name()),
            &response,
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, row.name.as_str()));
        if response.secondary_clicked() && !self.media_selection.contains(&row.id) {
            self.media_selection = [row.id].into_iter().collect();
            self.media_anchor = Some(row.id);
            self.session.end_merge();
        }
        let selected = self.media_selection.contains(&row.id);
        let highlighted = self
            .media_reveal
            .is_some_and(|reveal| reveal.asset == row.id);
        let dark = self.preferences.dark;
        let fill = if selected {
            self.accent()
        } else if response.hovered() {
            theme::hover(dark)
        } else {
            self.control_fill()
        };
        let text = if selected {
            theme::on_accent(dark)
        } else {
            self.text_color()
        };
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, theme::radius::CONTROL, fill);
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(
                if highlighted { 2.0 } else { 1.0 },
                if highlighted {
                    self.accent()
                } else {
                    self.border()
                },
            ),
            egui::StrokeKind::Inside,
        );
        let secondary = if selected {
            theme::on_accent(dark)
        } else {
            self.secondary()
        };
        if grid {
            let thumbnail = Rect::from_min_size(
                rect.min + Vec2::new(4.0, 4.0),
                Vec2::new(rect.width() - 8.0, 58.0),
            );
            painter.rect_filled(thumbnail, theme::radius::CONTROL, theme::field(dark));
            self.media_thumbnail(ui, row, thumbnail, 24.0, self.text_color());
            if let Some(duration) = row.duration {
                let label = display_short_time(duration);
                let badge = Rect::from_min_size(
                    thumbnail.right_bottom() - Vec2::new(4.0, 4.0),
                    Vec2::new(0.0, 0.0),
                );
                let galley = painter.layout_no_wrap(
                    label,
                    FontId::proportional(theme::type_size::CHROME),
                    theme::on_scrim(),
                );
                let badge = Rect::from_min_size(
                    badge.min - Vec2::new(galley.size().x + 4.0, galley.size().y + 2.0),
                    galley.size() + Vec2::new(4.0, 2.0),
                );
                painter.rect_filled(badge, theme::radius::CONTROL, theme::scrim());
                painter.galley(badge.min + Vec2::new(2.0, 1.0), galley, theme::on_scrim());
            }
            painter.text(
                egui::pos2(rect.left() + 5.0, thumbnail.bottom() + 5.0),
                Align2::LEFT_TOP,
                &row.name,
                FontId::proportional(theme::type_size::CHROME),
                text,
            );
            painter.text(
                egui::pos2(rect.left() + 5.0, thumbnail.bottom() + 27.0),
                Align2::LEFT_TOP,
                &row.brief,
                FontId::proportional(theme::type_size::CHROME),
                secondary,
            );
        } else {
            let thumbnail = Rect::from_center_size(
                egui::pos2(rect.left() + 36.0, rect.center().y),
                Vec2::new(64.0, 36.0),
            );
            self.media_thumbnail(ui, row, thumbnail, BUTTON_ICON, text);
            painter.text(
                egui::pos2(rect.left() + 74.0, rect.center().y - 2.0),
                Align2::LEFT_BOTTOM,
                &row.name,
                FontId::proportional(theme::type_size::CHROME),
                text,
            );
            painter.text(
                egui::pos2(rect.left() + 74.0, rect.center().y + 2.0),
                Align2::LEFT_TOP,
                &row.info,
                FontId::proportional(theme::type_size::CHROME),
                secondary,
            );
        }
        let status_rect = Rect::from_min_size(
            if grid {
                rect.min + Vec2::splat(6.0)
            } else {
                egui::pos2(rect.right() - 102.0, rect.top() + 3.0)
            },
            Vec2::new((rect.width() - 12.0).min(96.0), 20.0),
        );
        painter.rect_filled(status_rect, theme::radius::CONTROL, self.control_fill());
        let status = ui.put(
            status_rect,
            egui::Label::new(
                egui::RichText::new(asset_state_text(self.preferences.locale, row.state))
                    .size(theme::type_size::CHROME)
                    .color(asset_state_color(row.state, dark)),
            )
            .truncate(),
        );
        self.remember(format!("media.asset.{}.status", row.id.0), &status);
        let ids: Vec<AssetId> = if selected {
            self.media_selection.iter().copied().collect()
        } else {
            vec![row.id]
        };
        let label = if ids.len() == 1 {
            self.tr(Text::DeleteItemOne).to_owned()
        } else {
            catalog::format_count(
                self.preferences.locale,
                ids.len() as u64,
                Text::DeleteItemsOther,
                Text::DeleteItemsOther,
            )
        };
        response.context_menu(|ui| {
            let delete = ui.add_enabled(!importing, egui::Button::new(label));
            self.remember("media.delete", &delete);
            if delete.clicked() {
                self.request_media_delete(ids.clone());
                ui.close();
            }
        });
        response
    }
    pub(in crate::editor) fn media_thumbnail(
        &mut self,
        ui: &mut egui::Ui,
        row: &AssetRow,
        rect: Rect,
        fallback_size: f32,
        color: Color32,
    ) {
        if let Some(texture) = self.thumbnail_texture(ui.ctx(), row.id) {
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let image_rect = Rect::from_center_size(rect.center(), size * scale);
            let response = ui.put(
                image_rect,
                egui::Image::new((texture.id(), image_rect.size())),
            );
            self.remember(format!("media.asset.{}.thumbnail", row.id.0), &response);
        } else {
            icons::tinted(row.icon, fallback_size, color).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(fallback_size)),
            );
        }
    }
}
