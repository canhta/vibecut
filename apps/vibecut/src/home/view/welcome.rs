//! The first-launch welcome: three steps, shown once and never again.

use super::widgets::set_author_id;
use crate::{
    egui,
    home::state::State,
    i18n::{self, Text},
    kit::{self, Icon, Look},
};

/// Welcome, the honest note on what is missing, then ready.
const STEPS: [(Icon, Text, Text); 3] = [
    (Icon::Sparkle, Text::WelcomeTitle, Text::WelcomeBody),
    (
        Icon::ChatCircle,
        Text::WelcomeMissingTitle,
        Text::WelcomeMissingBody,
    ),
    (
        Icon::FilmStrip,
        Text::WelcomeReadyTitle,
        Text::WelcomeReadyBody,
    ),
];

/// Draw the dialog while `seen` is unset; the last step or Escape sets it.
pub(super) fn show(state: &mut State, ctx: &egui::Context, look: Look, seen: &mut bool) {
    if *seen {
        return;
    }
    let locale = look.locale;
    let step = state.welcome_step.min(STEPS.len() - 1);
    let (glyph, title, body) = STEPS.get(step).copied().unwrap_or(STEPS[0]);
    let last = step == STEPS.len() - 1;
    let (mut back, mut next) = (false, false);
    let response = egui::Modal::new(egui::Id::unique("welcome.dialog"))
        .frame(kit::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(380.0);
            ui.vertical_centered(|ui| {
                ui.add(kit::tinted(glyph, kit::icon::STATE, kit::accent(look.dark)));
                ui.add_space(kit::space::MD);
                kit::dialog_title(ui, i18n::text(locale, title));
                ui.add_space(kit::space::MD);
                ui.label(i18n::text(locale, body));
            });
            ui.add_space(kit::space::DIALOG);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(step > 0, |ui| {
                    back = kit::dialog_button(
                        ui,
                        "welcome.back",
                        i18n::text(locale, Text::WelcomeBack),
                        false,
                    )
                    .clicked();
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (id, label) = if last {
                        ("welcome.start", Text::WelcomeStart)
                    } else {
                        ("welcome.next", Text::WelcomeNext)
                    };
                    next = kit::dialog_button(ui, id, i18n::text(locale, label), true).clicked();
                });
            });
        });
    set_author_id(&response.response, "welcome.dialog");
    if back {
        state.welcome_step = step - 1;
    }
    if next && !last {
        state.welcome_step = step + 1;
    }
    if (next && last) || response.should_close() {
        *seen = true;
        state.welcome_step = 0;
    }
}
