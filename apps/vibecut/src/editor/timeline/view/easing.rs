//! Draws the keyframe graph popover opened from the timeline toolbar.
//!
//! Every change is staged as a preview inside one engine gesture: Apply
//! publishes it as one undo step, Cancel and Escape restore the pre-open state.

use crate::editor::timeline::graph::{self, GraphPopover, MARGIN, PLOT};
use crate::editor::timeline::{
    Action, Align2, Context, Id, KeyframeSelection, Rect, State, Text, Vec2, egui, theme,
    view::{Ink, tr},
};
use crate::preferences::{EasingTab, MAX_SAVED_EASINGS};
use project::{EasingCurve, EasingPreset};

const FOOTER_GAP: f32 = theme::space::XS;

fn preset_text(preset: EasingPreset) -> Text {
    match preset {
        EasingPreset::Smooth => Text::EasingSmooth,
        EasingPreset::EaseOut => Text::EasingEaseOut,
        EasingPreset::EaseIn => Text::EasingEaseIn,
        EasingPreset::InOut => Text::EasingInOut,
        EasingPreset::Pop => Text::EasingPop,
        EasingPreset::Linear => Text::EasingLinear,
    }
}

impl State {
    /// Open the popover on the selected keyframes, or close it when already open.
    pub(in crate::editor::timeline) fn toggle_graph(
        &mut self,
        cx: &Context<'_>,
        anchor: egui::Pos2,
    ) {
        if self.graph_popover.is_some() {
            self.escape_graph();
            return;
        }
        let keys: Vec<KeyframeSelection> = cx.keyframes.iter().cloned().collect();
        if graph::segments(cx.staged_project(), &keys).is_empty() {
            return;
        }
        self.graph_popover = Some(GraphPopover {
            anchor,
            keys,
            tab: 0,
            grab: None,
            staged: false,
        });
    }

    /// Whether the selection has a segment the popover can ease.
    pub(in crate::editor::timeline) fn graph_available(&self, cx: &Context<'_>) -> bool {
        let keys: Vec<KeyframeSelection> = cx.keyframes.iter().cloned().collect();
        !graph::segments(cx.staged_project(), &keys).is_empty()
    }

    /// Draw the open popover. It returns commands and gesture requests only.
    pub(in crate::editor::timeline) fn show_graph_popover(
        &mut self,
        cx: &Context<'_>,
        ctx: &egui::Context,
    ) {
        let Some(mut open) = self.graph_popover.take() else {
            return;
        };
        // Another interaction discarded the staged previews: nothing left to edit.
        if open.staged && !cx.session.gesture_active() {
            return;
        }
        let segments = graph::segments(cx.staged_project(), &open.keys);
        if segments.is_empty() {
            if open.staged {
                self.outbox.push(Action::DiscardGesture);
            }
            return;
        }
        let components = graph::components(&segments);
        open.tab = open.tab.min(components.len().saturating_sub(1));
        let active = components.get(open.tab).copied().flatten();
        let tab_segments: Vec<(KeyframeSelection, EasingCurve)> = segments
            .iter()
            .filter(|(key, _)| key.component == active)
            .cloned()
            .collect();
        let mut curve = tab_segments
            .first()
            .map_or(EasingCurve::LINEAR, |(_, curve)| *curve);
        let tab_keys = graph::refs(tab_segments.iter().map(|(key, _)| key));
        let all_keys = graph::refs(segments.iter().map(|(key, _)| key));

        let ink = Ink::of(cx);
        let mut actions = Vec::new();
        let mut widgets: Vec<(String, egui::Response)> = Vec::new();
        let mut staged = Vec::new();
        let mut close = false;
        let window = egui::Window::new(tr(cx, Text::GraphEditor))
            .id(Id::unique("timeline.graph.popover"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .pivot(Align2::LEFT_TOP)
            .fixed_pos(open.anchor)
            .frame(
                egui::Frame::window(&ctx.style_of(if cx.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                }))
                .inner_margin(egui::Margin::same(theme::space::LG as i8)),
            )
            .constrain_to(ctx.content_rect().shrink(theme::space::MD));
        window.show(ctx, |ui| {
            ui.set_width(PLOT + 2.0 * MARGIN);
            ui.spacing_mut().item_spacing.y = theme::space::XS;
            ui.label(egui::RichText::new(tr(cx, Text::GraphEditor)).strong());

            if components.len() > 1 {
                ui.horizontal(|ui| {
                    for (index, component) in components.iter().enumerate() {
                        let label = component.map_or_else(|| "·".to_owned(), |c| format!("{c:?}"));
                        let tab = ui.selectable_label(index == open.tab, label);
                        record(
                            &mut widgets,
                            &format!("timeline.graph.component.{index}"),
                            &tab,
                        );
                        if tab.clicked() {
                            open.tab = index;
                        }
                    }
                });
            }

            let (outer, response) = ui.allocate_exact_size(
                Vec2::splat(PLOT + 2.0 * MARGIN),
                egui::Sense::click_and_drag(),
            );
            let plot: Rect = outer.shrink(MARGIN);
            record(&mut widgets, "timeline.graph.plot", &response);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Slider, true, tr(cx, Text::GraphEditor))
            });
            if response.drag_started()
                && let Some(point) = ui
                    .input(|input| input.pointer.press_origin())
                    .or_else(|| response.interact_pointer_pos())
            {
                // The press, not the first moved position, decides which handle is held.
                open.grab = graph::nearest(plot, curve, point);
            }
            if let (Some(handle), true, Some(point)) = (
                open.grab,
                response.dragged(),
                response.interact_pointer_pos(),
            ) {
                let next = graph::dragged(plot, curve, handle, point);
                if next != curve {
                    curve = next;
                    staged.push(engine::Command::SetKeyframeCurve {
                        keys: tab_keys.clone(),
                        curve,
                    });
                }
            }
            if response.drag_stopped() {
                open.grab = None;
            }
            paint_plot(ui, &ink, plot, curve);

            ui.horizontal(|ui| {
                for (tab, text, id) in [
                    (
                        EasingTab::Presets,
                        Text::GraphPresets,
                        "timeline.graph.tab.presets",
                    ),
                    (
                        EasingTab::Saved,
                        Text::GraphSaved,
                        "timeline.graph.tab.saved",
                    ),
                ] {
                    let button =
                        ui.selectable_label(cx.preferences.easing.tab == tab, tr(cx, text));
                    record(&mut widgets, id, &button);
                    if button.clicked() {
                        actions.push(Action::SetEasingTab(tab));
                    }
                }
            });
            match cx.preferences.easing.tab {
                EasingTab::Presets => {
                    ui.horizontal_wrapped(|ui| {
                        for preset in EasingPreset::ALL {
                            let button = ui.button(tr(cx, preset_text(preset)));
                            record(
                                &mut widgets,
                                &format!("timeline.graph.preset.{}", preset.id()),
                                &button,
                            );
                            if button.clicked() {
                                curve = preset.curve();
                                staged.push(engine::Command::ApplyEasingPreset {
                                    keys: all_keys.clone(),
                                    preset,
                                });
                            }
                        }
                    });
                }
                EasingTab::Saved => {
                    ui.horizontal_wrapped(|ui| {
                        for (index, saved) in cx.preferences.easing.saved.iter().enumerate() {
                            let button = ui.button(saved.name.as_str());
                            record(
                                &mut widgets,
                                &format!("timeline.graph.saved.{index}"),
                                &button,
                            );
                            if button.clicked() {
                                curve = EasingCurve(saved.curve);
                                staged.push(engine::Command::SetKeyframeCurve {
                                    keys: all_keys.clone(),
                                    curve,
                                });
                            }
                        }
                    });
                }
            }

            ui.add_space(FOOTER_GAP);
            ui.horizontal(|ui| {
                let known = cx
                    .preferences
                    .easing
                    .saved
                    .iter()
                    .any(|saved| saved.curve == curve.0);
                let room = cx.preferences.easing.saved.len() < MAX_SAVED_EASINGS;
                let save = ui.add_enabled(
                    room && !known,
                    egui::Button::new(tr(cx, Text::GraphSavePreset)),
                );
                record(&mut widgets, "timeline.graph.save", &save);
                if save.clicked() {
                    actions.push(Action::SaveEasing(curve.0));
                }
                let apply = ui.button(tr(cx, Text::GraphApply));
                record(&mut widgets, "timeline.graph.apply", &apply);
                if apply.clicked() {
                    close = true;
                    actions.push(Action::EndGesture);
                }
                let cancel = ui.button(tr(cx, Text::Cancel));
                record(&mut widgets, "timeline.graph.cancel", &cancel);
                if cancel.clicked() {
                    staged.clear();
                    close = true;
                    if open.staged {
                        actions.push(Action::DiscardGesture);
                    }
                }
            });
        });

        // Stage in order ahead of any Apply: begin the gesture on first use.
        let mut requests = Vec::new();
        for command in staged {
            if !open.staged {
                requests.push(Action::BeginGesture { select: None });
                open.staged = true;
            }
            requests.push(Action::Preview(Box::new(command)));
        }
        // Apply with nothing staged has no gesture to end.
        requests.extend(
            actions
                .into_iter()
                .filter(|action| !matches!(action, Action::EndGesture) || open.staged),
        );
        for (id, response) in widgets {
            self.remember(id, &response);
        }
        self.outbox.extend(requests);
        if !close {
            self.graph_popover = Some(open);
        }
    }
}

fn paint_plot(ui: &egui::Ui, ink: &Ink, plot: Rect, curve: EasingCurve) {
    let painter = ui.painter();
    painter.rect_filled(plot, 0.0, ink.field());
    let border = egui::Stroke::new(1.0, ink.border());
    for (from, to) in [
        (plot.left_top(), plot.right_top()),
        (plot.right_top(), plot.right_bottom()),
        (plot.right_bottom(), plot.left_bottom()),
        (plot.left_bottom(), plot.left_top()),
    ] {
        painter.line_segment([from, to], border);
    }
    // Start and end value guides.
    for y in [0.0, 1.0] {
        painter.line_segment(
            [graph::to_point(plot, 0.0, y), graph::to_point(plot, 1.0, y)],
            egui::Stroke::new(1.0, ink.secondary().gamma_multiply(0.5)),
        );
    }
    let [x1, y1, x2, y2] = curve.0;
    let start = graph::to_point(plot, 0.0, 0.0);
    let end = graph::to_point(plot, 1.0, 1.0);
    let out = graph::to_point(plot, x1, y1);
    let incoming = graph::to_point(plot, x2, y2);
    let guide = egui::Stroke::new(1.0, ink.secondary());
    painter.line_segment([start, out], guide);
    painter.line_segment([end, incoming], guide);
    painter.add(egui::Shape::line(
        graph::outline(plot, curve),
        egui::Stroke::new(2.0, ink.accent()),
    ));
    for knob in [out, incoming] {
        painter.circle_filled(knob, 5.0, ink.accent());
    }
}

/// Register `response` under its stable automation id.
fn record(widgets: &mut Vec<(String, egui::Response)>, id: &str, response: &egui::Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
    widgets.push((id.to_owned(), response.clone()));
}
