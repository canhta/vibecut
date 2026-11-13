//! The Recent row of the entry screen: earlier runs of this machine, and the run in progress.
use super::{
    State,
    progress::GAP,
    widgets::{self, Look, bold, corner},
};
use crate::{
    egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2},
    i18n::{self, Text},
    kit::{self, Icon},
};
use project::Thumbnail;

/// Measured at the 1718×989 window (reference 221941, the `My projects` row), from the content
/// origin; the row starts at the drop card's left edge.
const HEADING_Y: f32 = 691.0;
const THUMB_TOP: f32 = 718.0;
const THUMB: [f32; 2] = [217.0, 122.0];
const THUMB_RADIUS: f32 = 8.0;
const CHIP_INSET: f32 = 13.0;
const CHIP_FROM_BOTTOM: f32 = 20.0;
const NAME_BELOW: f32 = 16.0;
/// Unmeasured (the reference shows one card): the gap is the results grid's, and a second row
/// follows the first at the thumbnail, the name line and one gap.
const ROW_PITCH: f32 = THUMB[1] + NAME_BELOW + 10.0 + GAP;
/// Ours: the reference has no way to remove an entry.
const REMOVE: f32 = 24.0;

enum Card {
    /// The run in progress, first in the row.
    Active,
    Run(usize),
}

pub(super) fn show(state: &mut State, ui: &mut egui::Ui, rect: egui::Rect, left: f32, width: f32) {
    let active = state.generating.is_some();
    if state.runs.entries.is_empty() && !active {
        return;
    }
    let dark = state.preferences.dark;
    ui.painter().text(
        pos2(left, rect.top() + HEADING_Y),
        Align2::LEFT_CENTER,
        state.tr(Text::ShortsRecent),
        bold(18.0),
        kit::text(dark),
    );
    let cards: Vec<Card> = active
        .then_some(Card::Active)
        .into_iter()
        .chain((0..state.runs.entries.len()).map(Card::Run))
        .collect();
    let columns = (((width + GAP) / (THUMB[0] + GAP)).floor() as usize).max(1);
    let region = Rect::from_min_max(
        pos2(left, rect.top() + THUMB_TOP),
        pos2(left + width, rect.bottom()),
    );
    let rows = cards.len().div_ceil(columns) as f32;
    let mut open = None;
    let mut remove = None;
    let mut resume = false;
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("shorts.recent")
            .max_rect(region),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("shorts.recent.scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (area, _) = ui.allocate_exact_size(
                        vec2(region.width(), rows * ROW_PITCH),
                        Sense::hover(),
                    );
                    for (index, card) in cards.iter().enumerate() {
                        let origin = pos2(
                            area.left() + (THUMB[0] + GAP) * (index % columns) as f32,
                            area.top() + ROW_PITCH * (index / columns) as f32,
                        );
                        let thumb = Rect::from_min_size(origin, vec2(THUMB[0], THUMB[1]));
                        let (name, count, id) = match card {
                            Card::Active => (
                                state.tr(Text::ShortsCreating).to_owned(),
                                u64::from(state.generation_count().unwrap_or(0)),
                                None,
                            ),
                            Card::Run(at) => {
                                let Some(entry) = state.runs.entries.get(*at) else {
                                    continue;
                                };
                                (entry.source_name.clone(), entry.clips as u64, Some(entry.id))
                            }
                        };
                        let slot = format!("shorts.run.{index}");
                        let response = ui.interact(
                            thumb.expand2(vec2(0.0, NAME_BELOW + 12.0)),
                            egui::Id::new(("shorts.run", index)),
                            Sense::click(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, &name)
                        });
                        widgets::author(&response, &slot);
                        paint_thumb(state, ui, thumb, id, dark);
                        let chip = i18n::format(
                            state.preferences.locale,
                            Text::ShortsCountLabel,
                            &[("count", &i18n::number(state.preferences.locale, count))],
                        );
                        ui.painter().text(
                            pos2(
                                thumb.left() + CHIP_INSET,
                                thumb.bottom() - CHIP_FROM_BOTTOM,
                            ),
                            Align2::LEFT_CENTER,
                            chip,
                            egui::FontId::proportional(12.0),
                            Color32::WHITE,
                        );
                        paint_name(ui, thumb, &name, dark);
                        if let Some(id) = id {
                            let tap = Rect::from_min_size(
                                pos2(thumb.right() - REMOVE - 4.0, thumb.top() + 4.0),
                                vec2(REMOVE, REMOVE),
                            );
                            if widgets::button(
                                ui,
                                &format!("shorts.run.remove.{index}"),
                                tap,
                                Some(Icon::X),
                                None,
                                state.tr(Text::ShortsRemoveRun),
                                Look::Quiet,
                                true,
                            )
                            .clicked()
                            {
                                remove = Some(id);
                                continue;
                            }
                        }
                        if response.clicked() {
                            match id {
                                Some(id) => open = Some(id),
                                None => resume = true,
                            }
                        }
                    }
                });
        },
    );
    if resume {
        state.return_to_run();
    }
    if let Some(id) = remove {
        state.remove_run(id);
    }
    if let Some(id) = open
        && let Err(e) = state.open_run(id)
    {
        state.fail(e, Text::ShortsError);
    }
}

/// The poster of a run, uploaded on first paint; a run without one and the run in progress
/// show the plain card.
fn paint_thumb(
    state: &mut State,
    ui: &mut egui::Ui,
    thumb: Rect,
    id: Option<project::ProjectId>,
    dark: bool,
) {
    let painter = ui.painter();
    painter.rect_filled(thumb, corner(THUMB_RADIUS), Color32::BLACK);
    let texture = id.and_then(|id| {
        if !state.runs.textures.contains_key(&id) {
            let entry = state.runs.entries.iter().find(|entry| entry.id == id)?;
            let Thumbnail::Ready {
                width,
                height,
                rgba,
            } = &entry.poster
            else {
                return None;
            };
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [*width as usize, *height as usize],
                rgba,
            );
            let texture = ui.ctx().load_texture(
                format!("shorts.run.poster.{}", id.0),
                image,
                egui::TextureOptions::LINEAR,
            );
            state.runs.textures.insert(id, texture);
        }
        state
            .runs
            .textures
            .get(&id)
            .map(|texture| (texture.id(), texture.size_vec2()))
    });
    let painter = ui.painter();
    match texture {
        Some((texture, size)) => {
            // Contain: the whole frame, centred on the black card.
            let scale = (thumb.width() / size.x).min(thumb.height() / size.y);
            let shown = Rect::from_center_size(thumb.center(), size * scale);
            painter.image(
                texture,
                shown,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None if id.is_none() => {
            painter.rect_filled(thumb, corner(THUMB_RADIUS), kit::field(dark));
        }
        None => {}
    }
    painter.rect_stroke(
        thumb,
        corner(THUMB_RADIUS),
        Stroke::new(1.0, kit::border(dark)),
        egui::StrokeKind::Inside,
    );
}

/// The source name on one line below the card, cut with an ellipsis.
fn paint_name(ui: &egui::Ui, thumb: Rect, name: &str, dark: bool) {
    let mut job = egui::text::LayoutJob::simple_singleline(
        name.to_owned(),
        egui::FontId::proportional(14.0),
        kit::text(dark),
    );
    job.wrap.max_width = thumb.width();
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    let top = thumb.bottom() + NAME_BELOW - galley.size().y / 2.0;
    ui.painter()
        .galley(pos2(thumb.left(), top), galley, kit::text(dark));
}
