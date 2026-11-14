//! Paints an item's expanded keyframe lanes and their diamond interactions.
//!
//! Lanes are the panel's only keyframe surface: click selects, drag retimes with
//! timeline snapping, an empty-lane marquee selects, the diamond menu owns the easing
//! choice, and the lane menu owns the before/after extrapolation choice.

use crate::i18n;
use crate::{
    editor::timeline::{
        Action, Align2, BTreeSet, Context, FontId, Id, ItemId, ItemView, KeyframeSelection, Rect,
        State, Tick, TimelineLayout, Vec2, egui, layout::LANE_HEIGHT, theme, view::Ink,
    },
    i18n::Text,
};

impl State {
    pub(in crate::editor::timeline) fn paint_lanes(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        items: &[ItemView],
        modifiers: egui::Modifiers,
    ) {
        let ink = Ink::of(cx);
        let dark = ink.dark();
        let mut keyframe_rects: Vec<(KeyframeSelection, Rect)> = Vec::new();
        // The first diamond painted anchors the first-use hint.
        let mut hint: Option<egui::Pos2> = None;
        let lane_fill = theme::field(dark).gamma_multiply(0.55);
        let painter = ui.painter().with_clip_rect(layout.body);
        for item in items {
            for lane in &item.lanes {
                let lane_rect = Rect::from_min_max(
                    egui::pos2(layout.body.left(), lane.top),
                    egui::pos2(layout.body.right(), lane.top + LANE_HEIGHT),
                )
                .intersect(layout.body);
                if !lane_rect.is_positive() {
                    continue;
                }
                painter.rect_filled(lane_rect, 0.0, lane_fill);
                painter.hline(
                    lane_rect.x_range(),
                    lane_rect.top(),
                    egui::Stroke::new(1.0, theme::separator(dark)),
                );
                let start_x = self.x_at(item.start, layout.body.left());
                let end_x = self.x_at(item.end, layout.body.left());
                painter.line_segment(
                    [
                        egui::pos2(start_x.max(lane_rect.left()), lane_rect.center().y),
                        egui::pos2(end_x.min(lane_rect.right()), lane_rect.center().y),
                    ],
                    egui::Stroke::new(1.0, theme::border(dark).gamma_multiply(0.7)),
                );
                painter.text(
                    egui::pos2(lane_rect.left() + 6.0, lane_rect.center().y),
                    Align2::LEFT_CENTER,
                    &lane.label,
                    FontId::proportional(theme::type_size::BADGE),
                    ink.secondary(),
                );
                let response = ui.interact(
                    lane_rect,
                    Id::unique((
                        "timeline.lane",
                        item.id.0,
                        lane.path.as_str(),
                        component_id(lane.component),
                    )),
                    egui::Sense::click_and_drag(),
                );
                self.remember(
                    format!(
                        "timeline.lane.{}.{}",
                        item.id.0,
                        lane.path.as_str().replace('.', "_")
                    ),
                    &response,
                );
                self.lane_menu(cx, ui, item, lane, &response);
                let first = self.paint_lane_keys(cx, ui, layout, item, lane, &mut keyframe_rects);
                hint = hint.or(first);
                self.lane_marquee(cx, ui, layout, &response, &keyframe_rects, modifiers);
            }
        }
        self.keyframe_rects = keyframe_rects;
        if let Some(anchor) = hint
            && !cx.preferences.keyframe_hint_seen
        {
            self.keyframe_hint(cx, ui, anchor);
        }
    }

    fn paint_lane_keys(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        lane: &crate::editor::timeline::layout::ItemLane,
        keyframe_rects: &mut Vec<(KeyframeSelection, Rect)>,
    ) -> Option<egui::Pos2> {
        let ink = Ink::of(cx);
        let param = item_param(cx, item.id, &lane.path)?;
        let channel = channel_of(param, lane.component)?;
        // The first diamond painted anchors the first-use hint.
        let mut hint = None;
        for key in channel.keys() {
            let Some(at) = item.start.checked_add(key.time) else {
                continue;
            };
            let x = self.x_at(at, layout.body.left());
            if x < layout.body.left() - 6.0 || x > layout.body.right() + 6.0 {
                continue;
            }
            let center = egui::pos2(x, lane.top + LANE_HEIGHT / 2.0);
            hint = hint.or(Some(egui::pos2(center.x + 10.0, center.y)));
            let selection = KeyframeSelection {
                item: item.id,
                path: lane.path.clone(),
                component: lane.component,
                time: key.time,
            };
            let selected = cx.keyframes.contains(&selection);
            paint_diamond(ui.painter(), center, selected, &ink);
            let hit =
                Rect::from_center_size(center, Vec2::new(14.0, LANE_HEIGHT)).intersect(layout.body);
            let response = ui.interact(
                hit,
                Id::unique((
                    "timeline.keyframe",
                    item.id.0,
                    lane.path.as_str(),
                    component_id(lane.component),
                    key.time.0,
                )),
                egui::Sense::click_and_drag(),
            );
            self.remember(
                format!(
                    "timeline.keyframe.{}.{}.{}.{}",
                    item.id.0,
                    lane.path.as_str().replace('.', "_"),
                    component_id(lane.component),
                    key.time.0
                ),
                &response,
            );
            keyframe_rects.push((selection.clone(), hit));
            if response.clicked() {
                self.push(Action::SelectKeyframe {
                    keyframe: selection.clone(),
                    modifiers: ui.input(|input| input.modifiers),
                });
            }
            if response.drag_started() && self.keyframe_drag.is_none() {
                self.begin_keyframe_drag(cx, ui, item, selection.clone());
            }
            let anchor = egui::pos2(center.x, lane.top + LANE_HEIGHT + theme::space::XS);
            self.key_menu(cx, &response, channel, &selection, anchor);
        }
        hint
    }

    /// Right-click menu for one diamond: the easing kinds of its outgoing segment, the
    /// graph popover, then Copy and Delete. Paste stays on the lane menu.
    fn key_menu(
        &mut self,
        cx: &Context<'_>,
        response: &egui::Response,
        channel: &project::Channel,
        selection: &KeyframeSelection,
        anchor: egui::Pos2,
    ) {
        let locale = cx.locale;
        // Easing reads the segment leaving the key, so the last key offers none.
        let outgoing = channel
            .keys()
            .last()
            .is_some_and(|last| last.time > selection.time);
        let linear = channel
            .segment_curve(selection.time)
            .is_some_and(|curve| curve.is_linear());
        let keys = vec![engine::KeyframeRef {
            item: selection.item,
            path: selection.path.clone(),
            component: selection.component,
            time: selection.time,
        }];
        response.context_menu(|ui| {
            for (id, label, preset) in [
                ("ease_in", Text::EasingEaseIn, project::EasingPreset::EaseIn),
                (
                    "ease_out",
                    Text::EasingEaseOut,
                    project::EasingPreset::EaseOut,
                ),
            ] {
                let button = egui::Button::new(i18n::text(locale, label));
                let item = ui.add_enabled(outgoing, button);
                self.remember(format!("timeline.key_menu.{id}"), &item);
                if item.clicked() {
                    self.push(Action::Command(Box::new(
                        engine::Command::ApplyEasingPreset {
                            keys: keys.clone(),
                            preset,
                        },
                    )));
                    ui.close();
                }
            }
            let easing = egui::Button::new(i18n::text(locale, Text::KeyframeEasing));
            let item = ui.add_enabled(outgoing, easing);
            self.remember("timeline.key_menu.easing".to_owned(), &item);
            if item.clicked() {
                self.open_graph(cx, vec![selection.clone()], anchor);
                ui.close();
            }
            let line = egui::Button::selectable(linear, i18n::text(locale, Text::EasingLinear));
            let item = ui.add_enabled(outgoing, line);
            self.remember("timeline.key_menu.linear".to_owned(), &item);
            if item.clicked() {
                self.push(Action::Command(Box::new(
                    engine::Command::ApplyEasingPreset {
                        keys: keys.clone(),
                        preset: project::EasingPreset::Linear,
                    },
                )));
                ui.close();
            }
            let presets = egui::Button::new(i18n::text(locale, Text::KeyframeShowPresets));
            let item = ui.add_enabled(outgoing, presets);
            self.remember("timeline.key_menu.show_presets".to_owned(), &item);
            if item.clicked() {
                self.push(Action::SetEasingTab(crate::preferences::EasingTab::Presets));
                self.open_graph(cx, vec![selection.clone()], anchor);
                ui.close();
            }
            ui.separator();
            let copy = ui.button(i18n::text(locale, Text::ActionEditCopy));
            self.remember("timeline.key_menu.copy".to_owned(), &copy);
            if copy.clicked() {
                self.push(Action::SelectKeyframe {
                    keyframe: selection.clone(),
                    modifiers: egui::Modifiers::NONE,
                });
                self.push(Action::Activate("edit.copy".to_owned()));
                ui.close();
            }
            let delete = ui.button(i18n::text(locale, Text::DeleteKeyframes));
            self.remember("timeline.key_menu.delete".to_owned(), &delete);
            if delete.clicked() {
                self.push(Action::DeleteKeyframes(BTreeSet::from([selection.clone()])));
                ui.close();
            }
        });
    }

    /// The first-use hint, drawn on the first diamond's own row until it is dismissed. The
    /// card passes clicks through to the lanes beneath it; only OK takes them.
    fn keyframe_hint(&mut self, cx: &Context<'_>, ui: &egui::Ui, anchor: egui::Pos2) {
        let locale = cx.locale;
        let area = egui::Area::new(egui::Id::unique("timeline.keyframe_hint"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .pivot(Align2::LEFT_CENTER)
            .fixed_pos(anchor)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(i18n::text(locale, Text::KeyframeHint))
                                    .size(theme::type_size::BODY)
                                    .color(theme::text(cx.dark)),
                            );
                            ui.button(i18n::text(locale, Text::Ok))
                        })
                        .inner
                    })
                    .inner
            });
        self.remember("timeline.keyframe_hint.ok".to_owned(), &area.inner);
        if area.inner.clicked() {
            self.push(Action::DismissKeyframeHint);
        }
    }

    /// Right-click menu for one lane: the before/after extrapolation choice.
    fn lane_menu(
        &mut self,
        cx: &Context<'_>,
        _ui: &egui::Ui,
        item: &ItemView,
        lane: &crate::editor::timeline::layout::ItemLane,
        response: &egui::Response,
    ) {
        let Some(channel) =
            item_param(cx, item.id, &lane.path).and_then(|param| channel_of(param, lane.component))
        else {
            return;
        };
        let current = channel.extrapolation();
        let locale = cx.locale;
        response.context_menu(|ui| {
            for (action, suffix, label, icon) in [
                (
                    "edit.copy",
                    "copy",
                    Text::ActionEditCopy,
                    crate::kit::Icon::Copy,
                ),
                (
                    "edit.paste",
                    "paste",
                    Text::ActionEditPaste,
                    crate::kit::Icon::Copy,
                ),
            ] {
                let enabled = !cx.disabled.contains(action)
                    && cx.selection().len() == 1
                    && cx.selection().contains(&item.id);
                let response = ui.add_enabled(
                    enabled,
                    egui::Button::image_and_text(
                        crate::kit::tinted(icon, 16.0, theme::text(cx.dark)),
                        i18n::text(locale, label),
                    ),
                );
                self.remember_icon(
                    format!(
                        "timeline.lane.{}.{}.{}.{}",
                        item.id.0,
                        lane.path.as_str().replace('.', "_"),
                        component_id(lane.component),
                        suffix
                    ),
                    Some(icon.name()),
                    &response,
                );
                if response.clicked() {
                    self.push(Action::Activate(action.to_owned()));
                    ui.close();
                }
            }
            ui.separator();
            ui.label(
                egui::RichText::new(&lane.label)
                    .size(theme::type_size::BODY)
                    .color(theme::text(cx.dark)),
            );
            for side in [Side::Before, Side::After] {
                ui.menu_button(side.label(locale), |ui| {
                    for mode in [project::Extrapolation::Hold, project::Extrapolation::Linear] {
                        let chosen = match side {
                            Side::Before => current.0,
                            Side::After => current.1,
                        };
                        let response = ui.selectable_label(chosen == mode, mode.label(locale));
                        self.remember(
                            format!(
                                "timeline.lane.{}.{}.{}.{}.{}",
                                item.id.0,
                                lane.path.as_str().replace('.', "_"),
                                component_id(lane.component),
                                side.id(),
                                mode.id()
                            ),
                            &response,
                        );
                        if response.clicked() {
                            let (before, after) = match side {
                                Side::Before => (mode, current.1),
                                Side::After => (current.0, mode),
                            };
                            self.push(Action::Command(Box::new(
                                engine::Command::SetKeyframeExtrapolation {
                                    item: item.id,
                                    path: lane.path.clone(),
                                    component: lane.component,
                                    before,
                                    after,
                                },
                            )));
                            ui.close();
                        }
                    }
                });
            }
            if !cx.keyframes.is_empty() {
                let response = ui.button(i18n::text(locale, Text::DeleteKeyframes));
                self.remember(
                    format!(
                        "timeline.lane.{}.{}.delete",
                        item.id.0,
                        lane.path.as_str().replace('.', "_")
                    ),
                    &response,
                );
                if response.clicked() {
                    self.push(Action::DeleteKeyframes(cx.keyframes.clone()));
                    ui.close();
                }
            }
        });
    }

    fn begin_keyframe_drag(
        &mut self,
        cx: &Context<'_>,
        _ui: &egui::Ui,
        item: &ItemView,
        selection: KeyframeSelection,
    ) {
        self.push(Action::SelectKeyframe {
            keyframe: selection.clone(),
            modifiers: egui::Modifiers::NONE,
        });
        self.push(Action::BeginGesture { select: None });
        let Some(item) = cx
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item.id)
        else {
            return;
        };
        let Ok(end) = item.end() else {
            return;
        };
        self.keyframe_drag = Some(crate::editor::timeline::state::KeyframeDrag {
            selection: selection.clone(),
            current: selection.time,
            item_start: item.start,
            item_end: end,
        });
    }

    /// Drive the retime preview from global pointer state and commit one undo step.
    pub(in crate::editor::timeline) fn advance_keyframe_drag(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let Some(drag) = self.keyframe_drag.clone() else {
            return;
        };
        let (pointer, down) = ui
            .ctx()
            .input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        if let Some(pointer) = pointer {
            let raw = self.tick_at(pointer.x, layout.body.left());
            let bypass = ui.input(|input| input.modifiers.shift);
            let exclude: BTreeSet<ItemId> = [drag.selection.item].into_iter().collect();
            let (snapped, indicator) = self.snap_tick(cx, raw, &exclude, None, bypass);
            let target = snapped.clamp(drag.item_start, drag.item_end);
            let Some(target) = target.checked_sub(drag.item_start) else {
                return;
            };
            self.snap_indicator = indicator.filter(|_| target == snapped - drag.item_start);
            if target != drag.current && !key_exists(cx, &drag.selection, target) {
                self.push(Action::Preview(Box::new(engine::Command::RetimeKeyframe {
                    item: drag.selection.item,
                    path: drag.selection.path.clone(),
                    component: drag.selection.component,
                    from: drag.current,
                    to: target,
                })));
                if let Some(state) = &mut self.keyframe_drag {
                    state.current = target;
                }
                self.push(Action::SelectKeyframe {
                    keyframe: KeyframeSelection {
                        time: target,
                        ..drag.selection.clone()
                    },
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if !down {
            if self.keyframe_drag.take().is_some() {
                self.push(Action::EndGesture);
            }
            self.snap_indicator = None;
        }
    }
}

/// One lane-extrapolation side, named for the menu and for its automation id.
#[derive(Clone, Copy)]
enum Side {
    Before,
    After,
}
impl Side {
    fn id(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
        }
    }
    fn label(self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Before => i18n::text(locale, Text::BeforeKeyframes),
            Self::After => i18n::text(locale, Text::AfterKeyframes),
        }
    }
}
trait ExtrapolationLabel {
    fn id(self) -> &'static str;
    fn label(self, locale: crate::i18n::Locale) -> &'static str;
}
impl ExtrapolationLabel for project::Extrapolation {
    fn id(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Linear => "linear",
        }
    }
    fn label(self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Hold => i18n::text(locale, Text::HoldKeyframes),
            Self::Linear => i18n::text(locale, Text::LinearKeyframes),
        }
    }
}

fn component_id(component: Option<project::ColorComponent>) -> &'static str {
    match component {
        None => "none",
        Some(project::ColorComponent::R) => "r",
        Some(project::ColorComponent::G) => "g",
        Some(project::ColorComponent::B) => "b",
        Some(project::ColorComponent::A) => "a",
    }
}
pub(super) fn channel_of(
    param: &project::Param,
    component: Option<project::ColorComponent>,
) -> Option<&project::Channel> {
    match (param, component) {
        (project::Param::Value(channel), None) => Some(channel),
        (project::Param::Color(channels), Some(component)) => {
            let index = match component {
                project::ColorComponent::R => 0,
                project::ColorComponent::G => 1,
                project::ColorComponent::B => 2,
                project::ColorComponent::A => 3,
            };
            channels.get(index)
        }
        _ => None,
    }
}
fn item_param<'a>(
    cx: &'a Context<'_>,
    item: ItemId,
    path: &project::AnimationPath,
) -> Option<&'a project::Param> {
    cx.session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| candidate.animations.get(path))
}
fn key_exists(cx: &Context<'_>, selection: &KeyframeSelection, time: Tick) -> bool {
    item_param(cx, selection.item, &selection.path)
        .and_then(|param| channel_of(param, selection.component))
        .is_some_and(|channel| {
            channel
                .keys()
                .binary_search_by_key(&time, |key| key.time)
                .is_ok()
        })
}

impl State {
    fn lane_marquee(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        response: &egui::Response,
        keyframe_rects: &[(KeyframeSelection, Rect)],
        modifiers: egui::Modifiers,
    ) {
        if response.drag_started()
            && self.keyframe_marquee.is_none()
            && self.keyframe_drag.is_none()
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
        {
            self.keyframe_marquee = Some(crate::editor::timeline::state::KeyframeMarquee {
                origin,
                before: cx.keyframes.clone(),
                additive: modifiers.command || modifiers.ctrl || modifiers.shift,
            });
        }
        if let Some(marquee) = &self.keyframe_marquee
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(marquee.origin, pointer).intersect(layout.body);
            let mut selected: BTreeSet<KeyframeSelection> = keyframe_rects
                .iter()
                .filter(|(_, hit)| hit.intersects(rect))
                .map(|(selection, _)| selection.clone())
                .collect();
            if marquee.additive {
                selected.extend(marquee.before.iter().cloned());
            }
            self.push(Action::MarqueeKeyframes(selected));
            let ink = Ink::of(cx);
            ui.painter().with_clip_rect(layout.body).rect_filled(
                rect,
                0.0,
                ink.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(layout.body).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, ink.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if response.drag_stopped() && self.keyframe_marquee.take().is_some() {
            self.push(Action::SettleKeyframes);
        }
        if response.clicked() && self.keyframe_marquee.is_none() && self.keyframe_drag.is_none() {
            self.push(Action::ClearKeyframes);
        }
    }
}

fn paint_diamond(painter: &egui::Painter, center: egui::Pos2, selected: bool, ink: &Ink) {
    let half = if selected { 5.0 } else { 4.0 };
    let points = vec![
        center - egui::vec2(half, 0.0),
        center - egui::vec2(0.0, half),
        center + egui::vec2(half, 0.0),
        center + egui::vec2(0.0, half),
    ];
    if selected {
        painter.add(egui::Shape::convex_polygon(
            points,
            ink.accent(),
            egui::Stroke::new(1.0, ink.accent()),
        ));
    } else {
        painter.add(egui::Shape::closed_line(
            points,
            egui::Stroke::new(1.0, ink.text()),
        ));
    }
}
