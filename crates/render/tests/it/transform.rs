#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::common::{Empty, Hooks, solid_project};
use project::{AnimationPath, Param, ParamValue};
use render::{RenderSettings, Renderer};
use time::Tick;

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
