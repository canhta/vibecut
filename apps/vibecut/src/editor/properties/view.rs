//! Renders the properties area: a 42 px tab strip, then collapsible sections of rows.

use super::{
    Action, ColorTarget, Context, Rect, State, Text, Vec2, Widget, egui, element_kind, single,
    theme,
};
use crate::i18n;
use crate::kit;

mod dialog;

use super::{
    AUDIO_GAIN_SCRUB, ElementKind, ItemId, OPACITY_SCRUB, PropertiesTab, ScrubSpec, SelectionScope,
    TEXT_SIZE_SCRUB, shared_fields,
};

/// Fields that share one diamond: an X/Y pair keys together.
fn diamond_group(field: &str) -> &str {
    use project::BuiltinPath as P;
    match P::parse(field) {
        Some(P::TransformPositionX | P::TransformPositionY) => P::TransformPositionX.as_str(),
        Some(P::TransformScaleX | P::TransformScaleY) => P::TransformScaleX.as_str(),
        _ => field,
    }
}
/// Left edge of the value column inside a 39 px property row.
const VALUE_COLUMN: f32 = 120.0;

fn is_basic(tab: PropertiesTab) -> bool {
    matches!(
        tab,
        PropertiesTab::Transform | PropertiesTab::Blending | PropertiesTab::RemoveBackground
    )
}

/// The basic sub-tabs that also carry the clip's Mask row.
fn shows_mask_row(tab: PropertiesTab) -> bool {
    matches!(tab, PropertiesTab::Transform | PropertiesTab::Blending)
}

/// Content inset from the panel edge, measured.
const INSET: f32 = 12.0;

/// One frame of the panel: the area's state, the shared context and the
/// requests collected for the frame to route.
struct Panel<'a, 'c> {
    state: &'a mut State,
    cx: &'a Context<'c>,
    actions: Vec<Action>,
}

/// Draw the whole panel (its 42 px header included) into `ui`'s rect and return
/// what the frame must route. Edits never touch the session from here.
pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    // The area draws its own strip, so it fills the slot the frame measured.
    let area = ui.max_rect();
    state.set_area(area);
    let mut panel = Panel {
        state,
        cx,
        actions: Vec::new(),
    };
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("inspector.area")
            .max_rect(area),
    );
    ui.set_clip_rect(area);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    panel.area(&mut ui, area);
    panel.picker_dialog(ui.ctx());
    panel.font_dialog(ui.ctx());
    panel.mask_dialog(ui.ctx());
    panel.actions
}

impl<'a, 'c> Panel<'a, 'c> {
    fn area(&mut self, ui: &mut egui::Ui, area: Rect) {
        if !self.state.details && self.cx.selection.len() == 1 {
            self.properties(ui, area);
            return;
        }
        self.state.focus = None;
        // Without a single element selected the strip switches the project views.
        const IDS: [&str; 2] = ["inspector.project", "inspector.details"];
        let ids = IDS.map(str::to_owned);
        let labels = [self.tr(Text::Settings), self.tr(Text::Details)];
        let selected = usize::from(self.state.details);
        let tabs = kit::text_tabs(ui, &ids, &labels, selected);
        for (id, response) in IDS.into_iter().zip(&tabs) {
            self.remember(id, response);
            if response.clicked() {
                self.actions.push(Action::Activate(id));
            }
        }
        let mut body = body_ui(ui, area);
        if self.state.details {
            self.project_details(&mut body);
        } else if self.cx.selection.is_empty() {
            self.settings(&mut body);
        } else {
            egui::ScrollArea::vertical()
                .id_salt("inspector.shared")
                .show(&mut body, |ui| {
                    self.panel_section(
                        ui,
                        "inspector.section.selection",
                        "inspector.selection",
                        Text::Selection,
                        true,
                        false,
                        |panel, ui| panel.shared_inspector(ui),
                    );
                });
        }
    }

    fn tr(&self, key: Text) -> &'static str {
        i18n::text(self.cx.locale, key)
    }
    fn secondary(&self) -> egui::Color32 {
        theme::secondary(self.cx.dark)
    }
    fn text_color(&self) -> egui::Color32 {
        theme::text(self.cx.dark)
    }
    fn control_fill(&self) -> egui::Color32 {
        theme::field(self.cx.dark)
    }
    fn fail(&mut self, error: impl ToString) {
        self.actions.push(Action::Error(error.to_string()));
    }
    fn remember(&mut self, id: impl Into<String>, response: &egui::Response) {
        self.remember_with_icon(id, None, response);
    }
    fn remember_with_icon(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        response: &egui::Response,
    ) {
        let id = id.into();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(&id));
        self.state.widgets.push(Widget {
            id,
            icon,
            tooltip: None,
            response: response.clone(),
        });
    }
    #[allow(clippy::too_many_arguments)]
    fn icon_only(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        icon: kit::Icon,
        label: &str,
        selected: Option<bool>,
        enabled: bool,
    ) -> bool {
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                kit::icon_button(ui, id, icon, label, selected.unwrap_or(false))
            })
            .inner;
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, label)
            } else {
                egui::WidgetInfo::labeled(egui::Role::Button, enabled, label)
            }
        });
        self.remember_with_icon(id, Some(icon.name()), &response);
        if let Some(widget) = self.state.widgets.last_mut() {
            widget.tooltip = Some(label.to_owned());
        }
        enabled && response.clicked()
    }

    fn set_item_field(&mut self, item: ItemId, field: &str, value: &str) {
        self.actions.push(Action::SetField {
            item,
            field: field.to_owned(),
            value: value.to_owned(),
        });
    }
    fn set_shared_field(&mut self, field: &str, value: &str) {
        self.actions.push(Action::SetShared {
            field: field.to_owned(),
            value: value.to_owned(),
        });
    }
    fn set_speed(&mut self, item: ItemId, speed: f64, maintain_pitch: bool) {
        self.actions.push(Action::SetSpeed {
            item,
            speed,
            maintain_pitch,
        });
    }
    fn set_effect_param(&mut self, item: ItemId, effect: &str, key: &str, value: f64) {
        self.actions.push(Action::SetEffectParam {
            item,
            effect: effect.to_owned(),
            key: key.to_owned(),
            value,
        });
    }

    /// A diamond per animatable field in this row, at the right of the label column.
    fn keyframe_diamonds(
        &mut self,
        ui: &mut egui::Ui,
        row: Rect,
        item: ItemId,
        specs: &[ScrubSpec],
    ) {
        let animate: Vec<ScrubSpec> = specs
            .iter()
            .copied()
            .filter(|spec| {
                spec.field == "audio_gain"
                    || matches!(
                        project::BuiltinPath::parse(spec.field),
                        Some(
                            project::BuiltinPath::Opacity
                                | project::BuiltinPath::TransformPositionX
                                | project::BuiltinPath::TransformPositionY
                                | project::BuiltinPath::TransformScaleX
                                | project::BuiltinPath::TransformScaleY
                                | project::BuiltinPath::TransformRotate
                        )
                    )
            })
            .collect();
        if animate.is_empty() {
            return;
        }
        let Some((start, end)) = super::read::range_of(self.cx.session.project(), item) else {
            return;
        };
        let at = self.cx.playhead;
        let inside = start <= at && at <= end;
        let relative = at.saturating_sub(start);
        // CapCut shows one diamond for a position or scale pair.
        let mut groups: Vec<(&str, Vec<ScrubSpec>)> = Vec::new();
        for spec in animate {
            let key = diamond_group(spec.field);
            match groups.last_mut() {
                Some((last, group)) if *last == key => group.push(spec),
                _ => groups.push((key, vec![spec])),
            }
        }
        let width = theme::size::CONTROL * groups.len() as f32;
        let rect = Rect::from_min_max(
            egui::pos2(row.left() + VALUE_COLUMN - width - 4.0, row.top()),
            egui::pos2(row.left() + VALUE_COLUMN - 4.0, row.bottom()),
        );
        let mut strip = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        strip.spacing_mut().item_spacing.x = 0.0;
        for (_, group) in groups {
            let Some(lead) = group.first().map(|spec| spec.field) else {
                continue;
            };
            let id = format!("inspector.keyframe.{lead}");
            let exists = group.iter().all(|spec| {
                super::read::has_key_at(self.cx.session.project(), item, spec.field, relative)
            });
            let label = self.tr(if exists {
                Text::RemoveKeyframe
            } else {
                Text::AddKeyframe
            });
            let response = strip
                .add_enabled_ui(inside, |ui| {
                    kit::icon_button(ui, &id, crate::kit::Icon::Diamond, label, exists)
                })
                .inner;
            self.remember_with_icon(id, Some(crate::kit::Icon::Diamond.name()), &response);
            if response.clicked() && inside {
                let fields = group
                    .iter()
                    .map(|spec| {
                        let value = super::read::item_value_at(
                            self.cx.session.project(),
                            item,
                            spec.field,
                            relative,
                        )
                        .unwrap_or(0.0);
                        (spec.field.to_owned(), value)
                    })
                    .collect();
                self.actions.push(Action::ToggleKeyframe {
                    item,
                    fields,
                    time: relative,
                });
            }
        }
    }

    fn project_details(&mut self, ui: &mut egui::Ui) {
        self.panel_section(
            ui,
            "inspector.section.canvas",
            "inspector.canvas",
            Text::Canvas,
            true,
            false,
            |panel, ui| {
                let locale = panel.cx.locale;
                let data = panel.cx.session.project().data();
                let width = i18n::number(locale, u64::from(data.canvas.width));
                let height = i18n::number(locale, u64::from(data.canvas.height));
                let numerator = i18n::signed(locale, data.canvas.fps.num());
                let denominator = i18n::signed(locale, data.canvas.fps.den());
                let rate = i18n::number(locale, u64::from(data.settings.audio_sample_rate));
                let lines = [
                    i18n::format(
                        locale,
                        Text::CanvasSize,
                        &[("width", &width), ("height", &height)],
                    ),
                    i18n::format(
                        locale,
                        Text::FrameRate,
                        &[("num", &numerator), ("den", &denominator)],
                    ),
                    i18n::format(locale, Text::SampleRate, &[("rate", &rate)]),
                ];
                for line in lines {
                    panel.detail_row(ui, &line);
                }
            },
        );
        self.panel_section(
            ui,
            "inspector.section.metadata",
            "inspector.metadata",
            Text::Metadata,
            false,
            false,
            |panel, ui| {
                let locale = panel.cx.locale;
                let metadata = panel.cx.session.metadata();
                let created = i18n::date(locale, metadata.created_unix_ms);
                let modified = i18n::date(locale, metadata.modified_unix_ms);
                for line in [
                    i18n::format(locale, Text::CreatedDate, &[("date", &created)]),
                    i18n::format(locale, Text::ModifiedDate, &[("date", &modified)]),
                ] {
                    panel.detail_row(ui, &line);
                }
            },
        );
    }

    fn detail_row(&self, ui: &mut egui::Ui, text: &str) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(text)
                        .size(theme::type_size::BODY)
                        .color(self.secondary()),
                );
            },
        );
    }

    /// A collapsible section: header with caret and optional reset, rows below.
    /// Returns whether reset was clicked; the open state lives in preferences.
    #[allow(clippy::too_many_arguments)]
    fn panel_section(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: &'static str,
        label: Text,
        default_open: bool,
        reset: bool,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> bool {
        let stored = self.cx.preferences.section_open(key, default_open);
        let mut open = stored;
        let label = self.tr(label);
        let reset_label = self.tr(Text::ResetValue);
        let header =
            kit::section_header_with(ui, id, label, &mut open, reset.then_some(reset_label), None);
        self.remember(id, &header.toggle);
        let clicked = header.reset.as_ref().is_some_and(|response| {
            self.remember(format!("{id}.reset"), response);
            response.clicked()
        });
        if open != stored {
            self.actions.push(Action::SetSection { key, open });
        }
        if open {
            body(self, ui);
        }
        clicked
    }

    /// One selected element: its type's tab set, remembered per element type.
    fn properties(&mut self, ui: &mut egui::Ui, area: Rect) {
        let Some(item) = single(self.cx.selection) else {
            return;
        };
        let Some((kind, has_audio)) = element_kind(self.cx.session.project(), item) else {
            return;
        };
        if self.state.selection != *self.cx.selection {
            self.state.drafts.clear();
            self.state.focus = None;
            self.state.font_picker.close();
            self.state.selection = self.cx.selection.clone();
        }
        let tabs: Vec<PropertiesTab> = kind
            .tabs()
            .iter()
            .copied()
            .filter(|tab| {
                !(*tab == PropertiesTab::Audio && kind == ElementKind::Video && !has_audio)
            })
            .collect();
        let stored = self
            .cx
            .preferences
            .properties_tabs
            .get(kind.id())
            .and_then(|id| PropertiesTab::from_id(id))
            .filter(|tab| tabs.contains(tab));
        let active = stored.unwrap_or_else(|| kind.default_tab());
        if !shows_mask_row(active) || !matches!(kind, ElementKind::Video | ElementKind::Image) {
            self.state.mask.close();
        }
        // Transform and blending share one top tab with segmented sub-tabs.
        let basic: Vec<PropertiesTab> = tabs.iter().copied().filter(|tab| is_basic(*tab)).collect();
        let group_label = match kind {
            ElementKind::Video => Text::Video,
            ElementKind::Image => Text::Image,
            _ => Text::Basic,
        };
        let mut top: Vec<Option<PropertiesTab>> = Vec::new();
        for tab in &tabs {
            if !is_basic(*tab) {
                top.push(Some(*tab));
            } else if !top.contains(&None) {
                top.push(None);
            }
        }
        let ids: Vec<String> = top
            .iter()
            .map(|entry| match entry {
                Some(tab) => format!("inspector.tab.{}", tab.id()),
                None => "inspector.group.basic".to_owned(),
            })
            .collect();
        let labels: Vec<&str> = top
            .iter()
            .map(|entry| match entry {
                Some(tab) => self.tr(tab.label()),
                None => self.tr(group_label),
            })
            .collect();
        let selected = top
            .iter()
            .position(|entry| match entry {
                Some(tab) => *tab == active,
                None => is_basic(active),
            })
            .unwrap_or(0);
        let responses = kit::text_tabs(ui, &ids, &labels, selected);
        let mut clicked = None;
        for ((id, response), entry) in ids.iter().zip(&responses).zip(&top) {
            self.remember(id.clone(), response);
            if response.clicked() {
                clicked = match entry {
                    Some(tab) => Some(*tab),
                    None if is_basic(active) => None,
                    None => basic.first().copied(),
                };
            }
        }
        let mut body = body_ui(ui, area);
        let mut sub_clicked = None;
        if is_basic(active) && basic.len() > 1 {
            body.add_space(13.0);
            let sub_ids: Vec<String> = basic
                .iter()
                .map(|tab| format!("inspector.tab.{}", tab.id()))
                .collect();
            let sub_labels: Vec<&str> = basic.iter().map(|tab| self.tr(tab.label())).collect();
            let at = basic.iter().position(|tab| *tab == active).unwrap_or(0);
            let cells = kit::segmented(&mut body, &sub_ids, &sub_labels, at);
            for ((id, response), tab) in sub_ids.iter().zip(&cells).zip(&basic) {
                self.remember(id.clone(), response);
                if response.clicked() {
                    sub_clicked = Some(*tab);
                }
            }
            body.add_space(12.0);
        }
        if let Some(tab) = clicked.or(sub_clicked) {
            self.state.focus = None;
            self.actions.push(Action::SetTab {
                kind: kind.id(),
                tab: tab.id(),
            });
        }
        if matches!(
            self.cx.scope,
            SelectionScope::MaskPoints | SelectionScope::Keyframes
        ) {
            let scope = match self.cx.scope {
                SelectionScope::MaskPoints => Text::MaskPoints,
                _ => Text::Keyframes,
            };
            body.label(
                egui::RichText::new(self.tr(scope))
                    .size(theme::type_size::BODY)
                    .color(self.secondary()),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("inspector.properties")
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                if shows_mask_row(active) {
                    self.mask_row(ui, item, kind);
                }
                self.properties_tab(ui, item, kind, active)
            });
    }

    fn properties_tab(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        kind: ElementKind,
        tab: PropertiesTab,
    ) {
        match tab {
            PropertiesTab::Text
                if super::read::item_value(
                    self.cx.session.project(),
                    item,
                    TEXT_SIZE_SCRUB.field,
                )
                .is_some() =>
            {
                self.panel_section(
                    ui,
                    "inspector.section.text",
                    "inspector.text",
                    Text::TextItem,
                    true,
                    false,
                    |panel, ui| {
                        for field in shared_fields(panel.cx.session.project(), panel.cx.selection) {
                            match field.id {
                                "text_size" => panel.scrub_field(ui, item, TEXT_SIZE_SCRUB),
                                "text_color" => panel.text_color_field(ui, item),
                                "text_content" | "text_font" | "text_background"
                                | "text_position_x" | "text_position_y" => {
                                    panel.shared_field_controls(ui, std::iter::once(field));
                                }
                                _ => {}
                            }
                        }
                        panel.color_row(
                            ui,
                            "inspector.text_color.picker",
                            Text::ColorPicker,
                            ColorTarget::Text(item),
                        );
                        panel.color_row(
                            ui,
                            "inspector.background_color",
                            Text::BackgroundColor,
                            ColorTarget::Background(item),
                        );
                        panel.text_style_controls(ui, item);
                    },
                );
            }
            PropertiesTab::Transform => {
                let reset = self.panel_section(
                    ui,
                    "inspector.section.transform",
                    "inspector.transform",
                    Text::Transform,
                    true,
                    true,
                    |panel, ui| {
                        use project::TransformField as F;
                        let mut scale = super::transform_scrub(F::ScaleX);
                        if panel.state.uniform_scale {
                            scale.label = Text::Scale;
                        }
                        panel.slider_row(ui, item, scale);
                        let (row, mut value_ui) = panel.row_value_ui(ui, theme::size::PROPERTY_ROW);
                        panel.row_label(ui, row, panel.tr(Text::UniformScale));
                        let mut uniform = panel.state.uniform_scale;
                        let width = value_ui.available_width();
                        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
                        let response = kit::toggle(
                            &mut value_ui,
                            "inspector.uniform_scale",
                            &mut uniform,
                            panel.tr(Text::UniformScale),
                        );
                        panel.remember("inspector.uniform_scale", &response);
                        panel.state.uniform_scale = uniform;
                        if !uniform {
                            panel.slider_row(ui, item, super::transform_scrub(F::ScaleY));
                        }
                        panel.number_row(
                            ui,
                            item,
                            Text::Position,
                            &[
                                (super::transform_scrub(F::PositionX), Some("X")),
                                (super::transform_scrub(F::PositionY), Some("Y")),
                            ],
                        );
                        panel.scrub_field(ui, item, super::transform_scrub(F::Rotate));
                    },
                );
                if reset {
                    self.state.drafts.clear();
                    self.state.focus = None;
                    self.actions.push(Action::ResetTransform(item));
                }
            }
            PropertiesTab::Graphic => {
                self.panel_section(
                    ui,
                    "inspector.section.graphic",
                    "inspector.graphic",
                    Text::Graphic,
                    true,
                    false,
                    |panel, ui| {
                        panel.color_row(
                            ui,
                            "inspector.graphic_fill",
                            Text::GraphicFill,
                            ColorTarget::GraphicFill(item),
                        );
                        panel.color_row(
                            ui,
                            "inspector.graphic_stroke",
                            Text::GraphicStroke,
                            ColorTarget::GraphicStroke(item),
                        );
                    },
                );
            }
            PropertiesTab::Audio if kind == ElementKind::Audio => {
                self.panel_section(
                    ui,
                    "inspector.section.audio",
                    "inspector.audio",
                    Text::Audio,
                    true,
                    false,
                    |panel, ui| {
                        panel.scrub_field(ui, item, AUDIO_GAIN_SCRUB);
                        if let Some(field) =
                            shared_fields(panel.cx.session.project(), panel.cx.selection)
                                .into_iter()
                                .find(|field| field.id == "audio_muted")
                        {
                            panel.audio_mute_control(ui, &field);
                        }
                    },
                );
            }
            PropertiesTab::Speed if matches!(kind, ElementKind::Video | ElementKind::Audio) => {
                self.panel_section(
                    ui,
                    "inspector.section.speed",
                    "inspector.speed",
                    Text::Speed,
                    true,
                    false,
                    |panel, ui| panel.speed_controls(ui, item),
                );
            }
            PropertiesTab::Blending => {
                self.panel_section(
                    ui,
                    "inspector.section.blending",
                    "inspector.blending",
                    Text::Blending,
                    true,
                    false,
                    |panel, ui| {
                        panel.blend_control(ui, item);
                        panel.slider_row(ui, item, OPACITY_SCRUB);
                    },
                );
            }
            PropertiesTab::Adjust if matches!(kind, ElementKind::Video | ElementKind::Image) => {
                self.panel_section(
                    ui,
                    "inspector.section.adjust",
                    "inspector.adjust",
                    Text::Adjust,
                    true,
                    false,
                    |panel, ui| panel.adjust_controls(ui, item),
                );
            }
            PropertiesTab::RemoveBackground
                if matches!(kind, ElementKind::Video | ElementKind::Image) =>
            {
                self.remove_background_controls(ui, item);
            }
            PropertiesTab::Effects
                if matches!(
                    kind,
                    ElementKind::Video
                        | ElementKind::Image
                        | ElementKind::Sticker
                        | ElementKind::Graphic
                        | ElementKind::Text
                ) =>
            {
                self.effects_stack(ui, item);
            }
            _ => {}
        }
    }
}

fn body_ui(ui: &mut egui::Ui, area: Rect) -> egui::Ui {
    let rect = Rect::from_min_max(
        egui::pos2(area.left() + INSET, area.top() + theme::size::TAB_STRIP),
        egui::pos2(area.right() - INSET, area.bottom()),
    );
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing = Vec2::ZERO;
    body
}

mod canvas;

mod speed;

mod effects;

mod text_style;

mod appearance;

mod mask;

mod number_edit;

mod remove_background;

mod selection;

mod font_dialog;
