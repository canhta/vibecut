//! The review: the list of shorts, the selected one's preview and the tools beside it.
use super::{
    State, tools,
    widgets::{self, Look, bold, corner, icon_tap},
};
use crate::{
    catalog::{self, Text},
    egui::{self, Align2, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2},
    kit::{self, Icon},
    shorts::{Action, Context, Decision},
};
use time::{TICKS_PER_SECOND, Tick};

/// Measured at the 1718×989 window (reference 222002/222010), from the content origin.
const LIST: f32 = 360.0;
const CARD: [f32; 2] = [156.0, 277.0];
const COLUMN_X: (f32, f32) = (16.0, 188.0);
const PITCH: f32 = 362.0;
const LIST_TOP: f32 = 4.0;
const INSET: f32 = 8.0;
const PANEL_RADIUS: f32 = 12.0;
const HEADER_TOP: f32 = 12.0;
const BUTTON_HEIGHT: f32 = 40.0;
const EXPORT_WIDTH: f32 = 99.0;
const EDIT_WIDTH: f32 = 97.0;
const HEADER_PAD: f32 = 24.0;
const TOOL: f32 = 32.0;
const PREVIEW_TOP: f32 = 128.0;
const PREVIEW_BOTTOM: f32 = 55.0;
const TOOLS_RESERVE: f32 = 92.0;
const CONTROLS: f32 = 59.0;

pub(super) fn show(
    state: &mut State,
    cx: &Context,
    ui: &mut egui::Ui,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    let dark = state.preferences.dark;
    let list = Rect::from_min_size(rect.min, vec2(LIST, rect.height()));
    let left = if state.list_open {
        list.right() + INSET
    } else {
        rect.left() + INSET
    };
    let panel = Rect::from_min_max(
        pos2(left, rect.top() + INSET),
        pos2(rect.right() - INSET, rect.bottom() - INSET),
    );
    if state.list_open {
        list_ui(state, ui, list);
    }
    ui.painter()
        .rect_filled(panel, corner(PANEL_RADIUS), kit::panel(dark));
    header(state, cx, ui, panel, actions);
    if state.reviews.is_empty() {
        ui.painter().text(
            panel.center(),
            Align2::CENTER_CENTER,
            state.tr(Text::ShortsNoResults),
            egui::FontId::proportional(14.0),
            kit::secondary(dark),
        );
    } else {
        preview(state, ui, panel);
        tools::show(state, ui, panel);
    }
    handle(state, ui);
    // The handle that folds the list away sits on the panel's left edge.
    let tab = Rect::from_center_size(
        pos2(panel.left() + 11.0, panel.center().y),
        vec2(22.0, 76.0),
    );
    let response = widgets::tap(
        ui,
        "shorts.list.toggle",
        tab,
        state.tr(Text::ShortsList),
        true,
    );
    ui.painter().rect_filled(
        tab,
        egui::CornerRadius {
            nw: 0,
            sw: 0,
            ne: 8,
            se: 8,
        },
        kit::field(dark),
    );
    let flip = if state.list_open { 1.0 } else { -1.0 };
    let c = tab.center();
    ui.painter().add(egui::Shape::line(
        vec![
            c + vec2(-2.0 * flip, -5.0),
            c + vec2(2.0 * flip, 0.0),
            c + vec2(-2.0 * flip, 5.0),
        ],
        Stroke::new(1.6, kit::text(dark)),
    ));
    if response.clicked() {
        state.list_open = !state.list_open;
    }
}

fn list_ui(state: &mut State, ui: &mut egui::Ui, list: Rect) {
    let dark = state.preferences.dark;
    ui.painter().rect_filled(list, 0.0, kit::panel(dark));
    let mut picked = None;
    let count = state.reviews.len();
    let rows = count.div_ceil(2) as f32;
    ui.scope_builder(
        egui::UiBuilder::new().id_salt("shorts.list").max_rect(list),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("shorts.candidates")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (area, _) = ui.allocate_exact_size(
                        vec2(list.width(), LIST_TOP + rows * PITCH),
                        Sense::hover(),
                    );
                    let Some(source) = &state.source else { return };
                    for (index, (candidate, review)) in
                        source.candidates.iter().zip(&state.reviews).enumerate()
                    {
                        let origin = pos2(
                            area.left()
                                + if index % 2 == 0 {
                                    COLUMN_X.0
                                } else {
                                    COLUMN_X.1
                                },
                            area.top() + LIST_TOP + (index / 2) as f32 * PITCH,
                        );
                        let card = Rect::from_min_size(origin, vec2(CARD[0], CARD[1]));
                        let title = candidate.proposal.title.as_str();
                        let response = ui
                            .interact(
                                Rect::from_min_size(origin, vec2(CARD[0], PITCH - 20.0)),
                                egui::Id::unique(("shorts.candidate", index)),
                                Sense::click(),
                            )
                            .on_hover_text(candidate.proposal.reason.as_str());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::Button,
                                true,
                                index == state.selected,
                                title,
                            )
                        });
                        widgets::author(&response, &format!("shorts.candidate.{index}"));
                        if response.clicked() {
                            picked = Some(index);
                        }
                        card_ui(ui, card, title, review, index == state.selected, dark);
                    }
                });
        },
    );
    if let Some(index) = picked
        && let Err(e) = state.select(index)
    {
        state.fail(e, Text::ShortsError);
    }
}

fn card_ui(
    ui: &egui::Ui,
    card: Rect,
    title: &str,
    review: &crate::shorts::Review,
    selected: bool,
    dark: bool,
) {
    let painter = ui.painter();
    painter.rect_filled(card, corner(9.0), kit::field(dark));
    kit::tinted(Icon::FilmStrip, 24.0, kit::secondary(dark))
        .paint_at(ui, Rect::from_center_size(card.center(), Vec2::splat(24.0)));
    if selected {
        painter.rect_stroke(
            card.expand(4.0),
            corner(12.0),
            Stroke::new(2.0, kit::text(dark)),
            egui::StrokeKind::Inside,
        );
    }
    let badge = Rect::from_min_size(card.min + vec2(8.0, 8.0), vec2(24.0, 20.0));
    match review.decision {
        Decision::Accepted => {
            painter.rect_filled(badge, corner(4.0), kit::accent(dark));
            widgets::tick(painter, badge.center(), 10.0, kit::on_accent(dark));
        }
        Decision::Rejected => {
            painter.rect_filled(badge, corner(4.0), kit::scrim());
            kit::tinted(Icon::X, 12.0, kit::error_text(dark)).paint_at(
                ui,
                Rect::from_center_size(badge.center(), Vec2::splat(12.0)),
            );
        }
        Decision::Pending => {}
    }
    let total = review
        .segments
        .iter()
        .fold(Tick::ZERO, |sum, s| sum + s.range.duration());
    let pill = Rect::from_min_size(
        pos2(card.left() + 9.0, card.bottom() - 29.0),
        vec2(44.0, 20.0),
    );
    painter.rect_filled(pill, corner(4.0), kit::scrim());
    painter.text(
        pill.center(),
        Align2::CENTER_CENTER,
        clock(total),
        egui::FontId::proportional(12.0),
        kit::on_scrim(),
    );
    let mut job = egui::text::LayoutJob::single_section(
        title.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(14.0), kit::text(dark)),
    );
    job.wrap.max_width = card.width();
    job.wrap.max_rows = 3;
    job.wrap.break_anywhere = false;
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(card.left(), card.bottom() + 10.0),
        galley,
        kit::text(dark),
    );
}

fn header(
    state: &mut State,
    cx: &Context,
    ui: &mut egui::Ui,
    panel: Rect,
    actions: &mut Vec<Action>,
) {
    let dark = state.preferences.dark;
    let top = panel.top() + HEADER_TOP;
    let busy = state.busy();
    let accepted = state
        .reviews
        .iter()
        .any(|r| r.decision == Decision::Accepted);
    let export = Rect::from_min_size(
        pos2(panel.right() - HEADER_PAD - EXPORT_WIDTH, top),
        vec2(EXPORT_WIDTH, BUTTON_HEIGHT),
    );
    let edit = Rect::from_min_size(
        pos2(export.left() - 8.0 - EDIT_WIDTH, top),
        vec2(EDIT_WIDTH, BUTTON_HEIGHT),
    );
    if widgets::button(
        ui,
        "shorts.export",
        export,
        Some(Icon::Export),
        Some(state.tr(Text::Export)),
        state.tr(Text::ShortsExport),
        Look::Primary,
        accepted && !busy,
    )
    .clicked()
    {
        state.pick(ui.ctx(), super::super::state::Pick::Export);
    }
    if widgets::button(
        ui,
        "shorts.edit",
        edit,
        None,
        Some(state.tr(Text::ShortsEdit)),
        state.tr(Text::ShortsEdit),
        Look::Field,
        !state.reviews.is_empty() && !busy && cx.editor_available,
    )
    .clicked()
    {
        actions.push(Action::OpenEditor);
    }
    // Small actions: accept, reject, undo, redo, language.
    let gap = 4.0;
    let back = Rect::from_min_size(
        pos2(edit.left() - 16.0 - 6.0 * TOOL - 5.0 * gap, top + 4.0),
        Vec2::splat(TOOL),
    );
    let mut x = back.right() + gap;
    let at = |x: f32| Rect::from_min_size(pos2(x, top + 4.0), Vec2::splat(TOOL));
    let current = state.reviews.get(state.selected).map(|r| r.decision);
    let accept = at(x);
    if icon_tap_plain(
        ui,
        "shorts.accept",
        accept,
        state.tr(Text::ShortsAccept),
        current == Some(Decision::Accepted),
    )
    .clicked()
        && let Err(e) = state.decide(Decision::Accepted)
    {
        state.fail(e, Text::ShortsError);
    }
    widgets::tick(ui.painter(), accept.center(), 14.0, kit::text(dark));
    x += TOOL + gap;
    if icon_tap(
        ui,
        "shorts.reject",
        at(x),
        Icon::X,
        state.tr(Text::ShortsReject),
        true,
        current == Some(Decision::Rejected),
    )
    .clicked()
        && let Err(e) = state.decide(Decision::Rejected)
    {
        state.fail(e, Text::ShortsError);
    }
    x += TOOL + gap;
    if icon_tap(
        ui,
        "shorts.undo",
        at(x),
        Icon::ArrowCounterClockwise,
        state.tr(Text::Undo),
        !state.undo.is_empty(),
        false,
    )
    .clicked()
    {
        state.undo();
    }
    x += TOOL + gap;
    if icon_tap(
        ui,
        "shorts.redo",
        at(x),
        Icon::ArrowClockwise,
        state.tr(Text::Redo),
        !state.redo.is_empty(),
        false,
    )
    .clicked()
    {
        state.redo();
    }
    x += TOOL + gap;
    if icon_tap(
        ui,
        "shorts.locale",
        at(x),
        Icon::Translate,
        state.tr(Text::Language),
        true,
        false,
    )
    .clicked()
    {
        state.preferences.locale = if state.preferences.locale == catalog::Locale::En {
            catalog::Locale::Vi
        } else {
            catalog::Locale::En
        };
    }
    // Back to the pick screen sits first in the group; the title takes the left.
    if icon_tap(
        ui,
        "shorts.setup",
        back,
        Icon::ArrowLeft,
        state.tr(Text::ShortsBack),
        !busy,
        false,
    )
    .clicked()
        && let Err(e) = state.pick_video(state.request.path.clone())
    {
        state.fail(e, Text::ShortsError);
    }
    if let Some(candidate) = state
        .source
        .as_ref()
        .and_then(|s| s.candidates.get(state.selected))
    {
        let mut job = egui::text::LayoutJob::single_section(
            candidate.proposal.title.as_str().to_owned(),
            egui::TextFormat::simple(egui::FontId::proportional(14.0), kit::text(dark)),
        );
        job.wrap.max_width = (back.left() - 16.0 - (panel.left() + HEADER_PAD)).max(40.0);
        job.wrap.max_rows = 1;
        let galley = ui.painter().layout_job(job);
        let y = top + BUTTON_HEIGHT / 2.0 - galley.size().y / 2.0;
        ui.painter()
            .galley(pos2(panel.left() + HEADER_PAD, y), galley, kit::text(dark));
    }
}

/// A 32 px control that paints its own glyph on top.
fn icon_tap_plain(
    ui: &mut egui::Ui,
    id: &str,
    rect: Rect,
    tip: &str,
    selected: bool,
) -> egui::Response {
    widgets::button(
        ui,
        id,
        rect,
        None,
        None,
        tip,
        if selected { Look::Field } else { Look::Quiet },
        true,
    )
}

/// Only the 9:16 frame and its controls; the panel's right side keeps the tools.
fn preview(state: &mut State, ui: &mut egui::Ui, panel: Rect) {
    let dark = state.preferences.dark;
    let area_right = panel.right() - TOOLS_RESERVE;
    let available = (panel.bottom() - PREVIEW_BOTTOM - (panel.top() + PREVIEW_TOP)).max(160.0);
    let video_h = (available - CONTROLS).clamp(120.0, 731.0);
    let video_w = video_h * 9.0 / 16.0;
    let centre = (panel.left() + area_right) / 2.0;
    let video = Rect::from_min_size(
        pos2(centre - video_w / 2.0, panel.top() + PREVIEW_TOP),
        vec2(video_w, video_h),
    );
    let bar = Rect::from_min_size(video.left_bottom(), vec2(video_w, CONTROLS));
    let painter = ui.painter().clone();
    painter.rect_filled(video.union(bar), 0.0, kit::preview_backdrop());

    let Some(editor) = &mut state.preview else {
        egui::Spinner::new().paint_at(
            ui,
            Rect::from_center_size(video.center(), Vec2::splat(24.0)),
        );
        return;
    };
    let mut seek = None;
    let Ok(position) = editor.playback.position() else {
        return;
    };
    let duration = editor.session.project().duration();
    let playing = editor.playback.playing();
    if let Some(frame) = editor.playback.frame() {
        let key = (frame.epoch, frame.at);
        match &mut state.frame {
            Some(texture) if texture.key == key => {}
            Some(texture) => {
                texture
                    .handle
                    .set(frame.image.clone(), egui::TextureOptions::LINEAR);
                texture.key = key;
            }
            None => {
                state.frame = Some(super::FrameTexture {
                    handle: ui.ctx().load_texture(
                        "shorts.frame",
                        frame.image.clone(),
                        egui::TextureOptions::LINEAR,
                    ),
                    key,
                });
            }
        }
        if let Some(texture) = &state.frame {
            let [w, h] = frame.image.size;
            let scale = (video_w / w.max(1) as f32).min(video_h / h.max(1) as f32);
            let shown =
                Rect::from_center_size(video.center(), vec2(w as f32 * scale, h as f32 * scale));
            painter.with_clip_rect(video).image(
                texture.handle.id(),
                shown,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                kit::image_tint(),
            );
        }
    } else {
        egui::Spinner::new().paint_at(
            ui,
            Rect::from_center_size(video.center(), Vec2::splat(24.0)),
        );
    }

    // Play, time and a scrubbing line, as in the reference's bare player.
    let play = Rect::from_center_size(
        pos2(video.left() + 23.0, bar.top() + 21.0),
        Vec2::splat(28.0),
    );
    let (glyph, tip, id) = if playing {
        (Icon::Pause, Text::Pause, "transport.pause")
    } else {
        (Icon::Play, Text::Play, "transport.play")
    };
    let tap = widgets::tap(
        ui,
        "transport.play",
        play,
        catalog::text(state.preferences.locale, tip),
        true,
    );
    kit::tinted(glyph, 16.0, kit::on_scrim())
        .paint_at(ui, Rect::from_center_size(play.center(), Vec2::splat(16.0)));
    if tap.clicked() {
        let _ = editor.activate(id);
    }
    let now = painter.text(
        pos2(video.left() + 45.0, bar.top() + 21.0),
        Align2::LEFT_CENTER,
        clock(position),
        bold(14.0),
        kit::on_scrim(),
    );
    painter.text(
        pos2(now.right() + 8.0, bar.top() + 21.0),
        Align2::LEFT_CENTER,
        clock(duration),
        egui::FontId::proportional(14.0),
        kit::secondary(dark),
    );
    let track = Rect::from_min_size(
        pos2(video.left() + 14.0, bar.top() + 40.0),
        vec2(video_w - 28.0, 3.0),
    );
    painter.rect_filled(track, corner(1.5), kit::border(dark));
    let share = if duration.0 > 0 {
        (position.0 as f32 / duration.0 as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    painter.rect_filled(
        Rect::from_min_size(track.min, vec2(track.width() * share, track.height())),
        corner(1.5),
        kit::accent(dark),
    );
    let scrub = widgets::tap(
        ui,
        "shorts.scrub",
        track.expand2(vec2(0.0, 8.0)),
        catalog::text(state.preferences.locale, Text::ShortsSeek),
        duration.0 > 0,
    );
    if let Some(pointer) = scrub.interact_pointer_pos()
        && (scrub.clicked() || scrub.dragged())
    {
        let at = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
        seek = Some(Tick((duration.0 as f64 * f64::from(at)) as i64));
    }
    if let Some(at) = seek {
        let project = editor.session.project().clone();
        let _ = editor.playback.seek(at, playing, project);
    }
}

/// Keyboard: undo/redo and the transport keys when nothing is being typed.
fn handle(state: &mut State, ui: &mut egui::Ui) {
    if ui.ctx().text_edit_focused() || state.crop_entry.is_some() || state.reviews.is_empty() {
        return;
    }
    let redo = ui.input_mut(|i| {
        i.consume_key(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::Z,
        )
    });
    let undo = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
    if redo {
        state.redo();
    } else if undo {
        state.undo();
    }
    let frame_keys = [
        (egui::Key::ArrowLeft, "transport.frame_back"),
        (egui::Key::ArrowRight, "transport.frame_forward"),
    ];
    let keys = [
        (egui::Key::Space, "transport.play"),
        (egui::Key::K, "transport.pause"),
        (egui::Key::J, "transport.back"),
        (egui::Key::L, "transport.forward"),
        (egui::Key::Home, "transport.start"),
        (egui::Key::End, "transport.end"),
    ];
    let steps: Vec<_> = if state.crop_focused {
        keys.to_vec()
    } else {
        keys.into_iter().chain(frame_keys).collect()
    };
    for (key, id) in steps {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key))
            && let Some(preview) = &mut state.preview
            && let Err(e) = preview.activate(id)
        {
            state.fail(e, Text::ShortsError);
        }
    }
}

fn clock(tick: Tick) -> String {
    let seconds = (tick.0.max(0) / TICKS_PER_SECOND) as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}
