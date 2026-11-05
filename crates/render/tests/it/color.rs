#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::common::{Empty, Hooks, solid_project};
use project::{AnimationPath, Param, ParamValue};
use render::{RenderSettings, Renderer};
use time::Tick;

#[test]
fn graphic_stroke_and_text_background_reach_the_shared_compositor() {
    let hooks = Hooks::default();
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
    item.animations
        .set(
            AnimationPath::new("params.strokeColor").unwrap(),
            Param::new(ParamValue::Text("#00FF00FF".into())).unwrap(),
        )
        .unwrap();
    let project = project::Project::new(data).unwrap();
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let frame = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_eq!(frame.pixels()[0], [0., 1., 0., 1.]);
    assert_eq!(frame.pixels()[16 * 64 + 32], [1., 0., 0., 1.]);

    let mut data = project.into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = project::TrackKind::Text;
    let item = track.items.first_mut().unwrap();
    item.content = project::ItemParams::Text(project::TextParams {
        content: "A".into(),
        font_size: 8.,
        color: project::Color([0; 4]),
        background: Some(project::TextBackground {
            color: project::Color([0, 255, 0, 255]),
            padding: 0.,
            radius: 0.,
        }),
        ..Default::default()
    });
    item.animations = Default::default();
    let project = project::Project::new(data).unwrap();
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let frame = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert!(frame.pixels().iter().any(|p| p[1] > 0.9 && p[0] < 0.01));
    assert_eq!(frame.pixels()[0], [0., 0., 0., 1.]);
}
