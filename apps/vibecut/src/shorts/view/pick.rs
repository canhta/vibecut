//! The pick screen: hero, drop zone card and the create button.
use super::{
    State,
    widgets::{self, Look, bold, corner},
};
use crate::{
    egui::{self, Align2, Pos2, Rect, Stroke, Vec2, pos2, vec2},
    i18n::Text,
    kit::{self, Icon},
};

/// Measured at the 1718×989 window (reference 221941), from the content origin.
const HERO_LINES: [f32; 2] = [76.0, 125.0];
const HERO_SHIFT: f32 = -65.0;
const ART_SHIFT: f32 = 270.0;
const CARD: [f32; 2] = [920.0, 440.0];
const CARD_TOP: f32 = 200.0;
const CARD_RADIUS: f32 = 16.0;
const FRAME_INSET: f32 = 24.0;
const ICON_Y: f32 = 154.0;
const LABEL_Y: f32 = 202.0;
const BUTTON_Y: f32 = 271.0;
const BUTTON: [f32; 2] = [200.0, 40.0];
const SQUARE: f32 = 40.0;

pub(super) fn show(state: &mut State, ui: &mut egui::Ui, rect: Rect) {
    let dark = state.preferences.dark;
    let centre = rect.center().x;
    let top = rect.top();
    let painter = ui.painter().clone();
    for ((text, color), line) in [
        (state.tr(Text::ShortsHero), kit::text(dark)),
        (state.tr(Text::ShortsHeroAccent), kit::accent(dark)),
    ]
    .into_iter()
    .zip(HERO_LINES)
    {
        painter.text(
            pos2(centre + HERO_SHIFT, top + line),
            Align2::CENTER_CENTER,
            text,
            bold(38.5),
            color,
        );
    }
    stack_art(ui, pos2(centre + ART_SHIFT, top + 90.0), dark);

    let width = CARD[0].min((rect.width() - 80.0).max(320.0));
    let height = CARD[1].min((rect.height() - CARD_TOP - 40.0).max(240.0));
    let card = Rect::from_min_size(
        pos2(centre - width / 2.0, top + CARD_TOP),
        vec2(width, height),
    );
    painter.rect_filled(card, corner(CARD_RADIUS), kit::panel(dark));
    let hovered = ui.input(|i| !i.raw.hovered_files.is_empty());
    let stroke = Stroke::new(
        1.0,
        if hovered {
            kit::accent(dark)
        } else {
            kit::border(dark)
        },
    );
    widgets::dashed_frame(ui, card.shrink(FRAME_INSET), 8.0, stroke);

    let has_video = !state.request.path.as_os_str().is_empty();
    kit::tinted(Icon::UploadSimple, 44.0, kit::text(dark)).paint_at(
        ui,
        Rect::from_center_size(pos2(centre, card.top() + ICON_Y), Vec2::splat(44.0)),
    );
    let label = if has_video {
        state
            .request
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        state.tr(Text::ShortsDrop).to_owned()
    };
    painter.text(
        pos2(centre, card.top() + LABEL_Y),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(18.0),
        kit::text(dark),
    );

    let y = card.top() + BUTTON_Y;
    let button = |x: f32, w: f32| Rect::from_min_size(pos2(x, y), vec2(w, BUTTON[1]));
    let idle = !state.busy();
    if has_video {
        let total = SQUARE * 2.0 + BUTTON[0] + 16.0;
        let left = centre - total / 2.0;
        let replace = button(left, SQUARE);
        let create = button(left + SQUARE + 8.0, BUTTON[0]);
        let options = button(left + SQUARE + 16.0 + BUTTON[0], SQUARE);
        if widgets::icon_tap(
            ui,
            "shorts.pick",
            replace,
            Icon::FolderOpen,
            state.tr(Text::ShortsReplace),
            idle,
            false,
        )
        .clicked()
        {
            state.pick(ui.ctx(), super::super::state::Pick::Video);
        }
        let go = widgets::button(
            ui,
            "shorts.generate",
            create,
            Some(Icon::Sparkle),
            Some(state.tr(Text::ShortsGenerate)),
            state.tr(Text::ShortsGenerate),
            Look::Primary,
            idle,
        );
        if go.clicked()
            && let Err(e) = state.generate()
        {
            state.fail(e, Text::ShortsError);
        }
        let gear = widgets::icon_tap(
            ui,
            "shorts.options",
            options,
            Icon::SlidersHorizontal,
            state.tr(Text::ShortsOptions),
            true,
            state.options_open,
        );
        if gear.clicked() {
            state.options_open = !state.options_open;
        }
        if state.options_open {
            egui::Area::new(egui::Id::unique("shorts.options.area"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(centre - 190.0 - 16.0, options.bottom() + 8.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(16.0)
                        .show(ui, |ui| super::options::show(state, ui));
                });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.options_open = false;
            }
        }
    } else {
        let choose = button(centre - BUTTON[0] / 2.0, BUTTON[0]);
        if widgets::button(
            ui,
            "shorts.pick",
            choose,
            None,
            Some(state.tr(Text::ShortsPick)),
            state.tr(Text::ShortsPick),
            Look::Primary,
            idle,
        )
        .clicked()
        {
            state.pick(ui.ctx(), super::super::state::Pick::Video);
        }
    }
    super::recent::show(state, ui, rect, card.left(), card.width());
}

/// Three portrait cards stacked and turned, the hero's own illustration.
fn stack_art(ui: &egui::Ui, centre: Pos2, dark: bool) {
    let painter = ui.painter();
    let size = vec2(60.0, 92.0);
    for (offset, angle, fill) in [
        (vec2(34.0, 4.0), 0.30_f32, kit::hover(dark)),
        (vec2(16.0, 2.0), 0.16, kit::field(dark)),
        (vec2(-8.0, 0.0), -0.12, kit::panel(dark)),
    ] {
        widgets::tilted_card(
            painter,
            centre + offset,
            size,
            angle,
            fill,
            Stroke::new(1.0, kit::border(dark)),
        );
    }
}
