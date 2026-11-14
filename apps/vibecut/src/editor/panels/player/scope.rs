//! Plots the colour scopes off the UI thread and paints the strip under the player video.

use super::{Action, Context, view::remember};
use crate::{
    egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2},
    i18n::Text,
    kit,
    services::playback::Frame,
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use time::Tick;

/// The frame generation and tick a set of plots belongs to.
type Key = (u64, Tick);

/// One plot in flight: the worker sends exactly one result and wakes the UI.
struct Work {
    key: Key,
    receive: mpsc::Receiver<render::Result<render::Scopes>>,
}

/// The parade, waveform and vectorscope textures.
struct Textures {
    parade: egui::TextureHandle,
    waveform: egui::TextureHandle,
    vectorscope: egui::TextureHandle,
}

/// The scope textures and the plot being made for the frame on screen.
#[derive(Default)]
pub(in crate::editor) struct State {
    textures: Option<Textures>,
    /// The key of the plots in `textures`.
    shown: Option<Key>,
    /// The key whose plot failed, so it is not retried every pass.
    failed: Option<Key>,
    work: Option<Work>,
    /// A result taken by [`State::settle`], applied by the next pass.
    settled: Option<(Key, render::Result<render::Scopes>)>,
}

fn image(plot: &render::Plot) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([plot.width, plot.height], &plot.rgba)
}

impl State {
    /// Drop the plots, the failure and any plot in flight: the scopes are off or there is no
    /// displayed frame.
    pub(in crate::editor) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Block until the plot in flight, if any, has finished, for the audit's capture. The next
    /// pass shows it.
    pub(in crate::editor) fn settle(&mut self, timeout: Duration) -> crate::Result<()> {
        if let Some(work) = self.work.take() {
            let result = work
                .receive
                .recv_timeout(timeout)
                .map_err(|_| crate::Error::Invalid("colour scope plot"))?;
            self.settled = Some((work.key, result));
        }
        Ok(())
    }

    /// The finished plot, if one has arrived since the last pass.
    fn receive(&mut self) -> Option<(Key, render::Result<render::Scopes>)> {
        if let Some(done) = self.settled.take() {
            return Some(done);
        }
        let work = self.work.as_ref()?;
        let key = work.key;
        let result = match work.receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(render::Error::WorkerPanicked),
        };
        self.work = None;
        Some((key, result))
    }

    /// Take a finished plot, then start the plot of `frame` when the one shown is for another
    /// frame and none is running. At most one worker runs, so a busy worker skips frames
    /// instead of queueing them.
    fn update(&mut self, ctx: &egui::Context, frame: &Arc<Frame>) {
        let key = (frame.generation, frame.at);
        if let Some((finished, result)) = self.receive() {
            match result {
                Ok(scopes) => {
                    match &mut self.textures {
                        Some(textures) => {
                            textures
                                .parade
                                .set(image(&scopes.parade), egui::TextureOptions::LINEAR);
                            textures
                                .waveform
                                .set(image(&scopes.waveform), egui::TextureOptions::LINEAR);
                            textures
                                .vectorscope
                                .set(image(&scopes.vectorscope), egui::TextureOptions::LINEAR);
                        }
                        None => {
                            self.textures = Some(Textures {
                                parade: ctx.load_texture(
                                    "preview.scopes.parade",
                                    image(&scopes.parade),
                                    egui::TextureOptions::LINEAR,
                                ),
                                waveform: ctx.load_texture(
                                    "preview.scopes.waveform",
                                    image(&scopes.waveform),
                                    egui::TextureOptions::LINEAR,
                                ),
                                vectorscope: ctx.load_texture(
                                    "preview.scopes.vectorscope",
                                    image(&scopes.vectorscope),
                                    egui::TextureOptions::LINEAR,
                                ),
                            });
                        }
                    }
                    self.shown = Some(finished);
                    self.failed = None;
                }
                Err(_) => self.failed = Some(finished),
            }
        }
        if self.work.is_none() && self.shown != Some(key) && self.failed != Some(key) {
            let (send, receive) = wake::channel();
            let frame = frame.clone();
            wake::spawn(move || {
                let [width, height] = frame.image.size;
                let rgba: Vec<u8> = frame
                    .image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                // The receiver is gone when the scopes were turned off meanwhile.
                let _ = send.send(render::plot_scopes(width, height, &rgba));
            });
            self.work = Some(Work { key, receive });
        }
    }
}

/// The strip under the video: `kit::scope::PANEL_HEIGHT` tall, inset like the stage, ending at
/// `bottom`.
pub(super) fn strip(panel: Rect, bottom: f32) -> Rect {
    Rect::from_min_max(
        egui::pos2(
            panel.left() + kit::space::MD,
            bottom - kit::scope::PANEL_HEIGHT,
        ),
        egui::pos2(panel.right() - kit::space::MD, bottom),
    )
}

/// The three panels of a `strip`, left to right: RGB parade, waveform, vectorscope. Each is a
/// third of the strip less the gaps (127 px wide in the reference's 391 px strip).
pub(super) fn panels(strip: Rect) -> [Rect; 3] {
    let width = ((strip.width() - 2.0 * kit::scope::GAP) / 3.0).max(0.0);
    std::array::from_fn(|n| {
        let left = strip.left() + n as f32 * (width + kit::scope::GAP);
        Rect::from_min_size(
            egui::pos2(left, strip.top()),
            egui::vec2(width, strip.height()),
        )
    })
}

/// Paint the scopes in `strip` for the displayed `frame`, or clear the plots when no frame is
/// displayed. Registers `preview.scopes`; `preview.scopes.ready` and `preview.scopes.failed`
/// join it while the plots of this very frame are on screen or failed.
pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    strip: Rect,
    frame: Option<&Arc<Frame>>,
    actions: &mut Vec<Action>,
) {
    match frame {
        Some(frame) => state.update(ui.ctx(), frame),
        None => state.clear(),
    }
    let key = frame.map(|frame| (frame.generation, frame.at));
    let failed = key.is_some() && state.failed == key;
    let ready = key.is_some() && state.shown == key;
    let [parade, waveform, vectorscope] = panels(strip);
    let painter = ui.painter().clone();
    for rect in [parade, waveform, vectorscope] {
        painter.rect_filled(rect, kit::scope::RADIUS, kit::scope::BACKGROUND);
    }
    let textures = state.textures.as_ref();
    trace(&painter, parade, textures.map(|t| &t.parade));
    trace(&painter, waveform, textures.map(|t| &t.waveform));
    vector(&painter, vectorscope, textures.map(|t| &t.vectorscope));
    if failed {
        painter.text(
            strip.center(),
            Align2::CENTER_CENTER,
            cx.tr(Text::ScopeFailed),
            FontId::proportional(kit::type_size::BODY),
            kit::scope::TARGET,
        );
    }
    let response = ui.interact(
        strip,
        ui.make_persistent_id("preview.scopes"),
        Sense::hover(),
    );
    let name = if failed {
        Text::ScopeFailed
    } else {
        Text::ScopeMenu
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, cx.tr(name)));
    remember(cx, actions, "preview.scopes", response);
    for (id, on) in [
        ("preview.scopes.ready", ready),
        ("preview.scopes.failed", failed),
    ] {
        if on {
            let marker = ui.interact(strip, ui.make_persistent_id(id), Sense::hover());
            remember(cx, actions, id, marker);
        }
    }
    for (rect, id, label) in [
        (parade, "preview.scopes.parade", Text::ScopeParade),
        (waveform, "preview.scopes.waveform", Text::ScopeWaveform),
        (
            vectorscope,
            "preview.scopes.vectorscope",
            Text::ScopeVectorscope,
        ),
    ] {
        let panel = ui.interact(rect, ui.make_persistent_id(id), Sense::hover());
        panel.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, cx.tr(label)));
        remember(cx, actions, id, panel);
    }
}

/// The parade or waveform panel: three dashed level lines at 1023, 512 and 0, the plot between
/// the outer two, and the level labels over the trace.
fn trace(painter: &egui::Painter, rect: Rect, texture: Option<&egui::TextureHandle>) {
    let left = rect.left() + kit::scope::PLOT_INSET;
    let right = rect.right() - kit::scope::PLOT_INSET;
    let top = rect.top() + kit::scope::LINE_INSET;
    let bottom = rect.bottom() - kit::scope::LINE_INSET;
    let stroke = Stroke::new(kit::scope::GRATICULE, kit::scope::GRATICULE_LINE);
    for y in [top, (top + bottom) / 2.0, bottom] {
        painter.extend(egui::Shape::dashed_line(
            &[egui::pos2(left, y), egui::pos2(right, y)],
            stroke,
            kit::scope::DASH_ON,
            kit::scope::DASH_OFF,
        ));
    }
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let font = FontId::new(kit::scope::LABEL_SIZE, kit::label_font().family);
    for (y, label) in [(top, "1023"), ((top + bottom) / 2.0, "512"), (bottom, "0")] {
        painter.text(
            egui::pos2(rect.left() + kit::scope::LABEL_INSET, y),
            Align2::LEFT_CENTER,
            label,
            font.clone(),
            kit::scope::LABEL,
        );
    }
}

/// The six colour targets of the vectorscope with the point each one's pure colour plots at: the
/// angle of its chroma, counter-clockwise from the blue-difference axis.
fn targets() -> [(&'static str, f32); 6] {
    [
        ("R", [255.0, 0.0, 0.0]),
        ("M", [255.0, 0.0, 255.0]),
        ("B", [0.0, 0.0, 255.0]),
        ("C", [0.0, 255.0, 255.0]),
        ("G", [0.0, 255.0, 0.0]),
        ("Y", [255.0, 255.0, 0.0]),
    ]
    .map(|(name, [r, g, b])| {
        let cb: f32 = (-38.0 * r - 74.0 * g + 112.0 * b) / 256.0;
        let cr: f32 = (112.0 * r - 94.0 * g - 18.0 * b) / 256.0;
        (name, cr.atan2(cb))
    })
}

/// The vectorscope panel: ring, dashed axes, ticks, skin-tone line, targets and the plot.
fn vector(painter: &egui::Painter, rect: Rect, texture: Option<&egui::TextureHandle>) {
    let centre = rect.center();
    let radius = kit::scope::VECTOR_DIAMETER / 2.0;
    let at = |angle: f32, distance: f32| {
        egui::pos2(
            centre.x + distance * angle.cos(),
            centre.y - distance * angle.sin(),
        )
    };
    let ring = Stroke::new(kit::scope::RING, kit::scope::RING_LINE);
    painter.circle_stroke(centre, radius, ring);
    for (from, to) in [
        (at(0.0, -radius), at(0.0, radius)),
        (
            at(std::f32::consts::FRAC_PI_2, -radius),
            at(std::f32::consts::FRAC_PI_2, radius),
        ),
    ] {
        painter.extend(egui::Shape::dashed_line(
            &[from, to],
            ring,
            kit::scope::DASH_ON,
            kit::scope::DASH_OFF,
        ));
    }
    let tick = Stroke::new(kit::scope::RING, kit::scope::TICK_LINE);
    for degrees in (0..360).step_by(kit::scope::TICK_STEP_DEGREES as usize) {
        let angle = (degrees as f32).to_radians();
        let length = if degrees % 90 == 0 {
            kit::scope::TICK_MAJOR
        } else {
            kit::scope::TICK
        };
        painter.line_segment([at(angle, radius), at(angle, radius - length)], tick);
    }
    painter.line_segment(
        [centre, at(kit::scope::SKIN_DEGREES.to_radians(), radius)],
        Stroke::new(kit::scope::RING, kit::scope::SKIN_LINE),
    );
    let font = FontId::new(kit::scope::TARGET_SIZE, kit::label_font().family);
    let half = kit::scope::BRACKET / 2.0;
    let arm = kit::scope::BRACKET_ARM;
    let bracket = Stroke::new(kit::scope::RING, kit::scope::TARGET);
    for (name, angle) in targets() {
        let spot = at(angle, kit::scope::BRACKET_RADIUS);
        for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = egui::pos2(spot.x + sx * half, spot.y + sy * half);
            painter.line_segment([corner, egui::pos2(corner.x - sx * arm, corner.y)], bracket);
            painter.line_segment([corner, egui::pos2(corner.x, corner.y - sy * arm)], bracket);
        }
        painter.text(
            spot,
            Align2::CENTER_CENTER,
            name,
            font.clone(),
            kit::scope::TARGET,
        );
    }
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            Rect::from_center_size(centre, Vec2::splat(kit::scope::VECTOR_DIAMETER)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

/// The `Color oscilloscope` submenu of the player menu: Show and Hide, the current one
/// highlighted. A row for the state already in force does nothing; the other runs the registered
/// `preview.scopes` action, so the menu and agents share one path.
pub(super) fn submenu(ui: &mut egui::Ui, cx: &Context<'_>, actions: &mut Vec<Action>) {
    let shown = cx.preferences.preview_scopes;
    let response = ui.menu_button(cx.tr(Text::ScopeMenu), |ui| {
        for (id, text, on) in [
            ("preview.menu.scopes.show", Text::ScopeShow, true),
            ("preview.menu.scopes.hide", Text::ScopeHide, false),
        ] {
            let row = ui.add(egui::Button::new(cx.tr(text)).selected(shown == on));
            if row.clicked() {
                if shown != on {
                    actions.push(Action::Command("preview.scopes".into()));
                }
                ui.close();
            }
            remember(cx, actions, id, row);
        }
    });
    remember(cx, actions, "preview.menu.scope", response.response);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn three_panels_split_the_strip_with_the_measured_gap() {
        // The reference's panel is 407 wide: the strip is inset 8 each side and the panels are
        // 127 wide with a 5 gap.
        let panel = Rect::from_min_size(egui::pos2(1274.0, 100.0), egui::vec2(407.0, 600.0));
        let strip = strip(panel, 560.0);
        assert_eq!(strip.width(), 391.0);
        assert_eq!(strip.height(), kit::scope::PANEL_HEIGHT);
        assert_eq!(strip.bottom(), 560.0);
        let [parade, waveform, vectorscope] = panels(strip);
        for rect in [parade, waveform, vectorscope] {
            assert_eq!(rect.width(), 127.0);
            assert_eq!(rect.height(), 159.0);
            assert_eq!(rect.top(), strip.top());
        }
        assert_eq!(waveform.left() - parade.right(), kit::scope::GAP);
        assert_eq!(vectorscope.left() - waveform.right(), kit::scope::GAP);
        assert_eq!(vectorscope.right(), strip.right());
    }

    #[test]
    fn the_targets_sit_where_the_reference_puts_them() {
        // R up-left, M up-right, B right, C bottom, G bottom-left, Y left (04 and 08).
        let angles: std::collections::BTreeMap<_, _> = targets().into_iter().collect();
        let degrees = |name: &str| angles[name].to_degrees().rem_euclid(360.0);
        assert!((90.0..135.0).contains(&degrees("R")));
        assert!((45.0..90.0).contains(&degrees("M")));
        assert!(degrees("B") > 330.0 || degrees("B") < 15.0);
        assert!((255.0..300.0).contains(&degrees("C")));
        assert!((225.0..270.0).contains(&degrees("G")));
        assert!((150.0..195.0).contains(&degrees("Y")));
    }
}
