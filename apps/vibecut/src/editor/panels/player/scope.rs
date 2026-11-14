//! Plots the shown colour scope off the UI thread and paints it over the player.

use super::{Action, Context, view::remember};
use crate::{
    egui::{self, Align2, FontId, Rect, Sense, Vec2},
    i18n::Text,
    kit,
    services::playback::Frame,
};
use render::{SCOPE_SIDE, ScopeKind};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use time::Tick;

/// The frame, tick and kind a plot belongs to.
type Key = (u64, Tick, ScopeKind);

/// One plot in flight: the worker sends exactly one result and wakes the UI.
struct Work {
    key: Key,
    receive: mpsc::Receiver<render::Result<render::Plot>>,
}

/// The scope texture and the plot being made for the frame on screen.
#[derive(Default)]
pub(in crate::editor) struct State {
    texture: Option<egui::TextureHandle>,
    /// The key of the plot in `texture`.
    shown: Option<Key>,
    /// The key whose plot failed, so it is not retried every pass.
    failed: Option<Key>,
    work: Option<Work>,
    /// A result taken by [`State::settle`], applied by the next pass.
    settled: Option<(Key, render::Result<render::Plot>)>,
}

impl State {
    /// Drop the plot, the failure and any plot in flight: the scope is off or there is no
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
    fn receive(&mut self) -> Option<(Key, render::Result<render::Plot>)> {
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
    fn update(&mut self, ctx: &egui::Context, frame: &Arc<Frame>, kind: ScopeKind) {
        let key = (frame.generation, frame.at, kind);
        if let Some((finished, result)) = self.receive() {
            match result {
                Ok(plot) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [SCOPE_SIDE, SCOPE_SIDE],
                        &plot.rgba,
                    );
                    match &mut self.texture {
                        Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
                        None => {
                            self.texture = Some(ctx.load_texture(
                                "preview.scope",
                                image,
                                egui::TextureOptions::NEAREST,
                            ));
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
                // The receiver is gone when the scope was turned off meanwhile.
                let _ = send.send(render::plot_scope(kind, width, height, &rgba));
            });
            self.work = Some(Work { key, receive });
        }
    }
}

/// The panel rectangle inside the viewer `stage`: a [`kit::size::SCOPE_PANEL`] square at the
/// bottom left, clear of the left ruler, shrunk to fit a small stage; `None` when no room is left.
pub(super) fn panel(stage: Rect) -> Option<Rect> {
    let inset = kit::space::MD;
    let left = stage.left() + kit::size::PLAYER_RULER + inset;
    let side = kit::size::SCOPE_PANEL
        .min(stage.right() - inset - left)
        .min(stage.height() - kit::size::PLAYER_RULER - 2.0 * inset);
    if side <= 0.0 {
        return None;
    }
    let bottom = stage.bottom() - inset;
    Some(Rect::from_min_max(
        egui::pos2(left, bottom - side),
        egui::pos2(left + side, bottom),
    ))
}

/// Paint the scope chosen in the preferences over `stage` for the displayed `frame`, or clear
/// it when the scope is off or no frame is displayed.
///
/// The panel registers `preview.scope`; `preview.scope.ready` and `preview.scope.failed` join
/// it while the plot of this very frame is on screen or failed.
pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    stage: Rect,
    frame: Option<&Arc<Frame>>,
    actions: &mut Vec<Action>,
) {
    let (Some(kind), Some(frame)) = (cx.preferences.preview_scope, frame) else {
        state.clear();
        return;
    };
    state.update(ui.ctx(), frame, kind);
    let Some(rect) = panel(stage) else {
        return;
    };
    let dark = cx.dark;
    let key = (frame.generation, frame.at, kind);
    let failed = state.failed == Some(key);
    let ready = state.shown == Some(key);
    let label = if failed {
        Text::ScopeFailed
    } else {
        kind_text(kind)
    };
    let response = ui.interact(rect, ui.make_persistent_id("preview.scope"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, cx.tr(label)));
    let painter = ui.painter().with_clip_rect(stage);
    painter.rect_filled(rect, kit::radius::FRAME_PANEL, kit::strip(dark));
    painter.rect_stroke(
        rect,
        kit::radius::FRAME_PANEL,
        egui::Stroke::new(kit::space::HAIRLINE, kit::border(dark)),
        egui::StrokeKind::Inside,
    );
    let texture = state
        .texture
        .as_ref()
        .filter(|_| state.shown.is_some_and(|(_, _, shown)| shown == kind));
    if failed {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            cx.tr(Text::ScopeFailed),
            FontId::proportional(kit::type_size::BODY),
            kit::secondary(dark),
        );
    } else if let Some(texture) = texture {
        // The previous frame's plot of this kind stays up while the next one is made.
        let tint = if kind.tinted() {
            kit::text(dark)
        } else {
            kit::image_tint()
        };
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    } else {
        kit::tinted(
            kit::Icon::ArrowClockwise,
            kit::icon::DENSE,
            kit::secondary(dark),
        )
        .paint_at(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(kit::icon::DENSE)),
        );
    }
    remember(cx, actions, "preview.scope", response);
    for (id, on) in [("preview.scope.ready", ready), ("preview.scope.failed", failed)] {
        if on {
            let marker = ui.interact(rect, ui.make_persistent_id(id), Sense::hover());
            remember(cx, actions, id, marker);
        }
    }
}

fn kind_text(kind: ScopeKind) -> Text {
    match kind {
        ScopeKind::Waveform => Text::ScopeWaveform,
        ScopeKind::Vectorscope => Text::ScopeVectorscope,
        ScopeKind::Histogram => Text::ScopeHistogram,
    }
}

/// The `Color oscilloscope` submenu of the player menu. Each row is the registered action of
/// the same kind, ticked while its scope is the one shown, so the menu and agents share one path.
pub(super) fn submenu(ui: &mut egui::Ui, cx: &Context<'_>, actions: &mut Vec<Action>) {
    let active = cx.preferences.preview_scope;
    let response = ui.menu_button(cx.tr(Text::ScopeMenu), |ui| {
        for kind in ScopeKind::ALL {
            let (id, command) = match kind {
                ScopeKind::Waveform => ("preview.menu.scope.waveform", "preview.scope.waveform"),
                ScopeKind::Vectorscope => (
                    "preview.menu.scope.vectorscope",
                    "preview.scope.vectorscope",
                ),
                ScopeKind::Histogram => {
                    ("preview.menu.scope.histogram", "preview.scope.histogram")
                }
            };
            let row = ui.add(
                egui::Button::new(cx.tr(kind_text(kind))).selected(active == Some(kind)),
            );
            if row.clicked() {
                actions.push(Action::Command(command.into()));
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
    fn the_panel_sits_bottom_left_clear_of_the_ruler_and_shrinks_with_the_stage() {
        let stage = Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(600.0, 500.0));
        let rect = panel(stage).unwrap();
        assert_eq!(rect.width(), kit::size::SCOPE_PANEL);
        assert_eq!(rect.height(), kit::size::SCOPE_PANEL);
        assert_eq!(
            rect.left(),
            stage.left() + kit::size::PLAYER_RULER + kit::space::MD
        );
        assert_eq!(rect.bottom(), stage.bottom() - kit::space::MD);
        let small = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(120.0, 100.0));
        let shrunk = panel(small).unwrap();
        assert!(shrunk.width() < kit::size::SCOPE_PANEL);
        assert!(small.contains_rect(shrunk));
        assert_eq!(shrunk.width(), shrunk.height());
        let tiny = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(20.0, 20.0));
        assert!(panel(tiny).is_none());
    }
}
