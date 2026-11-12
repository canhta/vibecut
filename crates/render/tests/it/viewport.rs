#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support::{Empty, Hooks, solid_project};
use project::{AnimationPath, ItemId, Param, ParamValue, Project, TrackId};
use render::{
    Placement, Rect, RenderSettings, Renderer,
    viewport::{Handle, Viewport},
};
use time::Tick;

fn near(actual: [f64; 2], expected: [f64; 2]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-8, "{a} != {b}");
    }
}
fn screen() -> Rect {
    Rect {
        x: 10.,
        y: 20.,
        width: 400.,
        height: 300.,
    }
}
fn transformed() -> Project {
    let mut data = solid_project(1, 200, 100, [255; 4]).into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    for (path, value) in [
        ("positionX", 10.),
        ("positionY", -5.),
        ("scaleX", 0.5),
        ("scaleY", 2.),
        ("rotate", 90.),
    ] {
        item.animations
            .set(
                AnimationPath::new(format!("transform.{path}")).unwrap(),
                Param::new(ParamValue::Scalar(value)).unwrap(),
            )
            .unwrap();
    }
    Project::new(data).unwrap()
}
fn flipped(axes: &[project::FlipAxis]) -> Project {
    let mut data = transformed().into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    for axis in axes {
        item.animations
            .set(
                AnimationPath::from(axis.builtin()),
                Param::new(ParamValue::Bool(true)).unwrap(),
            )
            .unwrap();
    }
    Project::new(data).unwrap()
}
fn settings(project: &Project) -> RenderSettings {
    let mut settings = RenderSettings::from_project(project);
    settings.placements.insert(
        ItemId(0),
        Placement {
            destination: Rect {
                x: 0.2,
                y: 0.3,
                width: 0.4,
                height: 0.2,
            },
            ..Default::default()
        },
    );
    settings
}

#[test]
fn viewport_round_trips_letterbox_zoom_pan_and_refuses_hostile_geometry() {
    for (canvas, zoom, pan) in [
        ([200, 100], 1., [0., 0.]),
        ([100, 200], 0.25, [-32., 70.]),
        ([200, 100], 3., [30., -10.]),
    ] {
        let viewport = Viewport::new(screen(), canvas, zoom, pan).unwrap();
        for point in [[0., 0.], [100., 50.], [-100., -90.], [2000., 3000.]] {
            near(
                viewport
                    .screen_to_canvas(viewport.canvas_to_screen(point).unwrap())
                    .unwrap(),
                point,
            );
        }
        near(
            viewport
                .canvas_to_screen([f64::from(canvas[0]) / 2., f64::from(canvas[1]) / 2.])
                .unwrap(),
            [210. + pan[0], 170. + pan[1]],
        );
    }
    let viewport = Viewport::new(screen(), [200, 100], 1., [0., 0.]).unwrap();
    near(viewport.canvas_to_screen([0., 0.]).unwrap(), [10., 70.]);
    assert_eq!(viewport.canvas_distance(6.).unwrap(), 3.);
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Viewport::new(screen(), [200, 100], value, [0., 0.]).is_err());
        assert!(Viewport::new(screen(), [200, 100], 1., [value, 0.]).is_err());
        assert!(viewport.screen_to_canvas([value, 0.]).is_err());
        assert!(viewport.canvas_to_screen([0., value]).is_err());
        assert!(viewport.canvas_distance(value).is_err());
    }
    for zoom in [0., -1., f64::MAX, f64::from_bits(1)] {
        assert!(Viewport::new(screen(), [200, 100], zoom, [0., 0.]).is_err());
    }
    for canvas in [[0, 100], [100, 0]] {
        assert!(Viewport::new(screen(), canvas, 1., [0., 0.]).is_err());
    }
    for width in [0., -1., f64::NAN] {
        assert!(Viewport::new(Rect { width, ..screen() }, [200, 100], 1., [0., 0.]).is_err());
    }
    assert!(viewport.canvas_distance(-1.).is_err());
    assert!(Viewport::new(screen(), [200, 100], 1., [f64::MAX, 0.]).is_err());
}

#[test]
fn rotated_handles_and_per_axis_scaling_share_render_geometry() {
    let project = transformed();
    let renderer = Renderer::new(&project, settings(&project)).unwrap();
    let viewport = Viewport::new(screen(), [200, 100], 1., [0., 0.]).unwrap();
    let geometry = renderer
        .item_geometry(ItemId(0), Tick::ZERO)
        .unwrap()
        .unwrap();
    for (handle, point) in Handle::ALL.into_iter().zip([
        [110., 15.],
        [110., 35.],
        [110., 55.],
        [90., 55.],
        [70., 55.],
        [70., 35.],
        [70., 15.],
        [90., 15.],
    ]) {
        near(
            geometry.handle_position(&viewport, handle).unwrap(),
            viewport.canvas_to_screen(point).unwrap(),
        );
    }
    // The public bounds and point map agree with the transformed handles.
    let bounds = geometry.bounds();
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (40., 30., 80., 20.)
    );
    near(
        geometry.point(&viewport, [40., 30.]).unwrap(),
        geometry
            .handle_position(&viewport, Handle::TopLeft)
            .unwrap(),
    );
    near(
        geometry.point(&viewport, [120., 50.]).unwrap(),
        geometry
            .handle_position(&viewport, Handle::BottomRight)
            .unwrap(),
    );
    near(
        geometry
            .inverse(&viewport, geometry.point(&viewport, [60., 40.]).unwrap())
            .unwrap(),
        [60., 40.],
    );
    assert!(geometry.point(&viewport, [f64::NAN, 0.]).is_err());
    assert!(geometry.inverse(&viewport, [f64::INFINITY, 0.]).is_err());
    // Right is rotated downwards; crossing the centre must not flip the image.
    near(
        geometry.scale_at(Handle::Right, [90., 55.]).unwrap(),
        [0.5, 2.],
    );
    near(
        geometry.scale_at(Handle::Right, [90., -5.]).unwrap(),
        [0.01, 2.],
    );
    near(
        geometry.scale_at(Handle::Top, [90., 35.]).unwrap(),
        [0.5, 0.01],
    );
    near(
        geometry
            .scale_at(Handle::BottomRight, [-1000., 1000.])
            .unwrap(),
        [10., 10.],
    );
    assert!(geometry.scale_at(Handle::Top, [f64::NAN, 0.]).is_err());
    let corner = geometry
        .handle_position(&viewport, Handle::TopLeft)
        .unwrap();
    assert_eq!(
        geometry.hit_handle(&viewport, corner, 0.).unwrap(),
        Some(Handle::TopLeft)
    );
    assert_eq!(
        geometry
            .hit_handle(&viewport, [corner[0] + 6., corner[1]], 6.)
            .unwrap(),
        Some(Handle::TopLeft)
    );
    assert_eq!(
        geometry
            .hit_handle(&viewport, [corner[0] + 6.01, corner[1]], 6.)
            .unwrap(),
        None
    );
    for radius in [-1., f64::NAN, f64::INFINITY] {
        assert!(
            geometry
                .hit_handle(&viewport, [230., 100.], radius)
                .is_err()
        );
    }
    near(
        geometry.rotation_handle_position(&viewport, 24.).unwrap(),
        [126., 140.],
    );
    let zoomed = Viewport::new(screen(), [200, 100], 2., [5., 6.]).unwrap();
    near(
        geometry.handle_position(&zoomed, Handle::TopLeft).unwrap(),
        [255., 36.],
    );
    near(
        geometry.rotation_handle_position(&zoomed, 24.).unwrap(),
        [71., 116.],
    );
    for gap in [-1., f64::NAN, f64::INFINITY] {
        assert!(geometry.rotation_handle_position(&viewport, gap).is_err());
    }
    assert!(renderer.item_geometry(ItemId(0), Tick(-1)).is_err());
    assert!(
        renderer
            .item_geometry(ItemId(999), Tick::ZERO)
            .unwrap()
            .is_none()
    );
    // The centre is in the rotated box; its old unrotated left side is outside.
    for (canvas, expected) in [([90., 35.], Some(ItemId(0))), ([45., 35.], None)] {
        assert_eq!(
            renderer
                .hit_test(
                    &viewport,
                    viewport.canvas_to_screen(canvas).unwrap(),
                    Tick::ZERO
                )
                .unwrap(),
            expected
        );
    }
    // Cross the public pixel interface: selected centre and excluded point agree.
    let image = Renderer::new(&project, settings(&project))
        .unwrap()
        .render_frame(Tick::ZERO, &mut Empty, &Hooks::default())
        .unwrap();
    assert!(image.pixels()[35 * 200 + 90][0] > 0.9);
    assert_eq!(image.pixels()[35 * 200 + 45][0], 0.);
}

#[test]
fn reflected_handles_scaling_and_hit_bounds_share_the_composed_geometry() {
    let viewport = Viewport::new(screen(), [200, 100], 1., [0., 0.]).unwrap();
    use project::FlipAxis::{Horizontal, Vertical};
    for (axes, top_left) in [
        (&[Horizontal][..], [110., 55.]),
        (&[Vertical][..], [70., 15.]),
        (&[Horizontal, Vertical][..], [70., 55.]),
    ] {
        let project = flipped(axes);
        let renderer = Renderer::new(&project, settings(&project)).unwrap();
        let geometry = renderer
            .item_geometry(ItemId(0), Tick::ZERO)
            .unwrap()
            .unwrap();
        near(
            geometry
                .handle_position(&viewport, Handle::TopLeft)
                .unwrap(),
            viewport.canvas_to_screen(top_left).unwrap(),
        );
        // The canvas-axis envelope is unchanged by a centred reflection.
        let mut xs = [f64::MAX, f64::MIN];
        let mut ys = [f64::MAX, f64::MIN];
        for handle in [
            Handle::TopLeft,
            Handle::TopRight,
            Handle::BottomLeft,
            Handle::BottomRight,
        ] {
            let canvas = viewport
                .screen_to_canvas(geometry.handle_position(&viewport, handle).unwrap())
                .unwrap();
            xs = [xs[0].min(canvas[0]), xs[1].max(canvas[0])];
            ys = [ys[0].min(canvas[1]), ys[1].max(canvas[1])];
        }
        near([xs[0], ys[0]], [70., 15.]);
        near([xs[1], ys[1]], [110., 55.]);
        // Logical Right follows the reflected side: visual left on H, visual top on V.
        let right = if axes == [Vertical] {
            [90., 55.]
        } else {
            [90., 15.]
        };
        near(
            geometry.handle_position(&viewport, Handle::Right).unwrap(),
            viewport.canvas_to_screen(right).unwrap(),
        );
        // Dragging it outward keeps a positive magnitude and the reflection.
        if axes == [Horizontal] {
            near(
                geometry.scale_at(Handle::Right, [90., 5.]).unwrap(),
                [0.75, 2.],
            );
            near(
                geometry.scale_at(Handle::Right, [90., 45.]).unwrap(),
                [0.01, 2.],
            );
        }
        let centre = viewport.canvas_to_screen([90., 35.]).unwrap();
        assert_eq!(
            renderer.hit_test(&viewport, centre, Tick::ZERO).unwrap(),
            Some(ItemId(0))
        );
    }
}

#[test]
fn hit_test_uses_render_order_visibility_locks_and_half_open_edges() {
    for excluded in [
        "none",
        "hidden",
        "locked",
        "opacity",
        "transparent",
        "stroke",
    ] {
        let mut data = solid_project(1, 200, 100, [255; 4]).into_data();
        let mut top = data.scene().timeline.overlays[0].clone();
        top.id = TrackId(2);
        top.items[0].id = ItemId(2);
        match excluded {
            "hidden" => top.hidden = true,
            "locked" => top.locked = true,
            "opacity" => top.items[0]
                .animations
                .set(
                    AnimationPath::new("opacity").unwrap(),
                    Param::new(ParamValue::Scalar(0.)).unwrap(),
                )
                .unwrap(),
            "transparent" | "stroke" => {
                top.items[0].content = project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([0; 4]),
                });
                if excluded == "stroke" {
                    top.items[0]
                        .animations
                        .set(
                            AnimationPath::new("params.strokeColor").unwrap(),
                            Param::new(ParamValue::Text("#FFFFFFFF".into())).unwrap(),
                        )
                        .unwrap();
                }
            }
            _ => {}
        }
        data.scene_mut().timeline.insert_track(top);
        let project = Project::new(data).unwrap();
        let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
        let viewport = Viewport::new(screen(), [200, 100], 1., [0., 0.]).unwrap();
        assert_eq!(
            renderer
                .hit_test(&viewport, [210., 170.], Tick::ZERO)
                .unwrap(),
            Some(ItemId(if matches!(excluded, "none" | "stroke") {
                2
            } else {
                0
            }))
        );
        for point in [[10., 20.], [210., 69.], [410., 170.], [210., 270.]] {
            assert_eq!(
                renderer.hit_test(&viewport, point, Tick::ZERO).unwrap(),
                None
            );
        }
        assert_eq!(
            renderer
                .hit_test(&viewport, [10., 70.], Tick::ZERO)
                .unwrap(),
            Some(ItemId(if matches!(excluded, "none" | "stroke") {
                2
            } else {
                0
            }))
        );
        let end = project.duration();
        assert_eq!(
            renderer.hit_test(&viewport, [210., 170.], end).unwrap(),
            None
        );
        assert!(
            renderer
                .hit_test(&viewport, [210., 170.], Tick(-1))
                .is_err()
        );
        assert!(
            renderer
                .hit_test(&viewport, [f64::INFINITY, 0.], Tick::ZERO)
                .is_err()
        );
        let wrong = Viewport::new(screen(), [100, 100], 1., [0., 0.]).unwrap();
        assert!(renderer.hit_test(&wrong, [210., 170.], Tick::ZERO).is_err());
        let zoomed = Viewport::new(screen(), [200, 100], 2., [0., 0.]).unwrap();
        assert_eq!(
            renderer.hit_test(&zoomed, [0., 170.], Tick::ZERO).unwrap(),
            None
        );
        let panned = Viewport::new(screen(), [200, 100], 2., [100., 0.]).unwrap();
        assert_eq!(
            renderer
                .hit_test(&panned, [310., 170.], Tick::ZERO)
                .unwrap(),
            Some(ItemId(if matches!(excluded, "none" | "stroke") {
                2
            } else {
                0
            }))
        );
    }
}

#[test]
fn item_geometry_uses_item_local_animation_time_and_empty_text_visibility() {
    let mut data = solid_project(1, 200, 100, [255; 4]).into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    item.start = Tick(50);
    item.source_range = time::SourceRange::new(Tick(1000), Tick(1100)).unwrap();
    item.animations
        .set(
            AnimationPath::new("transform.positionX").unwrap(),
            Param::Value(
                project::Channel::new(
                    ParamValue::Scalar(0.),
                    vec![
                        project::Keyframe::new(Tick(0), ParamValue::Scalar(0.)),
                        project::Keyframe::new(Tick(20), ParamValue::Scalar(40.)),
                    ],
                    project::Extrapolation::Hold,
                    project::Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let project = Project::new(data).unwrap();
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let viewport = Viewport::new(screen(), [200, 100], 1., [0., 0.]).unwrap();
    assert!(
        renderer
            .item_geometry(ItemId(0), Tick(49))
            .unwrap()
            .is_none()
    );
    let geometry = renderer
        .item_geometry(ItemId(0), Tick(60))
        .unwrap()
        .unwrap();
    near(
        geometry.handle_position(&viewport, Handle::Left).unwrap(),
        [50., 170.],
    );
    assert!(
        renderer
            .item_geometry(ItemId(0), Tick(150))
            .unwrap()
            .is_none()
    );
    let mut data = project.into_data();
    let track = &mut data.scene_mut().timeline.overlays[0];
    track.kind = project::TrackKind::Text;
    track.items[0].content = project::ItemParams::Text(project::TextParams::new(""));
    let project = Project::new(data).unwrap();
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    assert!(
        renderer
            .item_geometry(ItemId(0), Tick(60))
            .unwrap()
            .is_none()
    );
}

#[test]
fn graphic_hit_testing_uses_centered_bounds_and_evaluated_fill_or_stroke_alpha() {
    let mut data = solid_project(1, 800, 600, [0; 4]).into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    item.content = project::ItemParams::Graphic(project::GraphicParams::Ellipse {
        fill: project::Color([0; 4]),
        stroke: project::Color([255; 4]),
        stroke_width: 20.,
        stroke_align: project::StrokeAlign::Outside,
    });
    let mut alpha = project::Channel::new(
        ParamValue::Scalar(0.),
        vec![
            project::Keyframe::new(Tick(0), ParamValue::Scalar(0.)),
            project::Keyframe::new(Tick(10), ParamValue::Scalar(1.)),
        ],
        project::Extrapolation::Hold,
        project::Extrapolation::Hold,
    )
    .unwrap();
    alpha
        .put(project::Keyframe::new(Tick(20), ParamValue::Scalar(0.)))
        .unwrap();
    let mut color = Param::new(ParamValue::Color([1., 0., 0., 0.])).unwrap();
    *color.channel_mut(Some(project::ColorComponent::A)).unwrap() = alpha;
    item.animations
        .set(AnimationPath::new("params.fill").unwrap(), color)
        .unwrap();
    item.animations
        .set(
            AnimationPath::new("params.stroke").unwrap(),
            Param::new(ParamValue::Color([0., 0., 1., 0.])).unwrap(),
        )
        .unwrap();
    let p = Project::new(data).unwrap();
    let mut renderer = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    assert!(
        renderer
            .item_geometry(ItemId(0), Tick(0))
            .unwrap()
            .is_none()
    );
    let geometry = renderer
        .item_geometry(ItemId(0), Tick(10))
        .unwrap()
        .unwrap();
    let bounds = geometry.bounds();
    near([bounds.x, bounds.y], [144., 44.]);
    near([bounds.width, bounds.height], [512., 512.]);
    assert!(
        renderer
            .item_geometry(ItemId(0), Tick(20))
            .unwrap()
            .is_none()
    );
    let viewport = Viewport::new(
        Rect {
            x: 0.,
            y: 0.,
            width: 800.,
            height: 600.,
        },
        [800, 600],
        1.,
        [0.; 2],
    )
    .unwrap();
    assert_eq!(
        renderer.hit_test(&viewport, [145., 45.], Tick(10)).unwrap(),
        Some(ItemId(0))
    ); // Bounding-box selection includes empty ellipse corner.
    let h = Hooks::default();
    renderer.render_frame(Tick(10), &mut Empty, &h).unwrap();
    assert_eq!(renderer.text_cache_stats().entries, 0);
}

#[test]
fn mask_geometry_matches_the_compositor_affine_under_mask_and_item_rotation() {
    let mut data = transformed().into_data();
    let mut mask = project::Mask::new(
        "path",
        project::MaskShape::Path {
            path: project::MaskPath::ellipse(),
        },
    );
    mask.center_x = 0.0;
    mask.center_y = 0.0;
    mask.width = 0.4;
    mask.height = 0.4;
    mask.rotation = 90.0;
    data.scene_mut().timeline.overlays[0].items[0]
        .masks
        .push(mask);
    let project = Project::new(data).unwrap();
    let mut settings = RenderSettings::from_project(&project);
    settings.placements.insert(
        ItemId(0),
        Placement {
            destination: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            ..Default::default()
        },
    );
    let viewport = Viewport::new(
        Rect {
            x: 10.,
            y: 20.,
            width: 400.,
            height: 200.,
        },
        [200, 100],
        1.0,
        [0., 0.],
    )
    .unwrap();
    let renderer = Renderer::new(&project, settings).unwrap();
    let geometry = renderer
        .mask_geometry(ItemId(0), "path", Tick::ZERO)
        .unwrap()
        .unwrap();
    // Literal oracle: anchor (0,0) and its (0.25,0) outgoing handle endpoint.
    near(
        geometry.point(&viewport, [0.0, 0.0]).unwrap(),
        [390.0, 130.0],
    );
    near(
        geometry.point(&viewport, [0.25, 0.0]).unwrap(),
        [310.0, 130.0],
    );
    for local in [[0.0, 0.0], [0.5, 0.5], [1.0, 1.0], [0.25, 0.75]] {
        near(
            geometry
                .inverse(&viewport, geometry.point(&viewport, local).unwrap())
                .unwrap(),
            local,
        );
    }
    // The default placement spans the canvas; the same point round-trips.
    let mut data = transformed().into_data();
    let mask = project::Mask::new(
        "path",
        project::MaskShape::Path {
            path: project::MaskPath::ellipse(),
        },
    );
    data.scene_mut().timeline.overlays[0].items[0]
        .masks
        .push(mask);
    let project = Project::new(data).unwrap();
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let geometry = renderer
        .mask_geometry(ItemId(0), "path", Tick::ZERO)
        .unwrap()
        .unwrap();
    near(
        geometry
            .inverse(&viewport, geometry.point(&viewport, [0.25, 0.75]).unwrap())
            .unwrap(),
        [0.25, 0.75],
    );
    // Hidden tracks expose no target.
    let mut settings = RenderSettings::from_project(&project);
    settings.placements.insert(
        ItemId(0),
        Placement {
            destination: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            ..Default::default()
        },
    );
    let mut data = project.data().clone();
    data.scene_mut().timeline.overlays[0].hidden = true;
    let hidden = Project::new(data).unwrap();
    let renderer = Renderer::new(&hidden, settings).unwrap();
    assert!(
        renderer
            .mask_geometry(ItemId(0), "path", Tick::ZERO)
            .unwrap()
            .is_none()
    );
}
