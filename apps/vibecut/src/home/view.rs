//! Draws the Home window: rail, top band, create banner, tool card and project rows.
//!
//! Geometry is the measured desktop reference at the 1728×1030 window, in
//! logical px from the window's top-left; the rail starts at x 0, the page
//! 240 px in, and the banner sits 44 px down.

mod chrome;
mod create;
mod dialogs;
mod projects;
mod widgets;

use super::ProjectsPreferences;
use super::state::State;
use crate::{egui, i18n::Text, kit::Look};

/// Rail width; the page starts here.
pub(super) const RAIL: f32 = 240.0;
/// Rail rows sit 20 px in and are 176×32.
const RAIL_INSET: f32 = 20.0;
const RAIL_ROW: [f32; 2] = [176.0, 32.0];
const RAIL_RULE_GAP: f32 = 20.0;
const ROW_RADIUS: f32 = 6.0;
/// Top band height; the banner starts below it and rail rows start level with it.
const TOP_BAND: f32 = 44.0;
/// The page keeps 60 px to the right of the banner.
const PAGE_RIGHT: f32 = 60.0;
const BANNER_HEIGHT: f32 = 144.0;
const CARD_RADIUS: f32 = 8.0;
const TOOL_CARD: [f32; 2] = [242.0, 136.0];
const BLOCK_GAP: f32 = 20.0;
const SECTION_GAP: f32 = 64.0;
/// Project thumbnails: 124 px squares on a 146 px pitch, 5 px in from the page edge.
const THUMB: f32 = 124.0;
const THUMB_RADIUS: f32 = 6.0;
const GRID_GAP: f32 = 22.0;
const GRID_INSET: f32 = 5.0;
/// The thumbnail row runs 20 px past the banner's right edge.
const GRID_BLEED: f32 = 20.0;
const ROW_GAP: f32 = 24.0;

pub(super) fn show(
    state: &mut State,
    ui: &mut egui::Ui,
    look: Look,
    projects: &mut ProjectsPreferences,
    status: Option<Text>,
) {
    let locale = look.locale;
    let full = ui.max_rect();
    let rail = egui::Rect::from_min_size(full.min, egui::vec2(RAIL, full.height()));
    let page = egui::Rect::from_min_max(egui::pos2(full.min.x + RAIL, full.min.y), full.max);
    let band = egui::Rect::from_min_size(page.min, egui::vec2(page.width(), TOP_BAND));
    chrome::rail(ui, rail, state, locale);
    chrome::top_bar(ui, band, state, locale, status);

    let body = egui::Rect::from_min_max(egui::pos2(page.min.x, page.min.y + TOP_BAND), page.max);
    let mut body_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(&mut body_ui, |ui| {
            let width = (body.width() - PAGE_RIGHT).max(240.0);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            create::banner(ui, width, state, locale);
            ui.add_space(BLOCK_GAP);
            create::shorts_card(ui, state, locale);
            ui.add_space(SECTION_GAP);
            projects::section(ui, state, look, projects);
        });
    dialogs::show(state, ui.ctx(), look);
}
