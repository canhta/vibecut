//! Reusable colour draft and native screen-sampler adapter. No project writes.
use crate::i18n;
pub mod palette;

use crate::{
    Error, Result,
    egui::{self, Color32, Id, Vec2},
    i18n::{Locale, Text},
};
use project::{BookmarkId, Color, ItemId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorTarget {
    Text(ItemId),
    Background(ItemId),
    GraphicFill(ItemId),
    GraphicStroke(ItemId),
    MaskStroke {
        item: ItemId,
        mask: String,
    },
    /// The chroma key's packed key colour on one effect instance.
    EffectKeyColor {
        item: ItemId,
        effect: String,
    },
    Bookmark(BookmarkId),
    Canvas,
}
impl ColorTarget {
    pub(crate) fn item(&self) -> Option<ItemId> {
        match self {
            Self::Text(id)
            | Self::Background(id)
            | Self::GraphicFill(id)
            | Self::GraphicStroke(id) => Some(*id),
            Self::MaskStroke { item, .. } | Self::EffectKeyColor { item, .. } => Some(*item),
            Self::Bookmark(_) | Self::Canvas => None,
        }
    }
    pub(crate) fn color(&self, project: &project::Project) -> Result<Color> {
        if *self == Self::Canvas {
            return Ok(project.data().canvas.background);
        }
        if let Self::Bookmark(bookmark) = self {
            return project
                .data()
                .scene()
                .bookmarks
                .iter()
                .find(|candidate| candidate.id == *bookmark)
                .map(|candidate| candidate.color)
                .ok_or(Error::Invalid("bookmark colour target"));
        }
        let item = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| Some(i.id) == self.item())
            .ok_or(Error::Invalid("colour target"))?;
        match (self, &item.content) {
            (Self::Text(_), project::ItemParams::Text(p)) => Ok(p.color),
            (Self::Background(_), project::ItemParams::Text(p)) => {
                Ok(p.background.as_ref().map_or(Color([0; 4]), |b| b.color))
            }
            (Self::GraphicFill(_), project::ItemParams::Overlay(p)) => Ok(p.color),
            (Self::GraphicStroke(_), project::ItemParams::Overlay(_)) => Ok(render::color::at(
                &item.animations,
                "params.strokeColor",
                time::Tick::ZERO,
                Color([0; 4]),
            )?),
            (Self::MaskStroke { mask, .. }, _) => item
                .masks
                .iter()
                .find(|m| m.id == *mask)
                .map(|m| m.stroke.color)
                .ok_or(Error::Invalid("mask colour target")),
            (Self::EffectKeyColor { effect, .. }, _) => item
                .effects
                .iter()
                .find(|candidate| candidate.id == *effect)
                .and_then(|candidate| match candidate.params {
                    project::EffectParams::ChromaKey { key_color, .. } => Some(key_color),
                    _ => None,
                })
                .ok_or(Error::Invalid("chroma key colour target")),
            _ => Err(Error::Invalid("colour target")),
        }
    }
    pub(crate) fn command(
        &self,
        project: &project::Project,
        color: Color,
    ) -> Result<(&'static str, serde_json::Value)> {
        if *self == Self::Canvas {
            return Ok((
                "project.set_background",
                serde_json::json!({"color": color}),
            ));
        }
        if let Self::Bookmark(bookmark) = self {
            return Ok((
                "bookmark.update",
                serde_json::json!({"bookmark": bookmark, "color": color}),
            ));
        }
        if let Self::MaskStroke { item, mask } = self {
            let mut value = project
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| i.id == *item)
                .and_then(|i| i.masks.iter().find(|m| m.id == *mask))
                .cloned()
                .ok_or(Error::Invalid("mask colour target"))?;
            value.stroke.color = color;
            return Ok((
                "masks.update",
                serde_json::json!({"item":item,"mask":value}),
            ));
        }
        if let Self::EffectKeyColor { item, effect } = self {
            let [r, g, b, _] = color.0;
            let packed = (u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)) as f64;
            return Ok((
                "effects.update_param",
                serde_json::json!({"item":item,"effect":effect,"key":"key_color","value":packed}),
            ));
        }
        let field = match self {
            Self::Text(_) => "text_color",
            Self::Background(_) => "text_background_color",
            Self::GraphicFill(_) => "graphic_fill",
            Self::GraphicStroke(_) => "graphic_stroke",
            _ => return Err(Error::Invalid("colour target")),
        };
        Ok((
            "timeline.set_item_param",
            serde_json::json!({"item":self.item(),"patch":{"field":field,"value":color}}),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EyedropperState {
    Ready,
    Pending,
    Unavailable,
    Cancelled,
    Failed,
}

/// One screen-colour operation, fenced by the lifetime of its owning draft.
pub struct NativeEyedropper {
    child: Option<std::process::Child>,
    started: Option<std::time::Instant>,
    pub state: EyedropperState,
}
impl Default for NativeEyedropper {
    fn default() -> Self {
        Self {
            child: None,
            started: None,
            state: if cfg!(target_os = "macos") {
                EyedropperState::Ready
            } else {
                EyedropperState::Unavailable
            },
        }
    }
}
impl NativeEyedropper {
    /// Explicitly unavailable adapter, also used by deterministic UI hosts.
    pub fn unavailable() -> Self {
        Self {
            child: None,
            started: None,
            state: EyedropperState::Unavailable,
        }
    }
    pub fn start(&mut self) -> Result<()> {
        if self.state == EyedropperState::Unavailable {
            return Err(Error::Unsupported("native screen colour picker"));
        }
        if self.child.is_some() {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            // JXA is the system scripting bridge to AppKit; no browser or screen capture.
            let script = include_str!("color_sampler.js");
            self.child = Some(
                std::process::Command::new("/usr/bin/osascript")
                    .args(["-l", "JavaScript", "-e", script])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()?,
            );
            self.started = Some(std::time::Instant::now());
            self.state = EyedropperState::Pending;
        }
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<Color>> {
        use std::io::Read;
        let Some(child) = &mut self.child else {
            return Ok(None);
        };
        if self
            .started
            .is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(120))
        {
            self.stop();
            self.state = EyedropperState::Cancelled;
            return Ok(None);
        }
        let Some(status) = child.try_wait()? else {
            return Ok(None);
        };
        let mut bytes = String::new();
        if let Some(stdout) = child.stdout.take() {
            stdout.take(128).read_to_string(&mut bytes)?;
        }
        self.child = None;
        if !status.success() {
            self.state = EyedropperState::Failed;
            return Err(Error::Unsupported("native screen colour picker failed"));
        }
        if bytes.trim().is_empty() {
            self.state = EyedropperState::Cancelled;
            return Ok(None);
        }
        self.state = EyedropperState::Ready;
        Ok(Some(render::color::parse_hex(bytes.trim())?))
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for NativeEyedropper {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct Picker {
    pub target: ColorTarget,
    pub original: Color,
    pub color: Color,
    hsva: [f64; 4],
    hex: String,
    entries: [String; 4],
    pub native: NativeEyedropper,
    /// Top-right corner the popover hangs from, beside the control that opened it.
    pub anchor: Option<egui::Pos2>,
    invalid: bool,
}
pub(crate) struct PickerEvent {
    pub preview: bool,
    pub commit: bool,
    pub cancel: bool,
    pub widgets: Vec<(String, egui::Response)>,
}
impl Picker {
    pub fn new(target: ColorTarget, color: Color) -> Self {
        let mut value = Self {
            target,
            original: color,
            color,
            hsva: [0.0; 4],
            hex: String::new(),
            entries: Default::default(),
            native: NativeEyedropper::default(),
            anchor: None,
            invalid: false,
        };
        value.set(color);
        value
    }
    fn set(&mut self, color: Color) {
        self.color = color;
        self.hex = render::color::hex(color);
        self.hsva = render::color::to_hsva(color);
        self.refresh_entries();
        self.invalid = false;
    }
    fn refresh_entries(&mut self) {
        // Hue in degrees, the other channels as whole percents.
        let [h, s, v, a] = self.hsva;
        self.entries = [h, s * 100.0, v * 100.0, a * 100.0].map(|v| format!("{v:.0}"));
    }
    fn set_hsva(&mut self, values: [f64; 4]) -> Result<()> {
        let color = render::color::from_hsva(values)?;
        let [h, s, v, a] = values;
        self.hsva = [
            h.clamp(0.0, 360.0),
            s.clamp(0.0, 1.0),
            v.clamp(0.0, 1.0),
            a.clamp(0.0, 1.0),
        ];
        self.color = color;
        self.hex = render::color::hex(color);
        self.refresh_entries();
        self.invalid = false;
        Ok(())
    }
    pub fn ui(&mut self, ctx: &egui::Context, locale: Locale, recent: &[Color]) -> PickerEvent {
        let mut event = PickerEvent {
            preview: false,
            commit: false,
            cancel: false,
            widgets: Vec::new(),
        };
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            event.cancel = true;
            return event;
        }
        match self.native.poll() {
            Ok(Some(color)) => {
                let [r, g, b, _] = color.0;
                let [_, _, _, a] = self.color.0;
                self.set(Color([r, g, b, a]));
                event.preview = true;
                event.commit = true;
            }
            Err(_) => self.native.state = EyedropperState::Failed,
            _ => {}
        }
        let window = egui::Window::new(i18n::text(locale, Text::ColorPicker))
            .id(Id::unique("color.picker"))
            .resizable(false)
            .collapsible(false)
            .default_width(280.0);
        let window = match self.anchor {
            Some(anchor) => window.pivot(egui::Align2::RIGHT_TOP).fixed_pos(anchor),
            None => window.default_pos(egui::pos2(
                (ctx.content_rect().right() - 610.0).max(16.0),
                112.0,
            )),
        };
        window.show(ctx, |ui| {
            let [r, g, b, a] = self.color.0;
            let swatch = Color32::from_rgba_unmultiplied(r, g, b, a);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 4.0, swatch);
            let caption = ui.label(i18n::text(locale, Text::ColorHex));
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut self.hex)
                        .id(Id::unique("color.hex"))
                        .char_limit(9),
                )
                .labelled_by(caption.id);
            if response.changed() {
                self.invalid = render::color::parse_hex(&self.hex).is_err();
            }
            if (response.lost_focus()
                || response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                && self.hex != render::color::hex(self.color)
                && let Ok(color) = render::color::parse_hex(&self.hex)
            {
                self.set(color);
                event.preview = true;
                event.commit = true;
            }
            event.widgets.push(("color.hex".into(), response));
            for (index, id, label, max) in [
                (0, "h", Text::Hue, 360.0),
                (1, "s", Text::Saturation, 1.0),
                (2, "v", Text::Value, 1.0),
                (3, "a", Text::Alpha, 1.0),
            ] {
                let caption = ui.label(i18n::text(locale, label));
                let mut value = self.hsva.get(index).copied().unwrap_or(0.0);
                let slider = ui
                    .add(egui::Slider::new(&mut value, 0.0..=max).show_value(false))
                    .labelled_by(caption.id);
                if slider.changed() {
                    if let Some(component) = self.hsva.get_mut(index) {
                        *component = value;
                    }
                    if let Ok(color) = render::color::from_hsva(self.hsva) {
                        self.color = color;
                        self.hex = render::color::hex(color);
                        self.refresh_entries();
                        event.preview = true;
                    }
                }
                if slider.drag_stopped()
                    || slider.changed() && !ui.input(|i| i.pointer.primary_down())
                {
                    event.commit = true;
                }
                event.widgets.push((format!("color.{id}.slider"), slider));
                let Some(entry) = self.entries.get_mut(index) else {
                    continue;
                };
                let response = ui
                    .add(
                        egui::TextEdit::singleline(entry)
                            .id(Id::unique(format!("color.{id}")))
                            .char_limit(32),
                    )
                    .labelled_by(caption.id);
                if response.lost_focus()
                    || response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    if let Ok(value) = entry.trim().parse::<f64>() {
                        let value = if index == 0 { value } else { value / 100.0 };
                        let mut values = self.hsva;
                        if let Some(component) = values.get_mut(index) {
                            *component = value;
                        }
                        if self.set_hsva(values).is_ok() {
                            event.preview = true;
                            event.commit = true;
                        } else {
                            self.invalid = true;
                        }
                    } else {
                        self.invalid = true;
                    }
                }
                event.widgets.push((format!("color.{id}"), response));
            }
            ui.label(i18n::text(locale, Text::Swatches));
            egui::ScrollArea::vertical()
                .id_salt("color.palette")
                .max_height(180.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (index, _) in palette::SOLID.iter().enumerate() {
                            let Some(color) = palette::solid(index) else {
                                continue;
                            };
                            let [r, g, b, a] = color.0;
                            let response = ui
                                .add(
                                    egui::Button::new(String::new())
                                        .fill(Color32::from_rgba_unmultiplied(r, g, b, a))
                                        .min_size(Vec2::splat(24.0)),
                                )
                                .on_hover_text(render::color::hex(color));
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    true,
                                    render::color::hex(color),
                                )
                            });
                            if response.clicked() {
                                self.set(color);
                                event.preview = true;
                                event.commit = true;
                            }
                            event
                                .widgets
                                .push((format!("color.swatch.{index}"), response));
                        }
                    });
                });
            ui.label(i18n::text(locale, Text::RecentColors));
            ui.horizontal_wrapped(|ui| {
                for (index, color) in recent.iter().enumerate() {
                    let [r, g, b, a] = color.0;
                    let response = ui
                        .add(
                            egui::Button::new(String::new())
                                .fill(Color32::from_rgba_unmultiplied(r, g, b, a))
                                .min_size(Vec2::splat(24.0)),
                        )
                        .on_hover_text(render::color::hex(*color));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            render::color::hex(*color),
                        )
                    });
                    if response.clicked() {
                        self.set(*color);
                        event.preview = true;
                        event.commit = true;
                    }
                    event
                        .widgets
                        .push((format!("color.recent.{index}"), response));
                }
            });
            let response = ui.add(egui::Button::image_and_text(
                crate::kit::tinted(crate::kit::Icon::Eye, 16.0, ui.visuals().text_color()),
                i18n::text(locale, Text::Eyedropper),
            ));
            if response.clicked() && self.native.start().is_err() {
                self.native.state = EyedropperState::Unavailable;
            }
            event.widgets.push(("color.eyedropper".into(), response));
            let status = match self.native.state {
                EyedropperState::Unavailable => Some(Text::EyedropperUnavailable),
                EyedropperState::Failed => Some(Text::EyedropperFailed),
                EyedropperState::Pending => Some(Text::EyedropperPending),
                _ => None,
            };
            if let Some(status) = status {
                let response = ui.label(i18n::text(locale, status));
                event
                    .widgets
                    .push(("color.eyedropper.status".into(), response));
            }
            if self.invalid {
                ui.label(i18n::text(locale, Text::InvalidColor));
            }
            let cancel = ui.button(i18n::text(locale, Text::Cancel));
            event.cancel |= cancel.clicked();
            event.widgets.push(("color.cancel".into(), cancel));
        });
        event
    }
}
