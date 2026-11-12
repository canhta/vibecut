#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support::{Empty, Hooks, solid_project};
use project::{AnimationPath, Param, ParamValue};
use render::{Placement, Rect, RenderSettings, Renderer};
use time::Tick;

#[test]
fn flip_mirrors_prepared_masks_and_composes_with_rotation() {
    let hooks = Hooks::default();
    let masked = |flips: &[project::FlipAxis], rotate: f64| {
        let mut data = solid_project(1, 64, 32, [255, 0, 0, 255]).into_data();
        let item = &mut data.scene_mut().timeline.overlays[0].items[0];
        let mut mask = project::Mask::new("right", project::MaskShape::Rectangle);
        mask.center_x = 0.25;
        mask.width = 0.5;
        mask.height = 1.;
        item.masks = vec![mask];
        for axis in flips {
            item.animations
                .set(
                    AnimationPath::from(axis.builtin()),
                    Param::new(ParamValue::Bool(true)).unwrap(),
                )
                .unwrap();
        }
        if rotate != 0. {
            item.animations
                .set(
                    AnimationPath::new("transform.rotate").unwrap(),
                    Param::new(ParamValue::Scalar(rotate)).unwrap(),
                )
                .unwrap();
        }
        project::Project::new(data).unwrap()
    };
    let halves = |project: &project::Project| {
        let settings = RenderSettings::from_project(project);
        let mut renderer = Renderer::new(project, settings).unwrap();
        let image = renderer
            .render_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        let preview = renderer
            .preview_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        assert_eq!(image.pixels(), preview.pixels());
        let mut left = 0;
        let mut right = 0;
        for (index, pixel) in image.pixels().iter().enumerate() {
            if pixel[0] > 0.9 {
                if index % 64 < 32 {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
        (left, right)
    };
    // The mask covers the item's right half before any transform.
    assert_eq!(halves(&masked(&[], 0.)), (0, 1024));
    assert_eq!(
        halves(&masked(&[project::FlipAxis::Horizontal], 0.)),
        (1024, 0)
    );
    assert_eq!(
        halves(&masked(&[project::FlipAxis::Vertical], 0.)),
        (0, 1024)
    );
    let both = masked(
        &[project::FlipAxis::Horizontal, project::FlipAxis::Vertical],
        0.,
    );
    let rotated = masked(&[], 180.);
    // Both axes equal a 180 degree turn for the prepared content.
    assert_eq!(halves(&both), (1024, 0));
    assert_eq!(halves(&rotated), (1024, 0));
    hooks.state.store(1, std::sync::atomic::Ordering::SeqCst);
    let cancelled = masked(&[project::FlipAxis::Horizontal], 0.);
    let settings = RenderSettings::from_project(&cancelled);
    assert!(matches!(
        Renderer::new(&cancelled, settings)
            .unwrap()
            .render_frame(Tick::ZERO, &mut Empty, &hooks),
        Err(render::Error::Cancelled)
    ));
}

#[test]
fn reflection_rotates_in_project_space_for_unequal_output_gains() {
    let hooks = Hooks::default();
    let mut data = solid_project(1, 200, 100, [255, 0, 0, 255]).into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    let mut corner = project::Mask::new("corner", project::MaskShape::Rectangle);
    corner.center_x = -0.4;
    corner.center_y = -0.4;
    corner.width = 0.2;
    corner.height = 0.2;
    item.masks = vec![corner];
    for (path, value) in [
        ("transform.positionX", 10.),
        ("transform.positionY", -5.),
        ("transform.scaleX", 0.5),
        ("transform.scaleY", 2.),
        ("transform.rotate", 90.),
    ] {
        item.animations
            .set(
                AnimationPath::new(path).unwrap(),
                Param::new(ParamValue::Scalar(value)).unwrap(),
            )
            .unwrap();
    }
    item.animations
        .set(
            AnimationPath::from(project::FlipAxis::Horizontal.builtin()),
            Param::new(ParamValue::Bool(true)).unwrap(),
        )
        .unwrap();
    let project = project::Project::new(data).unwrap();
    let mut settings = RenderSettings::from_project(&project);
    // Destination is the 200x100-canvas box from the viewport geometry case.
    settings.placements.insert(
        project::ItemId(0),
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
    settings.height = 200;
    let mut renderer = Renderer::new(&project, settings).unwrap();
    let image = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_eq!(image.width(), 200);
    assert_eq!(image.height(), 200);
    let (mut sum_x, mut sum_y, mut count) = (0.0, 0.0, 0.0);
    for (index, pixel) in image.pixels().iter().enumerate() {
        if pixel[0] > 0.9 {
            sum_x += (index % 200) as f64;
            sum_y += (index / 200) as f64;
            count += 1.;
        }
    }
    assert!(count > 0.);
    // The marked source corner (48, 32) canvas maps to (106, 51) canvas under
    // `c + t + R F S (p - c)`, then to (106, 102) at output gain (1, 2).
    assert!((sum_x / count - 106.).abs() <= 2.);
    assert!((sum_y / count - 102.).abs() <= 2.);
}

#[test]
fn transform_moves_rotates_scales_and_matches_preview() {
    let hooks = Hooks::default();
    for (position, scale, angle) in [
        (0., 1., 0.),
        (8., 0.5, 0.),
        (0., 0.5, 90.),
        (-8., 0.5, -90.),
        (0., 0.5, 360.),
    ] {
        let mut data = solid_project(1, 64, 32, [255, 0, 0, 255]).into_data();
        let item = data
            .scene_mut()
            .timeline
            .overlays
            .first_mut()
            .unwrap()
            .items
            .first_mut()
            .unwrap();
        for (path, value) in [
            ("transform.positionX", position),
            ("transform.scaleX", scale),
            ("transform.scaleY", scale),
            ("transform.rotate", angle),
        ] {
            item.animations
                .set(
                    AnimationPath::new(path).unwrap(),
                    Param::new(ParamValue::Scalar(value)).unwrap(),
                )
                .unwrap();
        }
        let project = project::Project::new(data).unwrap();
        let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
        let out = renderer
            .render_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        let preview = renderer
            .preview_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        assert_eq!(out.pixels(), preview.pixels());
        let red = out.pixels().iter().filter(|p| p[0] > 0.9).count();
        assert_eq!(red, if scale == 1.0 { 64 * 32 } else { 32 * 16 });
        if scale != 1.0 {
            assert_eq!(out.pixels()[0], [0., 0., 0., 1.]);
        }
    }
    hooks.state.store(1, std::sync::atomic::Ordering::SeqCst);
    let project = solid_project(1, 64, 32, [255, 0, 0, 255]);
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    assert!(matches!(
        renderer.render_frame(Tick::ZERO, &mut Empty, &hooks),
        Err(render::Error::Cancelled)
    ));
}
