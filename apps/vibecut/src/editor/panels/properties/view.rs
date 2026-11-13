//! Renders the properties area: a 42 px tab strip, then collapsible sections of rows.

use super::read;
use super::{
    Action, ColorTarget, Context, Rect, State, Text, Tick, Vec2, Widget, egui, element_kind, theme,
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

/// Width a row gives up at its right edge for the keyframe cluster: the cluster plus
/// the 10 px gap measured between the value stepper and the cluster (221703).
const KEYFRAME_RESERVE: f32 = theme::size::KEYFRAME_CLUSTER + theme::space::ROW_INSET;

/// What one keyframe cluster shows: where the item starts (neighbour keys are
/// item-relative), whether a key sits at the playhead and the keys on each side.
struct ClusterState {
    start: Tick,
    keyed: bool,
    can_toggle: bool,
    neighbours: read::Neighbours,
    /// Why the toggle cannot act, shown on hover while it cannot.
    blocked: Option<&'static str>,
}

/// The cluster's rectangle on the right edge of a row, centred on it.
fn keyframe_cluster_rect(row: Rect) -> Rect {
    Rect::from_min_size(
        egui::pos2(
            row.right() - theme::size::KEYFRAME_CLUSTER,
            row.center().y - theme::size::CONTROL / 2.0,
        ),
        Vec2::new(theme::size::KEYFRAME_CLUSTER, theme::size::CONTROL),
    )
}

/// The animatable fields of a row, one group per cluster (a position or scale pair keys together).
fn animatable_groups(specs: &[ScrubSpec]) -> Vec<(&'static str, Vec<ScrubSpec>)> {
    let mut groups: Vec<(&'static str, Vec<ScrubSpec>)> = Vec::new();
    for spec in specs.iter().copied().filter(|spec| {
        spec.field == "volume"
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
    }) {
        let key = diamond_group(spec.field);
        match groups.last_mut() {
            Some((last, group)) if *last == key => group.push(spec),
            _ => groups.push((key, vec![spec])),
        }
    }
    groups
}

/// The right edge a row's value controls stop at: clear of its cluster when it has one.
fn keyframe_reserve(specs: &[ScrubSpec]) -> f32 {
    KEYFRAME_RESERVE * animatable_groups(specs).len() as f32
}

fn is_basic(tab: PropertiesTab) -> bool {
    matches!(
        tab,
        PropertiesTab::Transform
            | PropertiesTab::Background
            | PropertiesTab::RemoveBackground
            | PropertiesTab::Mask
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
            // The tab that draws the field: graphic parameters on Graphic, a text item's
            // Opacity row inside its Text tab, a filter clip's Intensity (its opacity) on
            // Effects, every other Opacity row on Transform.
            let owner = if graphic_edit {
                PropertiesTab::Graphic
            } else if kind == ElementKind::Text {
                PropertiesTab::Text
            } else if kind == ElementKind::Effect {
                PropertiesTab::Effects
            } else {
                PropertiesTab::Transform
            };
            panel
                .cx
                .preferences
                .properties_tabs
                .get(kind.id())
                .map(String::as_str)
                .unwrap_or(kind.default_tab().id())
                == owner.id()
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
        let row = Rect::from_min_size(
            ui.next_widget_position(),
            Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
        );
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
        let enabled = target.inside(self.cx.session.project());
        let keyed = target.rgb_keys(self.cx.session.project(), true);
        let neighbours = target.key_neighbours(
            self.cx.session.project(),
            Tick(self.cx.playhead.0 - start.0),
        );
        let clicked = self.keyframe_cluster(
            ui,
            &format!("{id}.key"),
            keyframe_cluster_rect(row),
            ClusterState {
                start,
                keyed,
                can_toggle: enabled,
                neighbours,
                blocked: None,
            },
        );
        if clicked {
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
        let neighbours = read::param_neighbours(&param, Tick(self.cx.playhead.0 - target.start.0));
        let clicked = self.keyframe_cluster(
            ui,
            "inspector.graphic.strokeAlign.key",
            keyframe_cluster_rect(row),
            ClusterState {
                start: target.start,
                keyed: exists,
                can_toggle: inside,
                neighbours,
                blocked: None,
            },
        );
        if selected.is_none() && !clicked {
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
    fn set_reverse(&mut self, item: ItemId, reverse: bool) {
        self.actions.push(Action::SetReverse { item, reverse });
    }
    fn set_effect_param(&mut self, item: ItemId, effect: &str, key: &str, value: f64) {
        self.actions.push(Action::SetEffectParam {
            item,
            effect: effect.to_owned(),
            key: key.to_owned(),
            value,
        });
    }

    /// One keyframe cluster per animatable field group in this row, at the row's right edge.
    fn keyframe_diamonds(
        &mut self,
        ui: &mut egui::Ui,
        row: Rect,
        item: ItemId,
        specs: &[ScrubSpec],
    ) {
        let groups = animatable_groups(specs);
        if groups.is_empty() {
            return;
        }
        let Some((start, end)) = super::read::range_of(self.cx.session.project(), item) else {
            return;
        };
        let at = self.cx.playhead;
        let inside = start <= at
            && (at < end || (!specs.iter().any(|s| s.field.starts_with("params.")) && at == end));
        // Signed: a playhead before the item still navigates to its first key.
        let relative = Tick(at.0 - start.0);
        for (index, (_, group)) in groups.into_iter().enumerate() {
            let Some(lead) = group.first().map(|spec| spec.field) else {
                continue;
            };
            let id = format!("inspector.keyframe.{}", lead.replace("params.", "graphic."));
            let fields: Vec<&str> = group.iter().map(|spec| spec.field).collect();
            let keyed = group.iter().all(|spec| {
                super::read::has_key_at(self.cx.session.project(), item, spec.field, relative)
            });
            let neighbours =
                super::read::neighbour_keys(self.cx.session.project(), item, &fields, relative);
            let rect = keyframe_cluster_rect(row).translate(Vec2::new(
                -(index as f32) * theme::size::KEYFRAME_CLUSTER,
                0.0,
            ));
            let toggled = self.keyframe_cluster(
                ui,
                &id,
                rect,
                ClusterState {
                    start,
                    keyed,
                    can_toggle: inside,
                    neighbours,
                    blocked: None,
                },
            );
            if toggled && inside {
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

    /// The shared row-end cluster: draws the kit control, remembers its three cells for
    /// automation and queues a seek for a previous or next click. Returns whether the
    /// toggle was clicked; the caller owns what a toggle does.
    fn keyframe_cluster(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        rect: Rect,
        state: ClusterState,
    ) -> bool {
        let [previous, toggle, next] = kit::keyframe_cluster(
            ui,
            id,
            rect,
            &kit::KeyframeControl {
                previous_label: self.tr(Text::PreviousKeyframe),
                toggle_label: self.tr(if state.keyed {
                    Text::RemoveKeyframe
                } else {
                    Text::AddKeyframe
                }),
                next_label: self.tr(Text::NextKeyframe),
                keyed: state.keyed,
                can_toggle: state.can_toggle,
                has_previous: state.neighbours.previous.is_some(),
                has_next: state.neighbours.next.is_some(),
            },
        );
        let toggle = match state.blocked.filter(|_| !state.can_toggle) {
            Some(reason) => toggle.on_disabled_hover_text(reason),
            None => toggle,
        };
        self.remember(format!("{id}.previous"), &previous);
        self.remember_with_icon(id, Some(crate::kit::Icon::Diamond.name()), &toggle);
        self.remember(format!("{id}.next"), &next);
        if let Some(time) = state.neighbours.previous.filter(|_| previous.clicked()) {
            self.actions.push(Action::SeekToKey(state.start + time));
        }
        if let Some(time) = state.neighbours.next.filter(|_| next.clicked()) {
            self.actions.push(Action::SeekToKey(state.start + time));
        }
        state.can_toggle && toggle.clicked()
    }

    /// The project Details: name, path and imported media, a divider, then the canvas rows,
    /// the sample rate, the collapsed info section and `Modify`, which opens the Settings tab.
    fn project_details(&mut self, ui: &mut egui::Ui) {
        let locale = self.cx.locale;
        // Owned text first: the rows below may take `&mut self`.
        let (name, imported, aspect, resolution, frame_rate, sample_rate) = {
            let data = self.cx.session.project().data();
            let canvas = project::CanvasSize {
                width: data.canvas.width,
                height: data.canvas.height,
            };
            // The ratio label reads `Original` whenever the canvas is the retained media size.
            let aspect = if data.original_canvas_size == Some(canvas) {
                self.tr(Text::Original)
            } else {
                data.canvas
                    .preset
                    .map_or_else(|| self.tr(Text::Custom), |preset| self.preset_text(preset))
            };
            // The canvas follows the first media only in the Original preset.
            let resolution = if data.canvas.preset == Some(project::AspectPreset::Original) {
                self.tr(Text::DetailAdapted).to_owned()
            } else {
                let width = i18n::number(locale, u64::from(data.canvas.width));
                let height = i18n::number(locale, u64::from(data.canvas.height));
                i18n::format(
                    locale,
                    Text::CanvasSize,
                    &[("width", &width), ("height", &height)],
                )
            };
            // Two decimals for display only; the stored rate stays exact.
            let rate = i18n::decimal(locale, data.canvas.fps.as_f64(), 2);
            let frame_rate = i18n::format(locale, Text::FrameRate, &[("rate", &rate)]);
            let sample = i18n::number(locale, u64::from(data.settings.audio_sample_rate));
            let sample_rate = i18n::format(locale, Text::SampleRate, &[("rate", &sample)]);
            // Imported media is referenced by its path and never copied, so the file stays in
            // place. Ephemeral assets are internal (proxies, subtitle imports), not user media.
            let imported = if data.assets.iter().any(|asset| !asset.ephemeral()) {
                self.tr(Text::DetailStayInOriginal)
            } else {
                self.tr(Text::EmptyValue)
            };
            (
                data.name.as_str().to_owned(),
                imported,
                aspect,
                resolution,
                frame_rate,
                sample_rate,
            )
        };
        let path = self.cx.project_path.map(user_path);
        let modify = self.tr(Text::DetailModify);
        let fill = self.control_fill();
        egui::ScrollArea::vertical()
            .id_salt("inspector.details.rows")
            .show(ui, |ui| {
                self.detail_pair(ui, self.tr(Text::DetailName), &name);
                // An unsaved project has no path yet.
                if let Some(path) = &path {
                    self.detail_pair(ui, self.tr(Text::DetailPath), path);
                }
                self.detail_pair(ui, self.tr(Text::DetailImportedMedia), imported);
                ui.separator();
                self.detail_pair(ui, self.tr(Text::DetailAspectRatio), aspect);
                self.detail_pair(ui, self.tr(Text::DetailResolution), &resolution);
                self.detail_pair(ui, self.tr(Text::DetailFrameRate), &frame_rate);
                self.detail_row(ui, &sample_rate);
                self.project_info(ui);
                let response = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(egui::Button::new(modify).fill(fill))
                    })
                    .inner;
                self.remember("inspector.details.modify", &response);
                if response.clicked() {
                    self.state.request_canvas_focus();
                    // A collapsed canvas section would never draw the width that takes the focus.
                    if !self.cx.preferences.section_open(canvas::SECTION_KEY, true) {
                        self.actions.push(Action::SetSection {
                            key: canvas::SECTION_KEY,
                            open: true,
                        });
                    }
                    self.actions.push(Action::Activate("inspector.project"));
                }
            });
    }

    fn project_info(&mut self, ui: &mut egui::Ui) {
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

    /// A label on the left in the secondary colour and its value on the right; a long
    /// value (the project path) wraps inside the remaining width.
    fn detail_pair(&self, ui: &mut egui::Ui, label: &str, value: &str) {
        ui.horizontal_top(|ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(theme::type_size::BODY)
                    .color(self.secondary()),
            );
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(value)
                            .size(theme::type_size::BODY)
                            .color(self.text_color()),
                    )
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
            });
        });
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
        self.panel_section_with(ui, id, key, label, default_open, reset, None, body)
            .0
    }

    /// A section with an optional enable checkbox before its label (its label and checked
    /// state). Returns whether reset was clicked and whether the checkbox was.
    #[allow(clippy::too_many_arguments)]
    fn panel_section_with(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: &'static str,
        label: Text,
        default_open: bool,
        reset: bool,
        enable: Option<(Text, bool)>,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> (bool, bool) {
        let stored = self.cx.preferences.section_open(key, default_open);
        let mut open = stored;
        let label = self.tr(label);
        let reset_label = self.tr(Text::ResetValue);
        let header = kit::section_header_with(
            ui,
            id,
            label,
            &mut open,
            reset.then_some(reset_label),
            None,
            enable.map(|(text, checked)| (self.tr(text), checked)),
        );
        self.remember(id, &header.toggle);
        let clicked = header.reset.as_ref().is_some_and(|response| {
            self.remember(format!("{id}.reset"), response);
            response.clicked()
        });
        let enable_clicked = header.enable.as_ref().is_some_and(|response| {
            self.remember(format!("{id}.enable"), response);
            response.clicked()
        });
        if open != stored {
            self.actions.push(Action::SetSection { key, open });
        }
        if open {
            body(self, ui);
        }
        (clicked, enable_clicked)
    }

    /// The Blend checkbox: unchecking reverts the blend mode to Normal and remembers the
    /// mode; checking restores it, or picks the first non-Normal mode when none is remembered.
    fn toggle_blend(&mut self, item: ItemId) {
        let Some(current) = read::item(self.cx.session.project(), item).map(|i| i.blend_mode)
        else {
            return;
        };
        let next = if current == project::BlendMode::Normal {
            match self.state.blend_restore {
                Some((remembered, mode)) if remembered == item => mode,
                _ => project::BlendMode::ALL[1],
            }
        } else {
            self.state.blend_restore = Some((item, current));
            project::BlendMode::Normal
        };
        self.set_item_field(item, "blendMode", next.id());
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
            // The Background tab shows only while the clip leaves part of the canvas open.
            .filter(|tab| {
                *tab != PropertiesTab::Background
                    || read::clip_background_visible(
                        self.cx.session.project(),
                        item,
                        self.cx.playhead,
                    )
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

    /// The Text tab: Font, Format, Style, the Transform section every visual item shares,
    /// Blending, then Animation. Each section keeps its own collapse state.
    fn text_tab(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let fields = shared_fields(self.cx.session.project(), self.cx.selection());
        let font: Vec<_> = fields
            .iter()
            .filter(|field| matches!(field.id, "text_content" | "text_font"))
            .cloned()
            .collect();
        self.panel_section(
            ui,
            "inspector.section.text_font",
            "inspector.text_font",
            Text::TextFont,
            true,
            false,
            |panel, ui| panel.shared_field_controls(ui, font),
        );
        self.panel_section(
            ui,
            "inspector.section.text_format",
            "inspector.text_format",
            Text::TextFormat,
            true,
            false,
            |panel, ui| panel.text_format_controls(ui, item),
        );
        self.panel_section(
            ui,
            "inspector.section.text_style",
            "inspector.text_style",
            Text::TextStyleSection,
            true,
            false,
            |panel, ui| {
                panel.text_style_rows(ui);
                panel.text_style_controls(ui, item);
            },
        );
        self.opacity_row(ui, item);
        self.transform_section(ui, item);
        self.blending_section(ui, item, false);
        self.panel_section(
            ui,
            "inspector.section.text_animation",
            "inspector.text_animation",
            Text::TextAnimation,
            true,
            false,
            |panel, ui| panel.animation_controls(ui, item),
        );
    }

    /// The Blending section after Transform: the blend mode, and the opacity too where the
    /// tab has no Opacity row of its own. Its reset restores both.
    fn blending_section(&mut self, ui: &mut egui::Ui, item: ItemId, with_opacity: bool) {
        let blend_on = read::item(self.cx.session.project(), item)
            .is_some_and(|i| i.blend_mode != project::BlendMode::Normal);
        let (blend_reset, blend_toggle) = self.panel_section_with(
            ui,
            "inspector.section.blending",
            "inspector.blending",
            Text::Blending,
            true,
            true,
            Some((Text::BlendEnable, blend_on)),
            |panel, ui| {
                if with_opacity {
                    panel.blend_section(ui, item);
                } else {
                    panel.blend_control(ui, item);
                }
            },
        );
        if blend_toggle {
            self.toggle_blend(item);
        }
        if blend_reset {
            self.state.drafts.clear();
            self.state.focus = None;
            self.actions.push(Action::ResetBlend(item));
        }
    }

    /// The Transform section every visual item shares: Scale, Position, Rotate, Arrange and
    /// flips. Text items show it inside their Text tab; the other kinds show it in Transform.
    fn transform_section(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let reset = self.panel_section(
            ui,
            "inspector.section.transform",
            "inspector.transform",
            Text::Transform,
            true,
            true,
            |panel, ui| {
                use project::TransformField as F;
                panel.fit_control(ui, item);
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
                self.text_tab(ui, item);
            }
            PropertiesTab::Transform => {
                self.transform_section(ui, item);
                self.blending_section(ui, item, true);
                // The reference lists the switch among Basic's toggles; video items only.
                if let Some(project::ItemParams::Video(params)) =
                    read::item(self.cx.session.project(), item).map(|item| &item.content)
                {
                    let mut on = params.reduce_noise;
                    let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                    self.row_label(ui, row, self.tr(Text::ReduceImageNoise));
                    let width = value_ui.available_width();
                    value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
                    let response = kit::toggle(
                        &mut value_ui,
                        "inspector.reduce_noise",
                        &mut on,
                        self.tr(Text::ReduceImageNoise),
                    );
                    self.remember("inspector.reduce_noise", &response);
                    if response.clicked() {
                        self.set_item_field(
                            item,
                            "reduce_noise",
                            if on { "true" } else { "false" },
                        );
                    }
                }
                self.stabilize_rows(ui, item);
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
                                ScrubSpec::of(
                                    "graphic_stroke_width",
                                    Text::GraphicWidth,
                                    1.,
                                    2.,
                                    0,
                                ),
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
                                        ScrubSpec::of(
                                            "graphic_sides",
                                            Text::GraphicSides,
                                            1.,
                                            2.,
                                            0,
                                        ),
                                    );
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec::of(
                                            "graphic_corner_radius",
                                            Text::BackgroundRadius,
                                            1.,
                                            2.,
                                            0,
                                        ),
                                    );
                                }
                                Some(project::ItemParams::Graphic(
                                    project::GraphicParams::Star { .. },
                                )) => {
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec::of(
                                            "graphic_points",
                                            Text::GraphicPoints,
                                            1.,
                                            2.,
                                            0,
                                        ),
                                    );
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec::of(
                                            "graphic_depth",
                                            Text::GraphicDepth,
                                            1.,
                                            2.,
                                            0,
                                        ),
                                    );
                                }
                                Some(project::ItemParams::Graphic(
                                    project::GraphicParams::Rectangle { .. },
                                )) => {
                                    panel.scrub_field(
                                        ui,
                                        item,
                                        ScrubSpec::of(
                                            "graphic_corner_radius",
                                            Text::BackgroundRadius,
                                            1.,
                                            2.,
                                            0,
                                        ),
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
            PropertiesTab::Animation
                if matches!(
                    kind,
                    ElementKind::Video
                        | ElementKind::Image
                        | ElementKind::Sticker
                        | ElementKind::Graphic
                ) =>
            {
                self.panel_section(
                    ui,
                    "inspector.section.animation",
                    "inspector.animation",
                    Text::TextAnimation,
                    true,
                    false,
                    |panel, ui| panel.animation_controls(ui, item),
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
            PropertiesTab::Background if kind == ElementKind::Video => {
                self.background_controls(ui, item);
            }
            PropertiesTab::RemoveBackground
                if matches!(kind, ElementKind::Video | ElementKind::Image) =>
            {
                self.remove_background_controls(ui, item);
            }
            PropertiesTab::Effects if kind == ElementKind::Effect => {
                self.filter_section(ui, item);
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

mod animation;
pub(super) use animation::Previews as AnimationPreviews;

mod canvas;

mod playback;

mod effects;

mod filter;

mod text_style;

/// The path as the creator typed it: a saved path is canonical, and on Windows that adds the
/// verbatim `\\?\` prefix, which the Details row leaves out.
fn user_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(disk) if disk.as_bytes().get(1) == Some(&b':') => disk.to_owned(),
        _ => text,
    }
}

mod appearance;

mod number_edit;

mod basic;

mod footage;

mod selection;

mod transition;
