//! The review's tools: a strip of tool buttons and the open tool's panel.
use super::{
    State, Tool,
    widgets::{self, bold, corner, icon_tap},
};
use crate::{
    Error,
    catalog::{self, Text},
    crop,
    egui::{self, Align2, Rect, Vec2, pos2, vec2},
    kit::{self, Icon},
    shorts::state::parse_seconds,
};
use engine::ClipRole;

/// Measured at the 1718×989 window (reference 222002/222010), relative to the review panel.
const STRIP_WIDTH: f32 = 56.0;
const STRIP_RIGHT: f32 = 8.0;
const TOOLS_TOP: f32 = 129.0;
const TOOL_HEIGHT: f32 = 52.0;
const PANEL_GAP: f32 = 13.0;
const PANEL_WIDTH: f32 = 242.0;
const PANEL_RADIUS: f32 = 12.0;
const PANEL_PAD: f32 = 16.0;

pub(super) fn show(state: &mut State, ui: &mut egui::Ui, panel: Rect) {
    let dark = state.preferences.dark;
    let tools = [
        (
            Tool::Captions,
            "shorts.tool.captions",
            Icon::Subtitles,
            Text::Caption,
        ),
        (
            Tool::Trim,
            "shorts.tool.trim",
            Icon::Scissors,
            Text::ShortsTrim,
        ),
        (
            Tool::Layout,
            "shorts.tool.layout",
            Icon::SquaresFour,
            Text::ShortsLayout,
        ),
    ];
    let strip = Rect::from_min_size(
        pos2(
            panel.right() - STRIP_RIGHT - STRIP_WIDTH,
            panel.top() + TOOLS_TOP,
        ),
        vec2(STRIP_WIDTH, TOOL_HEIGHT * tools.len() as f32 + 8.0),
    );
    ui.painter()
        .rect_filled(strip, corner(PANEL_RADIUS), kit::window(dark));
    for (index, (tool, id, glyph, text)) in tools.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            strip.min + vec2(4.0, 4.0 + TOOL_HEIGHT * index as f32),
            vec2(STRIP_WIDTH - 8.0, TOOL_HEIGHT),
        );
        let label = catalog::text(state.preferences.locale, text);
        let response = widgets::tap(ui, id, cell, label, true);
        let selected = state.tool == Some(tool);
        if selected || response.hovered() {
            ui.painter().rect_filled(
                cell,
                corner(8.0),
                if selected {
                    kit::field(dark)
                } else {
                    kit::hover(dark)
                },
            );
        }
        let ink = if selected {
            kit::accent(dark)
        } else {
            kit::text(dark)
        };
        kit::tinted(glyph, 20.0, ink).paint_at(
            ui,
            Rect::from_center_size(pos2(cell.center().x, cell.top() + 18.0), Vec2::splat(20.0)),
        );
        ui.painter().text(
            pos2(cell.center().x, cell.top() + 39.0),
            Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(11.0),
            ink,
        );
        if response.clicked() {
            state.tool = if selected { None } else { Some(tool) };
        }
    }
    let Some(tool) = state.tool else { return };
    let origin = pos2(strip.left() - PANEL_GAP - PANEL_WIDTH, strip.top());
    let region = Rect::from_min_max(origin, pos2(origin.x + PANEL_WIDTH, panel.bottom() - 8.0));
    let mut close = false;
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("shorts.tool.panel")
            .max_rect(region),
        |ui| {
            egui::Frame::new()
                .fill(kit::window(dark))
                .corner_radius(PANEL_RADIUS as u8)
                .inner_margin(PANEL_PAD)
                .show(ui, |ui| {
                    ui.set_width(PANEL_WIDTH - 2.0 * PANEL_PAD);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let head = ui
                        .allocate_space(vec2(PANEL_WIDTH - 2.0 * PANEL_PAD, 24.0))
                        .1;
                    let title = match tool {
                        Tool::Captions => Text::Caption,
                        Tool::Trim => Text::ShortsTrim,
                        Tool::Layout => Text::ShortsLayout,
                    };
                    ui.painter().text(
                        head.left_center(),
                        Align2::LEFT_CENTER,
                        catalog::text(state.preferences.locale, title),
                        bold(16.0),
                        kit::text(dark),
                    );
                    let x = Rect::from_center_size(
                        head.right_center() - vec2(12.0, 0.0),
                        Vec2::splat(24.0),
                    );
                    close = icon_tap(
                        ui,
                        "shorts.tool.close",
                        x,
                        Icon::X,
                        catalog::text(state.preferences.locale, Text::ProjectsInfoClose),
                        true,
                        false,
                    )
                    .clicked();
                    ui.add_space(8.0);
                    match tool {
                        Tool::Captions => captions(state, ui),
                        Tool::Trim => trim(state, ui),
                        Tool::Layout => layout(state, ui),
                    }
                });
        },
    );
    if close {
        state.tool = None;
    }
}

fn captions(state: &mut State, ui: &mut egui::Ui) {
    let dark = state.preferences.dark;
    let row = ui.allocate_space(vec2(ui.available_width(), 40.0)).1;
    let label = state.tr(Text::ShortsCaptions);
    ui.painter().text(
        row.left_center(),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        kit::text(dark),
    );
    let switch = Rect::from_center_size(row.right_center() - vec2(18.0, 0.0), vec2(36.0, 16.0));
    let response = widgets::tap(ui, "shorts.captions", switch.expand(4.0), label, true);
    ui.painter().rect_filled(
        switch,
        corner(8.0),
        if state.captions {
            kit::accent(dark)
        } else {
            kit::border(dark)
        },
    );
    let knob = if state.captions {
        switch.right() - 8.0
    } else {
        switch.left() + 8.0
    };
    ui.painter().circle_filled(
        pos2(knob, switch.center().y),
        6.0,
        if state.captions {
            kit::on_accent(dark)
        } else {
            kit::text(dark)
        },
    );
    if response.clicked() {
        state.captions = !state.captions;
        state.preview_dirty = true;
    }
}

fn trim(state: &mut State, ui: &mut egui::Ui) {
    let segments = state
        .reviews
        .get(state.selected)
        .map(|r| r.segments.clone())
        .unwrap_or_default();
    let mut apply = None;
    for (index, segment) in segments.iter().enumerate() {
        let role = state.tr(match segment.role {
            ClipRole::Hook => Text::ShortsHook,
            ClipRole::Context => Text::ShortsContext,
            ClipRole::Payoff => Text::ShortsPayoff,
        });
        let in_label = state.tr(Text::ShortsIn);
        let out_label = state.tr(Text::ShortsOut);
        ui.label(role);
        let tip = state.tr(Text::ShortsApplyTrim);
        if let Some((start, end)) = state.trim_drafts.get_mut(index) {
            ui.horizontal(|ui| {
                for (value, id, label) in [
                    (start, format!("shorts.segment.{index}.in"), in_label),
                    (end, format!("shorts.segment.{index}.out"), out_label),
                ] {
                    let r = ui.add(
                        egui::TextEdit::singleline(value)
                            .char_limit(32)
                            .hint_text(label)
                            .desired_width(64.0),
                    );
                    widgets::named(&r, &id, label);
                }
                let slot = ui.allocate_space(vec2(32.0, 22.0)).1;
                let r = widgets::button(
                    ui,
                    &format!("shorts.segment.{index}.trim"),
                    slot,
                    Some(Icon::Scissors),
                    None,
                    tip,
                    widgets::Look::Field,
                    true,
                );
                if r.clicked() {
                    apply = Some(index);
                }
            });
        }
    }
    if let Some(index) = apply {
        let locale = state.preferences.locale;
        let result = state
            .trim_drafts
            .get(index)
            .ok_or(Error::Invalid("trim draft"))
            .and_then(|(s, e)| Ok((parse_seconds(locale, s)?, parse_seconds(locale, e)?)))
            .and_then(|(s, e)| state.trim(index, s, e));
        if let Err(e) = result {
            state.fail(e, Text::ShortsTrimError);
        }
    }
}

fn layout(state: &mut State, ui: &mut egui::Ui) {
    let Some(before) = state.reviews.get(state.selected).cloned() else {
        return;
    };
    let mut rect = before.crop;
    let aspect = state
        .source
        .as_ref()
        .and_then(|s| s.project.data().assets.iter().find(|a| a.id() == s.asset))
        .and_then(|a| a.probe().streams.iter().find_map(|s| s.video.as_ref()))
        .map(|v| v.width as f32 / v.height.max(1) as f32)
        .unwrap_or(16.0 / 9.0);
    let response = match crop::control(
        ui,
        "shorts.crop",
        &mut rect,
        aspect,
        state.tr(Text::ShortsCrop),
    ) {
        Ok(response) => response,
        Err(e) => {
            state.fail(e, Text::ShortsError);
            return;
        }
    };
    state.crop_focused = response.has_focus();
    if response.drag_started() {
        state.crop_entry = Some(before.clone());
        state.crop_cancelled = false;
    }
    if rect != before.crop && !state.crop_cancelled {
        if response.dragged() {
            if let Some(row) = state.reviews.get_mut(state.selected) {
                row.crop = rect;
            }
            state.preview_dirty = true;
        } else if let Err(e) = state.set_crop(rect) {
            state.fail(e, Text::ShortsError);
        }
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape))
        && let Some(entry) = state.crop_entry.take()
    {
        if let Some(row) = state.reviews.get_mut(state.selected) {
            *row = entry;
        }
        state.preview_dirty = true;
        state.crop_cancelled = true;
    }
    if response.drag_stopped()
        && let Some(entry) = state.crop_entry.take()
        && let Some(after) = state.reviews.get(state.selected).cloned()
    {
        if let Some(row) = state.reviews.get_mut(state.selected) {
            *row = entry;
        }
        if let Err(e) = state.commit(state.selected, after) {
            state.fail(e, Text::ShortsError);
        }
    }
    if response.drag_stopped() {
        state.crop_cancelled = false;
    }
    let slot = ui.allocate_space(vec2(32.0, 24.0)).1;
    let centre = widgets::button(
        ui,
        "shorts.crop.center",
        slot,
        Some(Icon::CornersOut),
        None,
        state.tr(Text::ShortsCenter),
        widgets::Look::Field,
        true,
    );
    if centre.clicked() {
        let mut centred = before.crop;
        centred.x = (1.0 - centred.width) / 2.0;
        centred.y = (1.0 - centred.height) / 2.0;
        if let Err(e) = state.set_crop(centred) {
            state.fail(e, Text::ShortsError);
        }
    }
}
