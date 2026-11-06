use super::*;
use egui_kittest::{Harness, kittest::Queryable};

/// The semibold label family only exists once the app fonts are installed.
fn harness<'a, S>(app: impl FnMut(&mut Ui, &mut S) + 'a, state: S) -> Harness<'a, S> {
    Harness::builder()
        .with_context_setup(crate::kit::install_fonts)
        .build_ui_state(app, state)
}

#[test]
fn tab_strip_selects_the_clicked_cell() {
    let mut harness = harness(
        |ui, selected: &mut usize| {
            tab_strip(ui, "tabs", &[("One", None), ("Two", None)], selected, 80.0);
        },
        0,
    );
    harness.run();
    harness.get_by_label("Two").click();
    harness.run();
    assert_eq!(*harness.state(), 1);
}

#[test]
fn section_header_keeps_reset_and_toggle_separate() {
    let mut harness = harness(
        |ui, state: &mut (bool, bool)| {
            let header = section_header(ui, "section", "Transform", &mut state.0, "Reset", "Key");
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
fn slider_stepper_arrows_clamp_to_the_range() {
    let mut harness = harness(
        |ui, value: &mut f64| {
            slider_stepper(
                ui,
                value,
                NumberSpec {
                    id: "scale",
                    label: "Scale",
                    unit: "%",
                    range: 0.0..=1.0,
                    step: 5.0,
                },
            );
        },
        0.5,
    );
    harness.run();
    for button in harness.get_all_by_role(egui::Role::Button) {
        button.click();
    }
    harness.run();
    let value = *harness.state();
    assert!(
        value == 0.0 || value == 1.0,
        "a step larger than the range clamps to a bound, got {value}"
    );
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
