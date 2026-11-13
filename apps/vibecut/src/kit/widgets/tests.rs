use super::*;
use egui_kittest::{Harness, kittest::Queryable};

/// The semibold label family only exists once the app fonts are installed.
fn harness<'a, S>(app: impl FnMut(&mut Ui, &mut S) + 'a, state: S) -> Harness<'a, S> {
    Harness::builder()
        .with_context_setup(crate::kit::install_fonts)
        .build_ui_state(app, state)
}

#[test]
fn section_header_keeps_reset_and_toggle_separate() {
    let mut harness = harness(
        |ui, state: &mut (bool, bool)| {
            let header = section_header_with(
                ui,
                "section",
                "Transform",
                &mut state.0,
                Some("Reset"),
                Some("Key"),
                None,
            );
            state.1 |= header.reset.is_some_and(|reset| reset.clicked());
        },
        (true, false),
    );
    harness.run();
    harness.get_by_label("Reset").click();
    harness.run();
    assert_eq!(*harness.state(), (true, true), "reset must not collapse");
    harness.get_by_label("Transform").click();
    harness.run();
    assert!(!harness.state().0, "the label toggles the section");
}

#[test]
fn buttons_dispatch_clicks_only_when_enabled() {
    for variant in ["icon", "primary", "play", "round", "badge"] {
        for enabled in [false, true] {
            let mut harness = harness(
                move |ui, clicks: &mut u32| {
                    ui.add_enabled_ui(enabled, |ui| {
                        let response = match variant {
                            "primary" => {
                                action_button(ui, "tool", Icon::Plus, "Add", size::EXPORT_WIDTH)
                            }
                            "play" => transport_button(ui, "tool", Icon::Play, "Add", false),
                            "round" => {
                                transport_button(ui, "tool", Icon::ArrowClockwise, "Add", true)
                            }
                            "badge" => badge_button(ui, "tool", "Add"),
                            _ => icon_button(ui, "tool", Icon::Plus, "Add", true),
                        };
                        if response.clicked() {
                            *clicks += 1;
                        }
                    });
                },
                0,
            );
            harness.run();
            harness.get_by_label("Add").click();
            harness.run();
            assert_eq!(*harness.state(), u32::from(enabled), "{variant}/{enabled}");
        }
    }
}

#[test]
fn rail_slider_maps_a_press_to_its_fraction() {
    let mut harness = harness(
        |ui, value: &mut f32| {
            let rect = Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(96.0, 24.0));
            zoom_rail(ui, "zoom", rect, value);
        },
        0.0,
    );
    harness.run();
    harness.get_by_label("zoom").click();
    harness.run();
    let value = *harness.state();
    assert!((value - 0.5).abs() < 0.05, "centre press, got {value}");
}

#[test]
fn section_header_enable_checkbox_is_its_own_control() {
    let mut harness = harness(
        |ui, state: &mut (bool, bool)| {
            let header = section_header_with(
                ui,
                "section",
                "Blend",
                &mut state.0,
                Some("Reset"),
                None,
                Some(("Enable", false)),
            );
            state.1 |= header.enable.is_some_and(|enable| enable.clicked());
        },
        (true, false),
    );
    harness.run();
    harness.get_by_label("Enable").click();
    harness.run();
    assert_eq!(
        *harness.state(),
        (true, true),
        "the checkbox must not collapse"
    );
}

#[test]
fn keyframe_cluster_reports_each_cell_and_disables_empty_sides() {
    let mut harness = harness(
        |ui, clicks: &mut [u32; 3]| {
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(size::KEYFRAME_CLUSTER, size::CONTROL),
                Sense::hover(),
            );
            let cells = keyframe_cluster(
                ui,
                "key",
                rect,
                &KeyframeControl {
                    previous_label: "Previous",
                    toggle_label: "Toggle",
                    next_label: "Next",
                    keyed: false,
                    can_toggle: true,
                    has_previous: false,
                    has_next: true,
                },
            );
            for (count, cell) in clicks.iter_mut().zip(&cells) {
                *count += u32::from(cell.clicked());
            }
        },
        [0; 3],
    );
    harness.run();
    assert!(
        harness
            .get_by_label("Previous")
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label("Previous").click();
    harness.run();
    harness.get_by_label("Toggle").click();
    harness.run();
    harness.get_by_label("Next").click();
    harness.run();
    assert_eq!(*harness.state(), [0, 1, 1], "an empty side never fires");
}
