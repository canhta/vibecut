use crate::{
    Error, Result,
    catalog::{self, Locale, Preferences, Text},
    egui::{self, Color32, Id, Rect, Vec2},
    playback::Playback,
};
use engine::Session;
use std::collections::BTreeMap;
use time::{Rounding, Tick};

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub id: &'static str,
    pub enabled: bool,
    pub focused: bool,
    pub rect: Rect,
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub widgets: Vec<WidgetState>,
    pub view: &'static str,
    pub playing: bool,
    pub position: Tick,
}
/// App state only. The immutable engine project is read through Session, never copied
/// into a parallel frontend project model. Every edit dispatches its registry ID.
pub struct Editor {
    pub session: Session,
    pub playback: Playback,
    pub preferences: Preferences,
    widgets: BTreeMap<&'static str, WidgetState>,
    pub panels: BTreeMap<&'static str, Rect>,
    texture: Option<egui::TextureHandle>,
    texture_key: Option<(u64, Tick)>,
    texture_owner: Option<std::sync::Arc<crate::playback::Frame>>,
    error: Option<String>,
    configured: bool,
    details: bool,
}
impl Editor {
    pub fn new(session: Session, playback: Playback, preferences: Preferences) -> Self {
        Self {
            session,
            playback,
            preferences,
            widgets: BTreeMap::new(),
            panels: BTreeMap::new(),
            texture: None,
            texture_key: None,
            texture_owner: None,
            error: None,
            configured: false,
            details: false,
        }
    }
    pub fn inspect(&self) -> Result<Inspection> {
        Ok(Inspection {
            widgets: self.widgets.values().cloned().collect(),
            view: "editor",
            playing: self.playback.playing(),
            position: self.playback.position()?,
        })
    }
    /// Last frame submitted by the frontend for presentation, independent of decode-ahead.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.texture_key.map(|(_, at)| at)
    }
    fn tr(&self, key: Text) -> &'static str {
        catalog::text(self.preferences.locale, key)
    }
    fn remember(&mut self, id: &'static str, response: &egui::Response) {
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(id));
        self.widgets.insert(
            id,
            WidgetState {
                id,
                enabled: response.enabled(),
                focused: response.has_focus(),
                rect: response.rect,
            },
        );
    }
    fn button(&mut self, ui: &mut egui::Ui, id: &'static str, key: Text, enabled: bool) -> bool {
        let label = self.tr(key);
        let response = ui
            .push_id(id, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::new(label).min_size(Vec2::new(28.0, 26.0)),
                )
            })
            .inner;
        self.remember(id, &response);
        response.clicked()
    }
    /// All control inputs (including tests/agents) use the same action path.
    /// Project edits are engine registry calls; transport/preferences are app state.
    pub fn activate(&mut self, id: &str) -> Result<()> {
        let at = self.playback.position()?;
        let duration = self.session.project().duration();
        let playing = self.playback.playing();
        match id {
            "transport.play" => self.playback.seek(
                if at >= duration { Tick::ZERO } else { at },
                !playing,
                self.session.project().clone(),
            )?,
            "transport.pause" => self
                .playback
                .seek(at, false, self.session.project().clone())?,
            "transport.back" => self.seek((at - Tick(time::TICKS_PER_SECOND)).max(Tick::ZERO))?,
            "transport.forward" => self.seek((at + Tick(time::TICKS_PER_SECOND)).min(duration))?,
            "transport.start" => self.seek(Tick::ZERO)?,
            "transport.end" => self.seek(duration)?,
            "edit.undo" | "edit.redo" => {
                self.session.execute(
                    if id == "edit.undo" { "undo" } else { "redo" },
                    serde_json::json!({}),
                )?;
                self.playback.seek(
                    at.min(self.session.project().duration()),
                    playing,
                    self.session.project().clone(),
                )?;
            }
            "appearance.theme" => self.preferences.dark = !self.preferences.dark,
            "appearance.locale" => {
                self.preferences.locale = if self.preferences.locale == Locale::En {
                    Locale::Vi
                } else {
                    Locale::En
                }
            }
            "layout.reset" => self.preferences.panels = Preferences::default().panels,
            "preview.retry" => self.playback.clear()?,
            "inspector.project" => self.details = false,
            "inspector.details" => self.details = true,
            _ => return Err(Error::Unsupported("widget action")),
        }
        Ok(())
    }
    fn seek(&mut self, at: Tick) -> Result<()> {
        self.playback
            .seek(at, self.playback.playing(), self.session.project().clone())
    }
    fn action(&mut self, id: &str) {
        if let Err(error) = self.activate(id) {
            self.error = Some(error.to_string());
        }
    }
    fn theme(&self, ctx: &egui::Context) {
        let mut visuals = if self.preferences.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let panel = if self.preferences.dark {
            Color32::from_rgb(15, 15, 15)
        } else {
            Color32::from_rgb(249, 250, 251)
        };
        visuals.panel_fill = panel;
        visuals.window_fill = panel;
        visuals.override_text_color = Some(if self.preferences.dark {
            Color32::from_rgb(229, 229, 229)
        } else {
            Color32::from_rgb(28, 28, 28)
        });
        visuals.selection.bg_fill = if self.preferences.dark {
            Color32::from_rgb(0, 193, 205)
        } else {
            Color32::from_rgb(0, 106, 116)
        };
        ctx.set_visuals(visuals);
    }
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        if let Err(e) = self.playback.poll() {
            self.error = Some(e.to_string());
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            let mut fonts = egui::FontDefinitions::empty();
            fonts.font_data.insert(
                "Inter".into(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../../../assets/fonts/Inter-Regular.ttf"
                ))),
            );
            fonts
                .families
                .insert(egui::FontFamily::Proportional, vec!["Inter".into()]);
            fonts
                .families
                .insert(egui::FontFamily::Monospace, vec!["Inter".into()]);
            ui.ctx().set_fonts(fonts);
            self.configured = true;
        }
        self.theme(ui.ctx());
        ui.style_mut().interaction.selectable_labels = false;
        self.widgets.clear();
        self.panels.clear();
        self.tick();
        self.keyboard(ui.ctx());
        let rect = ui.max_rect();
        let window_fill = if self.preferences.dark {
            Color32::from_rgb(15, 15, 15)
        } else {
            Color32::from_rgb(242, 243, 245)
        };
        ui.painter().rect_filled(rect, 0.0, window_fill);
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 52.0));
        let body = Rect::from_min_max(egui::pos2(rect.left(), top.bottom()), rect.max);
        let timeline_height = body.height() * self.preferences.panels[2];
        let upper = Rect::from_min_max(
            body.min,
            egui::pos2(body.right(), body.bottom() - timeline_height),
        );
        let timeline = Rect::from_min_max(egui::pos2(body.left(), upper.bottom()), body.max);
        let left_w = upper.width() * self.preferences.panels[0];
        let right_w = upper.width() * self.preferences.panels[1];
        let left = Rect::from_min_size(upper.min, Vec2::new(left_w, upper.height()));
        let right = Rect::from_min_max(egui::pos2(upper.right() - right_w, upper.top()), upper.max);
        let preview = Rect::from_min_max(
            egui::pos2(left.right(), upper.top()),
            egui::pos2(right.left(), upper.bottom()),
        );
        for (id, area) in [
            ("header", top),
            ("media", left),
            ("preview", preview),
            ("inspector", right),
            ("timeline", timeline),
        ] {
            self.panels.insert(id, area);
            let fill = if id == "header" {
                if self.preferences.dark {
                    Color32::from_rgb(19, 19, 19)
                } else {
                    Color32::WHITE
                }
            } else if self.preferences.dark {
                Color32::from_rgb(15, 15, 15)
            } else {
                Color32::from_rgb(249, 250, 251)
            };
            ui.painter().rect_filled(area, 0.0, fill);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(area.shrink(12.0)),
            );
            child.set_clip_rect(area);
            match id {
                "header" => self.header(&mut child),
                "media" => self.media(&mut child),
                "preview" => self.preview(&mut child),
                "inspector" => self.inspector(&mut child),
                _ => self.timeline(&mut child),
            }
        }
        let separator = if self.preferences.dark {
            Color32::from_rgb(31, 31, 31)
        } else {
            Color32::from_rgb(222, 222, 222)
        };
        for (id, area, axis) in [
            (
                "layout.media",
                Rect::from_center_size(
                    egui::pos2(left.right(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                0,
            ),
            (
                "layout.inspector",
                Rect::from_center_size(
                    egui::pos2(right.left(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                1,
            ),
            (
                "layout.timeline",
                Rect::from_center_size(
                    egui::pos2(timeline.center().x, timeline.top()),
                    Vec2::new(timeline.width(), 8.0),
                ),
                2,
            ),
        ] {
            let response = ui.interact(area, Id::unique(id), egui::Sense::drag());
            self.remember(id, &response);
            if response.dragged() {
                let delta = response.drag_delta();
                if let Some(v) = self.preferences.panels.get_mut(axis) {
                    *v = match axis {
                        0 => (*v + delta.x / upper.width()).clamp(0.15, 0.35),
                        1 => (*v - delta.x / upper.width()).clamp(0.15, 0.35),
                        _ => (*v - delta.y / body.height()).clamp(0.15, 0.7),
                    };
                }
            }
        }
        for (a, b) in [
            (top.left_bottom(), top.right_bottom()),
            (left.right_top(), left.right_bottom()),
            (right.left_top(), right.left_bottom()),
            (timeline.left_top(), timeline.right_top()),
        ] {
            ui.painter()
                .line_segment([a, b], egui::Stroke::new(1.0, separator));
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if self.playback.playing() { 8 } else { 33 },
            ));
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        for (key, id) in [
            (egui::Key::Space, "transport.play"),
            (egui::Key::K, "transport.pause"),
            (egui::Key::J, "transport.back"),
            (egui::Key::L, "transport.forward"),
            (egui::Key::Home, "transport.start"),
            (egui::Key::End, "transport.end"),
        ] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                self.action(id);
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("VibeCut").strong().size(18.0));
            ui.separator();
            ui.label(self.session.project().data().name.as_str());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.button(ui, "file.export", Text::Export, false);
                if self.button(ui, "appearance.theme", Text::Theme, true) {
                    self.action("appearance.theme");
                }
                if self.button(ui, "appearance.locale", Text::Language, true) {
                    self.action("appearance.locale");
                }
                if self.button(ui, "layout.reset", Text::Reset, true) {
                    self.action("layout.reset");
                }
            });
        });
    }
    fn media(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Media))
                .size(16.0)
                .strong(),
        );
        ui.add_space(16.0);
        self.button(ui, "file.import", Text::Import, false);
        ui.add_space(40.0);
        if self.session.project().data().assets.is_empty() {
            ui.label(self.tr(Text::EmptyMedia));
        } else {
            egui::ScrollArea::vertical()
                .id_salt("media.list")
                .show(ui, |ui| {
                    for asset in &self.session.project().data().assets {
                        ui.label(format!("{:?}", asset.id()));
                    }
                });
        }
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.button(ui, "inspector.project", Text::Project, true) {
                self.action("inspector.project");
            }
            if self.button(ui, "inspector.details", Text::Details, true) {
                self.action("inspector.details");
            }
        });
        ui.separator();
        ui.add_space(32.0);
        if self.details {
            let data = self.session.project().data();
            ui.label(format!("{} × {}", data.canvas.width, data.canvas.height));
            ui.label(format!(
                "{} / {} fps",
                data.canvas.fps.num(),
                data.canvas.fps.den()
            ));
            ui.label(format!("{} Hz", data.settings.audio_sample_rate));
        } else {
            ui.label(self.tr(Text::EmptyInspector));
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Preview))
                .size(16.0)
                .strong(),
        );
        ui.add_space(12.0);
        let area = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(
                ui.available_width(),
                (ui.available_height() - 86.0).max(40.0),
            ),
        );
        ui.painter().rect_filled(area, 0.0, Color32::BLACK);
        if let Some(frame) = self
            .playback
            .frame()
            .filter(|_| self.session.project().duration() > Tick::ZERO)
        {
            let key = (frame.epoch, frame.at);
            if self.texture_key != Some(key) {
                if let Some(texture) = &mut self.texture {
                    texture.set(frame.image.clone(), egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ui.ctx().load_texture(
                        "preview.frame",
                        frame.image.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.texture_key = Some(key);
                self.texture_owner = Some(frame.clone());
            }
            if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let size = size * (area.width() / size.x).min(area.height() / size.y);
                ui.painter().image(
                    texture.id(),
                    Rect::from_center_size(area.center(), size),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            self.texture = None;
            self.texture_key = None;
            self.texture_owner = None;
            let label = if self.playback.error().is_some() {
                Text::Unavailable
            } else if self.session.project().duration() == Tick::ZERO {
                Text::EmptyPreview
            } else {
                Text::Loading
            };
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::centered_and_justified(
                        egui::Direction::TopDown,
                    )),
                |ui| {
                    ui.label(
                        egui::RichText::new(self.tr(label))
                            .size(14.0)
                            .color(Color32::from_rgb(229, 229, 229)),
                    );
                },
            );
        }
        ui.advance_cursor_after_rect(area);
        let ready =
            self.session.project().duration() > Tick::ZERO && self.playback.device_available();
        ui.horizontal(|ui| {
            if self.button(ui, "transport.back", Text::Back, true) {
                self.action("transport.back");
            }
            if self.button(
                ui,
                "transport.play",
                if self.playback.playing() {
                    Text::Pause
                } else {
                    Text::Play
                },
                ready,
            ) {
                self.action("transport.play");
            }
            if self.button(ui, "transport.forward", Text::Forward, true) {
                self.action("transport.forward");
            }
        });
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        ui.label(format!(
            "{} / {}",
            display_time(at),
            display_time(self.session.project().duration())
        ));
        if !self.playback.device_available() {
            ui.label(self.tr(Text::Offline));
        }
        if self.error.is_some() || self.playback.error().is_some() {
            ui.label(self.tr(Text::Unavailable));
            if self.button(ui, "preview.retry", Text::Retry, true) {
                self.error = None;
                self.action("preview.retry");
            }
        }
    }
    fn timeline(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::Timeline))
                    .size(16.0)
                    .strong(),
            );
            for (id, key, command) in [
                ("edit.undo", Text::Undo, "undo"),
                ("edit.redo", Text::Redo, "redo"),
            ] {
                let enabled = engine::registry()
                    .iter()
                    .find(|s| s.id == command)
                    .is_some_and(|s| s.enabled(&self.session));
                if self.button(ui, id, key, enabled) {
                    self.action(id);
                }
            }
            ui.label(self.tr(Text::ReadOnly));
        });
        ui.add_space(12.0);
        let duration = self.session.project().duration();
        let mut seconds = self.playback.position().unwrap_or(Tick::ZERO).seconds();
        ui.spacing_mut().slider_width = (ui.available_width() - 120.0).max(100.0);
        let response = ui
            .push_id("transport.scrub", |ui| {
                ui.add_enabled(
                    duration > Tick::ZERO,
                    egui::Slider::new(&mut seconds, 0.0..=duration.seconds().max(0.001))
                        .show_value(false)
                        .trailing_fill(true)
                        .text(self.tr(Text::Playhead)),
                )
            })
            .inner;
        self.remember("transport.scrub", &response);
        if response.changed() {
            match Tick::from_seconds_f64(seconds, Rounding::Nearest)
                .map_err(Error::from)
                .and_then(|at| self.seek(at))
            {
                Ok(()) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if duration == Tick::ZERO {
            ui.add_space(48.0);
            ui.centered_and_justified(|ui| {
                ui.label(self.tr(Text::EmptyTimeline));
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("timeline.tracks")
                .show(ui, |ui| {
                    for track in &self.session.project().data().timeline.tracks {
                        ui.horizontal(|ui| {
                            ui.add_sized([112.0, 50.0], egui::Label::new(track.name.as_str()));
                            for item in &track.items {
                                ui.label(format!(
                                    "{} — {}",
                                    display_time(item.start),
                                    display_time(item.end().unwrap_or(item.start))
                                ));
                            }
                        });
                    }
                });
        }
    }
}
fn display_time(at: Tick) -> String {
    let s = at.seconds().max(0.0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (s / 3600.0) as u64,
        (s / 60.0) as u64 % 60,
        s as u64 % 60,
        ((s.fract() * 1000.0).floor()) as u64
    )
}
