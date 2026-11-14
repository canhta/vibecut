//! Docks the agent chat beside the timeline and marks a previewed change as read only.
use crate::editor::{Editor, Icon, Rect, Text, Vec2, egui, panels::agent, theme};

impl Editor {
    /// Paint the chat in its dock when it is open, otherwise the button that opens it, and
    /// route what the chat asked for. While a change is previewed both surfaces carry a mark.
    pub(in crate::editor) fn show_agent(
        &mut self,
        ui: &mut egui::Ui,
        dock: Option<Rect>,
        timeline: Rect,
        preview: Rect,
    ) {
        let dark = self.preferences.dark;
        match dock {
            Some(area) => {
                self.frame.panels.insert("agent", area);
                ui.painter()
                    .rect_filled(area, theme::radius::FRAME_PANEL, theme::panel(dark));
                let mut child =
                    ui.new_child(egui::UiBuilder::new().id_salt("agent").max_rect(area));
                child.set_clip_rect(area);
                let mut state = std::mem::take(&mut self.agent);
                let cx = self.context();
                let actions = agent::show(&mut state, &cx, &mut child);
                drop(cx);
                self.agent = state;
                for action in actions {
                    match action {
                        agent::Action::Widget(id, response) => self.remember(id, &response),
                        other => {
                            self.with_agent(|state, port| agent::apply(state, port, other));
                        }
                    }
                }
            }
            None => {
                let side = theme::size::CONTROL;
                // The button's place is unmeasured: the reference shows only the open panel.
                let rect = Rect::from_min_size(
                    egui::pos2(
                        timeline.right() - theme::space::MD - side,
                        timeline.bottom() - theme::size::TIMELINE_FOOT - theme::space::XS - side,
                    ),
                    Vec2::splat(side),
                );
                let label = self.tr(Text::AgentOpen);
                let response = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        theme::icon_button(ui, "agent.open", Icon::Sparkle, label, false)
                    })
                    .inner;
                if response.clicked() {
                    self.agent.open();
                }
                self.remember("agent.open", &response);
            }
        }
        if self.agent.previewing() {
            let text = self.tr(Text::AgentPreviewing);
            for (id, area) in [
                ("agent.preview.player", preview),
                ("agent.preview.timeline", timeline),
            ] {
                mark(
                    ui.ctx(),
                    id,
                    area.left_top() + Vec2::splat(theme::space::MD),
                    text,
                    dark,
                );
            }
        }
    }
}

/// A small accent tag over a surface that shows the agent's changes instead of the project.
fn mark(ctx: &egui::Context, id: &'static str, at: egui::Pos2, text: &str, dark: bool) {
    egui::Area::new(egui::Id::unique(id))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::accent(dark))
                .corner_radius(theme::radius::CONTROL)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(theme::type_size::BODY)
                            .color(theme::on_accent(dark)),
                    );
                });
        });
}
