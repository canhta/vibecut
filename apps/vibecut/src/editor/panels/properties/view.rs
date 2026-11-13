//! Renders the properties area: a 42 px tab strip, then collapsible sections of rows.

use super::read;
use super::{
    Action, ColorTarget, Context, Rect, State, Text, Vec2, Widget, egui, element_kind, theme,
};
use crate::editor::context::single;
use crate::i18n;
use crate::kit;

mod dialog;

use super::{
    ElementKind, ItemId, PropertiesTab, ScrubSpec, SelectionScope, TEXT_SIZE_SCRUB, shared_fields,
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
        PropertiesTab::Transform | PropertiesTab::RemoveBackground | PropertiesTab::Mask
    )
}

/// The Mask sub-tab carries the clip's Mask row.
fn shows_mask_row(tab: PropertiesTab) -> bool {
    tab == PropertiesTab::Mask
}

/// The reference names the Transform sub-tab `Basic`; it holds Transform and Blend.
fn sub_tab_label(tab: PropertiesTab) -> Text {
    match tab {
        PropertiesTab::Transform => Text::Basic,
        other => other.label(),
    }
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
    if let Some((snapshot, item, at)) = &panel.state.opacity_edit {
        let graphic_edit = panel
            .state
            .focus
            .as_deref()
            .is_some_and(|f| f.starts_with("params."))
            || panel.state.scrub.as_ref().is_some_and(
                |s| matches!(s.target, super::ScrubTarget::Item(f) if f.starts_with("params.")),
            );
        let visible = element_kind(panel.cx.session.project(), *item).is_some_and(|(kind, _)| {
            panel
                .cx
                .preferences
                .properties_tabs
                .get(kind.id())
                .map(String::as_str)
                .unwrap_or(kind.default_tab().id())
                == if graphic_edit { "graphic" } else { "transform" }
        });
        if !std::sync::Arc::ptr_eq(snapshot, panel.cx.session.project())
            || single(panel.cx.selection()) != Some(*item)
            || *at != panel.cx.playhead
            || read::item_locked(panel.cx.session.project(), *item)
            || !visible
            || panel.state.details
        {
            panel.actions.extend(panel.state.cancel_opacity());
        }
    }
    if let Some((item, ref target, time)) = panel.state.spatial_edit.clone() {
        let id = target.draft_key();
        let exists = read::item(panel.cx.session.project(), item).is_some_and(|candidate| {
            candidate
                .effects
                .iter()
                .any(|e| id.starts_with(&format!("effects.{}.param.", e.id)))
        });
        if single(panel.cx.selection()) != Some(item)
            || time != panel.cx.playhead
            || !exists
            || read::item_locked(panel.cx.session.project(), item)
        {
            panel.actions.extend(panel.state.end_scrub(false));
            panel.state.drafts.remove(&id);
            panel.state.focus = None;
            panel.state.spatial_edit = None;
        }
    }
    let stale_graphic_picker = panel.state.picker.as_ref().is_some_and(|draft| {
        if let ColorTarget::GraphicColor { item, time, .. } = draft.target {
            single(panel.cx.selection()) != Some(item)
                || read::item_locked(panel.cx.session.project(), item)
                || read::item(panel.cx.session.project(), item)
                    .is_none_or(|i| panel.cx.playhead.saturating_sub(i.start) != time)
                || panel
                    .cx
                    .preferences
                    .properties_tabs
                    .get("graphic")
                    .map(String::as_str)
                    .unwrap_or("graphic")
                    != "graphic"
                || panel.state.details
        } else {
            false
        }
    });
    if stale_graphic_picker {
        panel.actions.extend(panel.state.close_picker());
    }
    panel.area(&mut ui, area);
    if panel.state.graph.is_some()
        && panel.actions.iter().any(|action| {
            matches!(
                action,
                Action::SetTab { .. }
                    | Action::Activate(_)
                    | Action::RemoveEffect { .. }
                    | Action::ToggleEffect { .. }
            )
        })
    {
        panel.actions.extend(panel.state.end_graph(false));
    }
    if let Some((_, ref target, _)) = panel.state.spatial_edit.clone()
        && !panel
            .state
            .widgets
            .iter()
            .any(|widget| widget.id == target.field_id())
    {
        let id = target.draft_key();
        panel.actions.extend(panel.state.end_scrub(false));
        panel.state.drafts.remove(&id);
        panel.state.focus = None;
        panel.state.spatial_edit = None;
    }
    panel.picker_dialog(ui.ctx());
    panel.font_dialog(ui.ctx());
    panel.mask_dialog(ui.ctx());
    panel.actions
}

impl<'a, 'c> Panel<'a, 'c> {
    fn area(&mut self, ui: &mut egui::Ui, area: Rect) {
        if let Some(transition) = self.cx.transition {
            self.transition_panel(ui, area, transition);
            return;
        }
        if !self.state.details && self.cx.selection().len() == 1 {
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
        } else if self.cx.selection().is_empty() {
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

    fn graphic_command(&mut self, command: engine::Command) {
        if let engine::Command::SetAnimatedParam { item, path, param } = command {
            self.actions
                .push(Action::SetAnimatedParam { item, path, param });
        }
    }
    fn graphic_color_row(&mut self, ui: &mut egui::Ui, item: ItemId, stroke: bool) {
        let Some((start, _)) = read::range_of(self.cx.session.project(), item) else {
            return;
        };
        let time = self.cx.playhead.saturating_sub(start);
        let target = ColorTarget::GraphicColor { item, stroke, time };
        let name = if stroke { "stroke" } else { "fill" };
        let id = format!("inspector.graphic.{name}");
        let pos = ui.next_widget_position();
        self.color_row(
            ui,
            &id,
            if stroke {
                Text::GraphicStroke
            } else {
                Text::GraphicFill
            },
            target.clone(),
        );
        let rect = Rect::from_min_size(
            pos + Vec2::new(VALUE_COLUMN - theme::size::CONTROL - 4., 8.),
            Vec2::splat(theme::size::CONTROL),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let enabled = target.inside(self.cx.session.project());
        let response = child
            .add_enabled_ui(enabled, |ui| {
                kit::icon_button(
                    ui,
                    &format!("{id}.key"),
                    crate::kit::Icon::Diamond,
                    self.tr(Text::AddKeyframe),
                    target.rgb_keys(self.cx.session.project(), true),
                )
            })
            .inner;
        self.remember(format!("{id}.key"), &response);
        if response.clicked() {
            match target.toggle_rgb(self.cx.session.project()) {
                Ok(command) => self.graphic_command(command),
                Err(e) => self.fail(e),
            }
        }
    }
    fn graphic_align(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(target) = read::item(self.cx.session.project(), item) else {
            return;
        };
        let project::ItemParams::Graphic(p) = &target.content else {
            return;
        };
        let align = match p.appearance().3 {
            project::StrokeAlign::Inside => "inside",
            project::StrokeAlign::Center => "center",
            project::StrokeAlign::Outside => "outside",
        };
        let Ok(path) = project::AnimationPath::new("params.strokeAlign") else {
            return;
        };
        let Ok(mut param) = target
            .animations
            .get(&path)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| project::Param::new(project::ParamValue::Text(align.into())))
        else {
            return;
        };
        let time = self.cx.playhead.saturating_sub(target.start);
        let inside = self.cx.playhead >= target.start
            && target.end().is_ok_and(|end| self.cx.playhead < end);
        let Ok(project::ParamValue::Text(current)) = param.value_at(time) else {
            return;
        };
        let Ok(channel) = param.channel(None) else {
            return;
        };
        let keyed = !channel.keys().is_empty();
        let exists = channel.keys().iter().any(|k| k.time == time);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::GraphicAlign));
        let mut selected = None;
        let response = value_ui
            .add_enabled_ui(!keyed || inside, |ui| {
                ui.spacing_mut().interact_size.y = theme::size::FIELD;
                egui::ComboBox::from_id_salt(super::Id::unique(format!(
                    "inspector.graphic.strokeAlign.{}.{}.{:p}",
                    item.0,
                    self.cx.playhead.0,
                    std::sync::Arc::as_ptr(self.cx.session.project())
                )))
                .selected_text(self.tr(match current.as_str() {
                    "inside" => Text::GraphicInside,
                    "outside" => Text::GraphicOutside,
                    _ => Text::AlignCenter,
                }))
                .width(150.)
                .show_ui(ui, |ui| {
                    for (value, label) in [
                        ("inside", Text::GraphicInside),
                        ("center", Text::AlignCenter),
                        ("outside", Text::GraphicOutside),
                    ] {
                        let response =
                            ui.selectable_label(current.as_str() == value, self.tr(label));
                        self.remember(format!("inspector.graphic.strokeAlign.{value}"), &response);
                        if response.clicked() {
                            selected = Some(value);
                            ui.close();
                        }
                    }
                })
                .response
            })
            .inner;
        self.remember("inspector.graphic.strokeAlign", &response);
        let rect = Rect::from_min_size(
            row.min + Vec2::new(VALUE_COLUMN - theme::size::CONTROL - 4., 8.),
            Vec2::splat(theme::size::CONTROL),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let response = child
            .add_enabled_ui(inside, |ui| {
                kit::icon_button(
                    ui,
                    "inspector.graphic.strokeAlign.key",
                    crate::kit::Icon::Diamond,
                    self.tr(Text::AddKeyframe),
                    exists,
                )
            })
            .inner;
        self.remember("inspector.graphic.strokeAlign.key", &response);
        if selected.is_none() && !response.clicked() {
            return;
        }
        let Ok(channel) = param.channel_mut(None) else {
            return;
        };
        let result = if let Some(selected) = selected {
            let value = project::ParamValue::Text(selected.into());
            if keyed {
                let mut key = channel
                    .keys()
                    .iter()
                    .find(|k| k.time == time)
                    .cloned()
                    .unwrap_or_else(|| project::Keyframe::new(time, value.clone()));
                key.value = value;
                key.segment = project::Segment::Step;
                channel.put(key)
            } else {
                project::Param::new(value).map(|v| param = v)
            }
        } else if exists {
            channel.remove(time)
        } else {
            let mut key = project::Keyframe::new(time, project::ParamValue::Text(current));
            key.segment = project::Segment::Step;
            channel.put(key)
        };
        match result {
            Ok(()) => self
                .actions
                .push(Action::SetAnimatedParam { item, path, param }),
            Err(e) => self.fail(e),
        }
    }
    /// Two reflection toggles under Rotate. They run the same registered actions
    /// as the timeline toolbar, so one typed command owns the item edit.
    fn transform_flips(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(target) = read::item(self.cx.session.project(), item) else {
            return;
        };
        if !target.content.supports_flip() {
            return;
        }
        let horizontal =
            project::FlipAxis::Horizontal.value_at(&target.animations, time::Tick::ZERO);
        let vertical = project::FlipAxis::Vertical.value_at(&target.animations, time::Tick::ZERO);
        let (Ok(horizontal), Ok(vertical)) = (horizontal, vertical) else {
            return;
        };
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        let label = self.tr(Text::Flip);
        self.row_label(ui, row, label);
        value_ui.spacing_mut().item_spacing.x = 4.0;
        let (horizontal_label, vertical_label) = (
            self.tr(Text::ActionFlipHorizontal),
            self.tr(Text::ActionFlipVertical),
        );
        // The panel's context is not the timeline's disabled set; decide here from
        // the same lock and keyed-axis rule the registered action uses.
        let locked = read::item_locked(self.cx.session.project(), item);
        let keyed = |axis: project::FlipAxis| {
            let path = project::AnimationPath::from(axis.builtin());
            target
                .animations
                .get(&path)
                .is_some_and(|param| match param.channel(None) {
                    Ok(channel) => !channel.keys().is_empty(),
                    Err(_) => true,
                })
        };
        let horizontal_enabled = !locked && !keyed(project::FlipAxis::Horizontal);
        let vertical_enabled = !locked && !keyed(project::FlipAxis::Vertical);
        if self.icon_only(
            &mut value_ui,
            "inspector.transform.flip_horizontal",
            kit::Icon::ArrowsInLineHorizontal,
            horizontal_label,
            Some(horizontal),
            horizontal_enabled,
        ) {
            self.actions
                .push(Action::Activate("timeline.flip_horizontal"));
        }
        if self.icon_only(
            &mut value_ui,
            "inspector.transform.flip_vertical",
            kit::Icon::ArrowsInLineVertical,
            vertical_label,
            Some(vertical),
            vertical_enabled,
        ) {
            self.actions
                .push(Action::Activate("timeline.flip_vertical"));
        }
    }
    /// The six canvas alignments and two even-spacing actions for the current
    /// selection. All controls share the registered engine actions, so a panel
    /// click, a shortcut and an agent command are the same typed edit.
    fn arrange_controls(&mut self, ui: &mut egui::Ui, items: &[ItemId]) {
        if items.is_empty() {
            return;
        }
        let ready = {
            let project = self.cx.session.project();
            items.iter().all(|item| {
                read::item(project, *item).is_some_and(|target| {
                    target.content.supports_arrange()
                        && !read::item_locked(project, *item)
                        && !self.cx.playback.placement_overridden(*item)
                })
            })
        };
        let spread = items.len() >= 3;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::Arrange));
        value_ui.spacing_mut().item_spacing.x = 2.0;
        for (id, icon, label, action, available) in [
            (
                "inspector.transform.align_left",
                kit::Icon::AlignLeft,
                Text::ActionAlignLeft,
                "timeline.align_left",
                true,
            ),
            (
                "inspector.transform.align_center_horizontal",
                kit::Icon::AlignCenterHorizontal,
                Text::ActionAlignHorizontalCenter,
                "timeline.align_horizontal_center",
                true,
            ),
            (
                "inspector.transform.align_right",
                kit::Icon::AlignRight,
                Text::ActionAlignRight,
                "timeline.align_right",
                true,
            ),
            (
                "inspector.transform.align_top",
                kit::Icon::AlignTop,
                Text::ActionAlignTop,
                "timeline.align_top",
                true,
            ),
            (
                "inspector.transform.align_center_vertical",
                kit::Icon::AlignCenterVertical,
                Text::ActionAlignVerticalCenter,
                "timeline.align_vertical_center",
                true,
            ),
            (
                "inspector.transform.align_bottom",
                kit::Icon::AlignBottom,
                Text::ActionAlignBottom,
                "timeline.align_bottom",
                true,
            ),
            (
                "inspector.transform.distribute_horizontal",
                kit::Icon::ArrowsOutLineHorizontal,
                Text::ActionDistributeHorizontal,
                "timeline.distribute_horizontal",
                spread,
            ),
            (
                "inspector.transform.distribute_vertical",
                kit::Icon::ArrowsOutLineVertical,
                Text::ActionDistributeVertical,
                "timeline.distribute_vertical",
                spread,
            ),
        ] {
            let label = self.tr(label);
            if self.icon_only(&mut value_ui, id, icon, label, None, ready && available) {
                self.actions.push(Action::Activate(action));
            }
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
                    || spec.field.starts_with("params.")
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
        let inside = start <= at
            && (at < end || (!specs.iter().any(|s| s.field.starts_with("params.")) && at == end));
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
            let id = format!("inspector.keyframe.{}", lead.replace("params.", "graphic."));
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
        let Some(item) = single(self.cx.selection()) else {
            return;
        };
        let Some((kind, has_audio)) = element_kind(self.cx.session.project(), item) else {
            return;
        };
        if self.state.selection != *self.cx.selection() {
            self.state.drafts.clear();
            self.state.focus = None;
            self.state.font_picker.close();
            self.state.selection = self.cx.selection().clone();
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
            let sub_labels: Vec<&str> = basic
                .iter()
                .map(|tab| self.tr(sub_tab_label(*tab)))
                .collect();
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
            self.actions.extend(self.state.cancel_opacity());
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
                        for field in shared_fields(panel.cx.session.project(), panel.cx.selection())
                        {
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
                        panel.arrange_controls(ui, &[item]);
                        panel.scrub_field(ui, item, super::transform_scrub(F::Rotate));
                        panel.transform_flips(ui, item);
                    },
                );
                if reset {
                    self.state.drafts.clear();
                    self.state.focus = None;
                    self.actions.push(Action::ResetTransform(item));
                }
                // Blend is a section inside Basic, after Transform.
                let blend_reset = self.panel_section(
                    ui,
                    "inspector.section.blending",
                    "inspector.blending",
                    Text::Blending,
                    true,
                    true,
                    |panel, ui| panel.blend_section(ui, item),
                );
                if blend_reset {
                    self.state.drafts.clear();
                    self.state.focus = None;
                    self.actions.push(Action::ResetBlend(item));
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
                        if matches!(
                            read::item(panel.cx.session.project(), item).map(|i| &i.content),
                            Some(project::ItemParams::Graphic(_))
                        ) {
                            panel.graphic_color_row(ui, item, false);
                            panel.graphic_color_row(ui, item, true);
                            panel.scrub_field(
                                ui,
                                item,
                                ScrubSpec {
                                    field: "params.strokeWidth",
                                    label: Text::GraphicWidth,
                                    min: 0.,
                                    max: 64.,
                                    step: 1.,
                                    pixels_per_step: 2.,
                                    decimals: 0,
                                    suffix: "",
                                },
                            );
                            panel.graphic_align(ui, item);
                            match read::item(panel.cx.session.project(), item)
                                .map(|item| &item.content)
                            {
                                Some(project::ItemParams::Graphic(
                                    project::GraphicParams::Polygon { .. },
                                )) => {
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec {
                                            field: "params.sides",
                                            label: Text::GraphicSides,
                                            min: 3.,
                                            max: 12.,
                                            step: 1.,
                                            pixels_per_step: 2.,
                                            decimals: 0,
                                            suffix: "",
                                        },
                                    );
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec {
                                            field: "params.cornerRadius",
                                            label: Text::BackgroundRadius,
                                            min: 0.,
                                            max: 50.,
                                            step: 1.,
                                            pixels_per_step: 2.,
                                            decimals: 0,
                                            suffix: "%",
                                        },
                                    );
                                }
                                Some(project::ItemParams::Graphic(
                                    project::GraphicParams::Star { .. },
                                )) => {
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec {
                                            field: "params.points",
                                            label: Text::GraphicPoints,
                                            min: 3.,
                                            max: 12.,
                                            step: 1.,
                                            pixels_per_step: 2.,
                                            decimals: 0,
                                            suffix: "",
                                        },
                                    );
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec {
                                            field: "params.depth",
                                            label: Text::GraphicDepth,
                                            min: 1.,
                                            max: 99.,
                                            step: 1.,
                                            pixels_per_step: 2.,
                                            decimals: 0,
                                            suffix: "%",
                                        },
                                    );
                                }
                                Some(project::ItemParams::Graphic(
                                    project::GraphicParams::Rectangle { .. },
                                )) => {
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec {
                                            field: "params.cornerRadius",
                                            label: Text::BackgroundRadius,
                                            min: 0.,
                                            max: 50.,
                                            step: 1.,
                                            pixels_per_step: 2.,
                                            decimals: 0,
                                            suffix: "%",
                                        },
                                    );
                                }
                                _ => {}
                            }
                        } else {
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
                        }
                    },
                );
            }
            // The tab list already drops a video without source audio.
            PropertiesTab::Audio if matches!(kind, ElementKind::Audio | ElementKind::Video) => {
                self.audio_section(ui, item);
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

mod playback;

mod effects;

mod text_style;

mod appearance;

mod mask;

mod number_edit;

mod remove_background;

mod selection;

mod font_dialog;

mod transition;
