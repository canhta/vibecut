//! Drag handles for the selected item's effect points on the preview canvas.
//!
//! While the Properties Effects tab is open for the single selected item, each enabled
//! effect's point parameter (Corner Pin's four corners, a warp centre, a gradient end)
//! gets a circular handle at its frame fraction of the item's placement box, mapped through
//! the item transform so it stays on the rendered picture. A press within the shared hit
//! radius takes the pointer before the transform handles, which is why the handles only
//! show on that tab: Corner Pin's defaults sit exactly on the transform corners. A drag
//! stages `effects.update_param` inside `timeline.preview` as an absolute point clamped to
//! the frame; one drop is one undo step, and a drop where it started publishes nothing.

use super::{Action, Context, transform::paint_handle};
use crate::{
    egui::{self, CursorIcon, Pos2, Sense, Vec2},
    kit,
};
use project::{EffectParamKind, EffectValue, ItemId, POINT_MAX};
use render::viewport::{ItemGeometry, Viewport};

#[derive(Default)]
pub(in crate::editor) struct State {
    drag: Option<Drag>,
}

impl State {
    /// Abandon a live gesture; the caller discards its staged candidate.
    pub(in crate::editor) fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }
}

struct Drag {
    item: ItemId,
    effect: String,
    key: String,
    start: [f64; 2],
    last: Option<[f64; 2]>,
}

struct Handle {
    effect: String,
    key: &'static str,
    label: &'static str,
    point: [f64; 2],
    screen: Pos2,
}

fn handles(
    cx: &Context<'_>,
    item: ItemId,
    geometry: &ItemGeometry,
    viewport: &Viewport,
) -> Vec<Handle> {
    if !cx.effects_tab {
        return Vec::new();
    }
    // A locked track's points are read-only, as the inspector fields are disabled.
    let Some(source) = cx
        .staged_project()
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| !track.locked)
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
    else {
        return Vec::new();
    };
    let bounds = geometry.bounds();
    let mut found = Vec::new();
    for effect in source.effects.iter().filter(|effect| effect.enabled) {
        let Ok(definition) = effect.definition() else {
            continue;
        };
        for parameter in definition
            .params
            .iter()
            .filter(|parameter| parameter.kind == EffectParamKind::Point)
        {
            let Some(EffectValue::Point(point)) = effect.value(parameter.id) else {
                continue;
            };
            let Ok([x, y]) = geometry.point(
                viewport,
                [
                    bounds.x + point[0] * bounds.width,
                    bounds.y + point[1] * bounds.height,
                ],
            ) else {
                continue;
            };
            found.push(Handle {
                effect: effect.id.clone(),
                key: parameter.id,
                label: crate::i18n::effect_label(definition.id, Some(parameter.id))
                    .map_or(parameter.name, |key| cx.tr(key)),
                point: *point,
                screen: egui::pos2(x as f32, y as f32),
            });
        }
    }
    found
}

/// The frame fraction under a screen point, clamped to the frame the model admits.
fn fraction(geometry: &ItemGeometry, viewport: &Viewport, screen: Pos2) -> Option<[f64; 2]> {
    let bounds = geometry.bounds();
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return None;
    }
    let canvas = geometry
        .inverse(viewport, [f64::from(screen.x), f64::from(screen.y)])
        .ok()?;
    Some([
        ((canvas[0] - bounds.x) / bounds.width).clamp(0.0, POINT_MAX),
        ((canvas[1] - bounds.y) / bounds.height).clamp(0.0, POINT_MAX),
    ])
}

/// Paint the handles and drive the gesture. Returns whether a handle owns the pointer.
pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    viewport: &Viewport,
    frame: egui::Rect,
    target: Option<(ItemId, &ItemGeometry)>,
    actions: &mut Vec<Action>,
) -> bool {
    let Some((item, geometry)) = target else {
        if state.cancel() {
            actions.push(Action::EffectPointEnd { commit: false });
        }
        return false;
    };
    let found = handles(cx, item, geometry, viewport);
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let down = ui.input(|i| i.pointer.primary_down());
    if let Some(mut drag) = state.drag.take() {
        if down {
            if let Some(pointer) = pointer
                && let Some(point) = fraction(geometry, viewport, pointer)
                && drag.last != Some(point)
            {
                drag.last = Some(point);
                actions.push(Action::EffectPointPreview {
                    item: drag.item,
                    effect: drag.effect.clone(),
                    key: drag.key.clone(),
                    point,
                });
            }
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            state.drag = Some(drag);
        } else {
            actions.push(Action::EffectPointEnd {
                commit: drag.last.is_some_and(|last| last != drag.start),
            });
        }
        paint(ui, &found, cx.dark);
        return true;
    }
    paint(ui, &found, cx.dark);
    register(ui, &found, actions);
    let Some(pointer) = pointer else {
        return false;
    };
    let nearest = found
        .iter()
        .map(|handle| (handle.screen.distance(pointer), handle))
        .filter(|(distance, _)| *distance <= kit::size::TRANSFORM_HIT)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, handle)| handle);
    let Some(handle) = nearest else {
        return false;
    };
    ui.ctx().set_cursor_icon(CursorIcon::Grab);
    if ui.input(|i| i.pointer.primary_pressed()) && frame.contains(pointer) {
        state.drag = Some(Drag {
            item,
            effect: handle.effect.clone(),
            key: handle.key.to_owned(),
            start: handle.point,
            last: None,
        });
        return true;
    }
    false
}

fn paint(ui: &egui::Ui, handles: &[Handle], dark: bool) {
    for handle in handles {
        paint_handle(ui, handle.screen, kit::size::TRANSFORM_HANDLE / 2.0, dark);
    }
}

/// One stable automation target per handle, named by effect and parameter.
fn register(ui: &mut egui::Ui, handles: &[Handle], actions: &mut Vec<Action>) {
    for handle in handles {
        let id = format!("preview.effect.{}.{}", handle.effect, handle.key);
        let response = ui.interact(
            egui::Rect::from_center_size(
                handle.screen,
                Vec2::splat(kit::size::TRANSFORM_HIT * 2.0),
            ),
            ui.make_persistent_id(&id),
            Sense::click_and_drag(),
        );
        let label = handle.label;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        actions.push(Action::Remember {
            id,
            icon: None,
            tooltip: Some(label.to_owned()),
            response,
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use project::{
        Color, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
    };
    use time::{SourceRange, TICKS_PER_SECOND, Tick};

    /// A 200x100 canvas holding one overlay; the viewport maps canvas
    /// pixels to screen pixels 1:1.
    fn setup() -> (ItemGeometry, Viewport) {
        let mut data = Project::empty(ProjectId(38), "Point".into())
            .unwrap()
            .into_data();
        data.canvas.width = 200;
        data.canvas.height = 100;
        let item = testkit::item::item(
            ItemId(1),
            ItemParams::Overlay(OverlayParams {
                color: Color([255; 4]),
            }),
        )
        .source(SourceRange::new(Tick::ZERO, Tick(20 * TICKS_PER_SECOND)).unwrap())
        .build();
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(1),
            name: "V".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![item],
        });
        let project = Project::new(data).unwrap();
        let renderer =
            render::Renderer::new(&project, render::RenderSettings::from_project(&project))
                .unwrap();
        let viewport = Viewport::new(
            render::Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            [200, 100],
            1.0,
            [0.0, 0.0],
        )
        .unwrap();
        let geometry = renderer
            .item_geometry(ItemId(1), Tick::ZERO)
            .unwrap()
            .unwrap();
        (geometry, viewport)
    }

    #[test]
    fn fractions_round_trip_through_the_item_box_and_clamp_to_the_frame() {
        let (geometry, viewport) = setup();
        let bounds = geometry.bounds();
        for point in [[0.0, 0.0], [1.0, 0.0], [0.25, 0.75], [1.0, 1.0]] {
            let [x, y] = geometry
                .point(
                    &viewport,
                    [
                        bounds.x + point[0] * bounds.width,
                        bounds.y + point[1] * bounds.height,
                    ],
                )
                .unwrap();
            let back = fraction(&geometry, &viewport, egui::pos2(x as f32, y as f32)).unwrap();
            assert!(
                (back[0] - point[0]).abs() < 1e-4 && (back[1] - point[1]).abs() < 1e-4,
                "{point:?} -> {back:?}"
            );
        }
        // A drag far outside the frame stays on its edge instead of leaving the model's range.
        let far = fraction(&geometry, &viewport, egui::pos2(-5000.0, 9000.0)).unwrap();
        assert_eq!(far, [0.0, 1.0]);
    }
}
