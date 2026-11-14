//! Renders the player panel from read-only editor context.
use super::{
    Action, Context, State,
    overlay::{GRID_MAX, GRID_MIN, Guide, Overlay, OverlayVisibility, PLANES, custom_from_draft},
    state::{GridDraft, TimecodeEdit},
};
use crate::{
    actions::{BookmarkDirection, bookmark_target},
    egui::{self, Align2, FontId, Rect, Sense, Vec2},
    i18n::Text,
    kit,
    services::preview,
};
use time::Tick;

pub(super) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    let dark = cx.dark;
    let panel = ui.max_rect();
    let empty = cx.session.project().duration() == Tick::ZERO;
    ui.painter()
        .rect_filled(panel, kit::radius::FRAME_PANEL, kit::panel(dark));
    let header = Rect::from_min_size(panel.min, egui::vec2(panel.width(), kit::size::TAB_STRIP));
    ui.painter().rect_filled(
        header,
        egui::CornerRadius {
            nw: kit::radius::FRAME_PANEL,
            ne: kit::radius::FRAME_PANEL,
            sw: 0,
            se: 0,
        },
        kit::strip(dark),
    );
    ui.painter().hline(
        header.x_range(),
        header.bottom() - kit::space::HAIRLINE / 2.0,
        egui::Stroke::new(kit::space::HAIRLINE, kit::window(dark)),
    );
    ui.painter().circle_filled(
        egui::pos2(
            header.right() - kit::space::XS,
            header.top() + kit::space::MD,
        ),
        kit::space::ICON_LABEL / 2.0,
        kit::accent(dark),
    );
    let title = format!(
        "{}-{}",
        cx.tr(Text::Preview),
        cx.session.project().data().scene().name.as_str()
    );
    ui.painter()
        .with_clip_rect(Rect::from_min_max(
            header.min,
            egui::pos2(
                (header.right() - kit::size::CONTROL * 2.0).max(header.left()),
                header.bottom(),
            ),
        ))
        .text(
            egui::pos2(header.left() + kit::space::LG, header.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::new(kit::type_size::PLAYER_TITLE, kit::label_font().family),
            kit::text(dark),
        );
    let menu_rect = Rect::from_center_size(
        egui::pos2(header.right() - kit::size::CONTROL, header.center().y),
        Vec2::splat(kit::size::CONTROL),
    );
    at(ui, menu_rect, "player.menu", |ui| {
        let response = kit::icon_button(
            ui,
            "preview.menu",
            kit::Icon::ListBullets,
            cx.tr(Text::Preview),
            false,
        );
        // Commands close the menu themselves; the guide picker and its grid-size
        // fields stay open while they are edited or dragged.
        let shown = egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                menu(
                    ui,
                    cx,
                    &mut actions,
                    "preview.snapshot",
                    Text::SnapshotSave,
                    !empty,
                    "preview.snapshot",
                    None,
                );
                menu(
                    ui,
                    cx,
                    &mut actions,
                    "preview.copy",
                    Text::SnapshotCopy,
                    !empty,
                    "preview.copy_snapshot",
                    None,
                );
                ui.separator();
                menu(
                    ui,
                    cx,
                    &mut actions,
                    "transport.back",
                    Text::Back,
                    !empty,
                    "transport.back",
                    None,
                );
                menu(
                    ui,
                    cx,
                    &mut actions,
                    "transport.forward",
                    Text::Forward,
                    !empty,
                    "transport.forward",
                    None,
                );
                ui.separator();
                guide_menu(ui, cx, state, &mut actions);
            });
        if shown.is_none() {
            // Closed mid-drag: the drafts would otherwise reopen as stale values.
            state.grid_draft = None;
            state.margin_draft = None;
        }
        remember(cx, &mut actions, "preview.menu", response);
    });
    let footer = Rect::from_min_max(
        egui::pos2(
            panel.left(),
            (panel.bottom() - kit::size::PLAYER_FOOTER).max(header.bottom()),
        ),
        panel.max,
    );
    let stage = Rect::from_min_max(
        egui::pos2(panel.left() + kit::space::MD, header.bottom()),
        egui::pos2(
            panel.right() - kit::space::MD - kit::space::HAIRLINE,
            (footer.top() - kit::size::PLAYER_FIT_ROW).max(header.bottom()),
        ),
    );
    let fit_scale = preview::fit_scale(stage.size(), canvas_size(cx));
    let size = egui::vec2(canvas_size(cx)[0] as f32, canvas_size(cx)[1] as f32) * fit_scale;
    let frame = Rect::from_center_size(stage.center(), size);
    state.viewport = frame.size().max(Vec2::ZERO);
    canvas(state, cx, ui, frame, empty, &mut actions);
    if !empty {
        ui.painter().add(
            egui::epaint::RectShape::stroke(
                frame,
                kit::radius::PANEL,
                egui::Stroke::new(kit::space::HAIRLINE, kit::text(dark)),
                egui::StrokeKind::Middle,
            )
            .with_round_to_pixels(false),
        );
        let fit = Rect::from_center_size(
            egui::pos2(
                stage.center().x,
                (frame.bottom() + kit::size::PLAYER_FIT_OFFSET).min(footer.top() - kit::space::MD),
            ),
            Vec2::splat(kit::size::CONTROL),
        );
        at(ui, fit, "player.fit", |ui| {
            let r = kit::transport_button(
                ui,
                "preview.zoom.fit",
                kit::Icon::ArrowClockwise,
                cx.tr(Text::ZoomFit),
                true,
            );
            if r.clicked() {
                actions.push(Action::Command("preview.zoom.fit".into()));
            }
            remember(cx, &mut actions, "preview.zoom.fit", r);
        });
    }
    footer_controls(state, cx, ui, footer, empty, &mut actions);
    actions
}
fn at(ui: &mut egui::Ui, rect: Rect, id: &str, show: impl FnOnce(&mut egui::Ui)) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .scope_id(ui.make_persistent_id(id))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect());
    show(&mut child);
}
fn control(id: &str, playing: bool) -> Option<(kit::Icon, Text)> {
    Some(match id {
        "transport.back" => (kit::Icon::SkipBack, Text::Back),
        "transport.forward" => (kit::Icon::SkipForward, Text::Forward),
        "transport.play" => (
            if playing {
                kit::Icon::Pause
            } else {
                kit::Icon::Play
            },
            if playing { Text::Pause } else { Text::Play },
        ),
        "preview.snapshot" | "preview.menu.snapshot" => (kit::Icon::FloppyDisk, Text::SnapshotSave),
        "preview.copy" | "preview.menu.copy" => (kit::Icon::File, Text::SnapshotCopy),
        "preview.retry" => (kit::Icon::ArrowClockwise, Text::Retry),
        "preview.menu" => (kit::Icon::ListBullets, Text::Preview),
        "preview.audio_status" => (kit::Icon::SpeakerSlash, Text::Offline),
        "preview.fullscreen" | "preview.menu.fullscreen" => {
            (kit::Icon::CornersOut, Text::Fullscreen)
        }
        "preview.zoom.fit" => (kit::Icon::ArrowClockwise, Text::ZoomFit),
        "playback.loop" => (kit::Icon::Repeat, Text::ActionPlaybackLoop),
        "preview.volume.mute" => (kit::Icon::SpeakerHigh, Text::AudioMuted),
        "preview.zoom.in" => (kit::Icon::MagnifyingGlassPlus, Text::ZoomIn),
        "preview.zoom.out" => (kit::Icon::MagnifyingGlassMinus, Text::ZoomOut),
        "preview.scenes" => (kit::Icon::FilmStrip, Text::Scenes),
        _ => return None,
    })
}
fn remember(cx: &Context<'_>, actions: &mut Vec<Action>, id: &str, response: egui::Response) {
    let metadata = control(id, cx.playback.playing());
    let tooltip = metadata.map(|(_, text)| {
        let label = cx.tr(text);
        let command = if id == "transport.play" && cx.playback.playing() {
            "transport.pause"
        } else {
            id
        };
        let chord = crate::actions::action(command)
            .and_then(|spec| cx.keybindings.effective_chords(spec).first().cloned())
            .and_then(|text| crate::actions::Chord::parse(&text).ok())
            .map(|chord| chord.display(crate::actions::Platform::current()));
        chord.map_or_else(
            || label.to_owned(),
            |shortcut| {
                crate::i18n::format(
                    cx.locale,
                    Text::ActionShortcut,
                    &[("action", label), ("shortcut", &shortcut)],
                )
            },
        )
    });
    let response = if let Some(tooltip) = &tooltip {
        response.on_hover_text(tooltip)
    } else {
        response
    };
    actions.push(Action::Remember {
        id: id.into(),
        icon: metadata
            .map(|(icon, _)| icon.name())
            .or((id == "preview.zoom").then_some(kit::Icon::MagnifyingGlassPlus.name())),
        tooltip,
        response,
    });
}

#[allow(clippy::too_many_arguments)]
fn menu(
    ui: &mut egui::Ui,
    cx: &Context<'_>,
    actions: &mut Vec<Action>,
    id: &str,
    text: Text,
    enabled: bool,
    command: &str,
    shortcut: Option<String>,
) {
    let button = if let Some((icon, _)) = control(id, cx.playback.playing()) {
        egui::Button::image_and_text(
            kit::tinted(icon, kit::icon::DENSE, kit::text(cx.dark)),
            cx.tr(text),
        )
    } else {
        egui::Button::new(cx.tr(text))
    };
    let shortcut = shortcut.or_else(|| cx.shortcut(command));
    let button = match shortcut {
        Some(shortcut) => button.shortcut_text(shortcut),
        None => button,
    };
    let r = ui.add_enabled(enabled, button);
    if r.clicked() {
        actions.push(Action::Command(command.into()));
        ui.close();
    }
    remember(cx, actions, id, r);
}
/// The guide picker, its grid size and the guide switch in the player menu.
fn guide_menu(ui: &mut egui::Ui, cx: &Context<'_>, state: &mut State, actions: &mut Vec<Action>) {
    let visibility = cx.preferences.preview_overlays;
    let mut guide = visibility.guide;
    let response = ui.checkbox(&mut guide, cx.tr(Text::PreviewGuideShow));
    if response.changed() {
        actions.push(Action::SetPreference {
            key: "preview_overlays",
            value: serde_json::json!(OverlayVisibility {
                guide,
                ..visibility
            }),
        });
    }
    remember(cx, actions, "preview.guide.show", response);
    let current = cx.preferences.preview_guide;
    for (id, choice, text) in [
        ("preview.guide.none", Guide::None, Text::GuideNone),
        ("preview.guide.grid", Guide::default(), Text::GuideGrid),
        ("preview.guide.tiktok", Guide::TikTok, Text::GuideTikTok),
        (
            "preview.guide.instagram_reels",
            Guide::InstagramReels,
            Text::GuideInstagramReels,
        ),
        (
            "preview.guide.youtube_shorts",
            Guide::YouTubeShorts,
            Text::GuideYouTubeShorts,
        ),
        // The margins start at zero, which shades nothing until the user sets them.
        (
            "preview.guide.custom",
            Guide::Custom {
                top: 0,
                bottom: 0,
                left: 0,
                right: 0,
            },
            Text::GuideCustom,
        ),
    ] {
        let selected = current.same_choice(choice);
        let response = ui.selectable_label(selected, cx.tr(text));
        if response.clicked() && !selected {
            actions.push(Action::SetPreference {
                key: "preview_guide",
                value: serde_json::json!(choice),
            });
        }
        remember(cx, actions, id, response);
    }
    if let Guide::Grid { rows, cols } = current {
        // The gesture rule of the properties scrubs: a drag or typed edit drafts
        // the sizes, and one preference edit publishes the final value on release
        // or when the field is left. Unchanged values record no history step.
        let mut draft = state.grid_draft.unwrap_or(GridDraft { rows, cols });
        let (rows_response, cols_response) = ui
            .horizontal(|ui| {
                ui.label(cx.tr(Text::GuideRows));
                let rows_response =
                    ui.add(egui::DragValue::new(&mut draft.rows).range(GRID_MIN..=GRID_MAX));
                ui.label(cx.tr(Text::GuideCols));
                let cols_response =
                    ui.add(egui::DragValue::new(&mut draft.cols).range(GRID_MIN..=GRID_MAX));
                (rows_response, cols_response)
            })
            .inner;
        let ended = rows_response.drag_stopped()
            || cols_response.drag_stopped()
            || rows_response.lost_focus()
            || cols_response.lost_focus();
        // Escape cancels the draft like a scrub's Escape; it also closes the menu, and the
        // focus it takes away must not publish the value being dragged.
        let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if ended || cancelled {
            state.grid_draft = None;
            if !cancelled && draft != (GridDraft { rows, cols }) {
                actions.push(Action::SetPreference {
                    key: "preview_guide",
                    value: serde_json::json!(Guide::Grid {
                        rows: draft.rows,
                        cols: draft.cols,
                    }),
                });
            }
        } else if rows_response.changed() || cols_response.changed() || state.grid_draft.is_some() {
            state.grid_draft = Some(draft);
        }
        remember(cx, actions, "preview.guide.rows", rows_response);
        remember(cx, actions, "preview.guide.cols", cols_response);
    }
    if let Guide::Custom {
        top,
        bottom,
        left,
        right,
    } = current
    {
        custom_margins(cx, ui, actions, state, current, [top, bottom, left, right]);
    } else {
        // A draft left by a kept invalid value must not reappear on a later pick.
        state.margin_draft = None;
    }
}

/// The custom guide's four margin fields. They follow the grid-size rule: a drag or
/// typed edit drafts the values, and one preference edit publishes the valid final
/// value when the gesture ends. An invalid draft stays on screen with its error.
fn custom_margins(
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    state: &mut State,
    current: Guide,
    margins: [u8; 4],
) {
    let saved = margins.map(i32::from);
    let mut draft = state.margin_draft.unwrap_or(saved);
    let fields = [
        ("preview.guide.margin.top", Text::GuideTop),
        ("preview.guide.margin.bottom", Text::GuideBottom),
        ("preview.guide.margin.left", Text::GuideLeft),
        ("preview.guide.margin.right", Text::GuideRight),
    ];
    let responses = ui
        .horizontal_wrapped(|ui| {
            let mut responses = Vec::with_capacity(fields.len());
            for (value, (_, text)) in draft.iter_mut().zip(fields) {
                ui.label(cx.tr(text));
                responses.push(ui.add(egui::DragValue::new(value).suffix("%")));
            }
            responses
        })
        .inner;
    // Typed values outside the model's range stay visible with this error until fixed.
    if custom_from_draft(draft).is_none() {
        ui.colored_label(kit::error_text(cx.dark), cx.tr(Text::GuideMarginsInvalid));
    }
    let ended = responses
        .iter()
        .any(|response| response.drag_stopped() || response.lost_focus());
    let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if cancelled {
        state.margin_draft = None;
    } else if ended {
        state.margin_draft = None;
        match custom_from_draft(draft) {
            Some(guide) if guide != current => actions.push(Action::SetPreference {
                key: "preview_guide",
                value: serde_json::json!(guide),
            }),
            Some(_) => {}
            None => state.margin_draft = Some(draft),
        }
    } else if responses.iter().any(egui::Response::changed) || state.margin_draft.is_some() {
        state.margin_draft = Some(draft);
    }
    for ((id, _), response) in fields.into_iter().zip(responses) {
        remember(cx, actions, id, response);
    }
}

/// Paints the visible overlays plane by plane over the frame image, so the
/// transform handles drawn afterwards stay above all of them.
fn overlays(cx: &Context<'_>, ui: &mut egui::Ui, stage: Rect, displayed: Rect) {
    for plane in PLANES {
        for overlay in [Overlay::Guide, Overlay::Notes] {
            if overlay.plane() != plane || !cx.preferences.preview_overlays.shows(overlay) {
                continue;
            }
            match overlay {
                Overlay::Guide => guide(cx, ui, stage, displayed),
                Overlay::Notes => notes(cx, ui, displayed),
            }
        }
    }
}

/// Draws the selected guide (grid lines or platform regions) over the displayed frame, clipped to the stage.
fn guide(cx: &Context<'_>, ui: &egui::Ui, stage: Rect, displayed: Rect) {
    let painter = ui.painter().with_clip_rect(stage);
    let stroke = egui::Stroke::new(kit::space::HAIRLINE, kit::text(cx.dark).gamma_multiply(0.5));
    let guide = cx.preferences.preview_guide;
    let (vertical, horizontal) = guide.grid_lines();
    for fraction in vertical {
        painter.vline(
            displayed.left() + displayed.width() * fraction,
            displayed.y_range(),
            stroke,
        );
    }
    for fraction in horizontal {
        painter.hline(
            displayed.x_range(),
            displayed.top() + displayed.height() * fraction,
            stroke,
        );
    }
    // A platform guide shades the regions its UI covers.
    let fill = kit::text(cx.dark).gamma_multiply(0.18);
    for [left, top, right, bottom] in guide.covered_regions() {
        let region = Rect::from_min_max(
            egui::pos2(
                displayed.left() + displayed.width() * left,
                displayed.top() + displayed.height() * top,
            ),
            egui::pos2(
                displayed.left() + displayed.width() * right,
                displayed.top() + displayed.height() * bottom,
            ),
        );
        painter.rect_filled(region, 0.0, fill);
    }
}

/// Lists the bookmark notes active at the playhead, top-left over the frame.
/// A bookmark is active from its start through its end, inclusive; empty
/// notes are skipped.
fn notes(cx: &Context<'_>, ui: &mut egui::Ui, displayed: Rect) {
    let at = cx.playhead;
    let scene = cx.staged_project().data().scene();
    let width = (displayed.width() - kit::space::MD * 2.0).max(0.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                displayed.left_top() + Vec2::splat(kit::space::MD),
                Vec2::new(width, displayed.height()),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(displayed);
    for bookmark in &scene.bookmarks {
        let note = bookmark.note.as_str().trim();
        let end = bookmark.start.saturating_add(bookmark.duration);
        if note.is_empty() || at < bookmark.start || at > end {
            continue;
        }
        child.label(egui::RichText::new(note).color(kit::text(cx.dark)));
    }
}

fn canvas_size(cx: &Context<'_>) -> [u32; 2] {
    let c = &cx.session.project().data().canvas;
    [c.width, c.height]
}
fn timecode(cx: &Context<'_>, at: Tick) -> String {
    let rate = cx.session.project().data().canvas.fps;
    super::workflow::format_timecode(at, rate)
}
fn footer_controls(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    footer: Rect,
    empty: bool,
    actions: &mut Vec<Action>,
) {
    let dark = cx.dark;
    let y = footer.center().y;
    let current = timecode(cx, cx.hover.unwrap_or(cx.playhead));
    let total_hint = crate::i18n::format(
        cx.locale,
        Text::TimeTotal,
        &[("duration", &timecode(cx, cx.session.project().duration()))],
    );
    let total = timecode(cx, cx.session.project().duration());
    let field = Rect::from_center_size(
        egui::pos2(
            footer.left() + kit::space::MD + kit::size::PLAYER_TIMECODE / 2.0,
            y,
        ),
        egui::vec2(kit::size::PLAYER_TIMECODE, kit::size::FIELD),
    );
    at(ui, field, "player.timecode", |ui| {
        if let Some(edit) = state.timecode_edit.as_mut() {
            let r = ui.add_sized(
                field.size(),
                egui::TextEdit::singleline(&mut edit.text)
                    .id_salt("preview.timecode.field")
                    .font(FontId::monospace(kit::type_size::PLAYER_VALUE)),
            );
            if edit.focus {
                r.request_focus();
                edit.focus = false;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.timecode_edit = None;
            } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(Action::CommitTimecode);
            } else if r.lost_focus() {
                state.timecode_edit = None;
            }
            remember(cx, actions, "preview.timecode", r);
        } else {
            let r = ui
                .add_enabled_ui(!empty, |ui| {
                    let (rect, response) = ui.allocate_exact_size(field.size(), Sense::click());
                    let color = if empty {
                        kit::disabled(dark)
                    } else {
                        kit::accent(dark)
                    };
                    ui.painter().text(
                        rect.left_center() + egui::vec2(kit::space::HAIRLINE, 0.0),
                        Align2::LEFT_CENTER,
                        &current,
                        FontId::monospace(kit::type_size::PLAYER_VALUE),
                        color,
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, !empty, &current)
                    });
                    response
                })
                .inner
                .on_hover_text(&total_hint);
            if r.clicked() {
                state.timecode_edit = Some(TimecodeEdit {
                    text: current.clone(),
                    focus: true,
                });
                state.timecode_error = None;
            }
            remember(cx, actions, "preview.timecode", r);
        }
    });
    let slash = egui::pos2(field.right() + kit::space::XS, y);
    ui.painter().text(
        slash,
        Align2::LEFT_CENTER,
        cx.tr(Text::TimeTotal)
            .split_whitespace()
            .next()
            .unwrap_or_default(),
        FontId::monospace(kit::type_size::PLAYER_VALUE),
        kit::border(dark),
    );
    ui.painter().text(
        egui::pos2(slash.x + kit::space::ROW_INSET, y),
        Align2::LEFT_CENTER,
        total,
        FontId::monospace(kit::type_size::PLAYER_VALUE),
        kit::secondary(dark),
    );
    let has_status = !cx.playback.device_available()
        || state.failed
        || state.snapshot_failed
        || state.timecode_error.is_some();
    if !has_status && footer.width() > (kit::size::PLAYER_SCENES_X + kit::size::CONTROL) * 2.0 {
        let scenes = Rect::from_center_size(
            egui::pos2(footer.left() + kit::size::PLAYER_SCENES_X, y),
            Vec2::splat(kit::size::CONTROL),
        );
        at(ui, scenes, "player.scenes", |ui| {
            let r = ui
                .add_enabled_ui(!empty, |ui| {
                    kit::icon_button(
                        ui,
                        "preview.scenes",
                        kit::Icon::FilmStrip,
                        cx.tr(Text::Scenes),
                        false,
                    )
                })
                .inner;
            if r.clicked() {
                actions.push(Action::Command("scene.open".into()));
            }
            remember(cx, actions, "preview.scenes", r);
        });
    }
    // This readout precedes the centred transport without changing its position.
    let play = Rect::from_center_size(
        egui::pos2(footer.center().x, y),
        Vec2::splat(kit::size::CONTROL),
    );
    at(ui, play, "player.play", |ui| {
        let playing = cx.playback.playing();
        let label = cx.tr(if playing { Text::Pause } else { Text::Play });
        let r = ui
            .add_enabled_ui(!empty && cx.playback.can_play(), |ui| {
                kit::transport_button(
                    ui,
                    "transport.play",
                    if playing {
                        kit::Icon::Pause
                    } else {
                        kit::Icon::Play
                    },
                    label,
                    false,
                )
            })
            .inner;
        if r.clicked() {
            actions.push(Action::Command("transport.play".into()));
        }
        remember(cx, actions, "transport.play", r);
    });
    // [new] Master volume and loop sit left of Quality. CapCut's footer has no such row,
    // so it reuses the existing control, slider and spacing tokens only.
    let half = kit::size::CONTROL / 2.0;
    let quality_left = footer.right()
        - kit::space::XS
        - kit::size::PLAYER_QUALITY_X
        - kit::size::PLAYER_RATIO_WIDTH / 2.0;
    let loop_x = quality_left - kit::space::XS - half;
    let mute_x = loop_x - kit::size::CONTROL - kit::space::XS;
    let slider_left = mute_x - half - kit::space::XS - kit::size::ZOOM_SLIDER;
    let play_right = footer.center().x + half;
    if mute_x - half > play_right + kit::space::MD {
        let current = cx.session.preview_project().unwrap_or(cx.session.project());
        let settings = &current.data().settings;
        let (volume, muted) = (settings.master_volume, settings.master_muted);
        let loop_rect =
            Rect::from_center_size(egui::pos2(loop_x, y), Vec2::splat(kit::size::CONTROL));
        at(ui, loop_rect, "player.loop", |ui| {
            let r = ui
                .add_enabled_ui(!empty, |ui| {
                    kit::icon_button(
                        ui,
                        "playback.loop",
                        kit::Icon::Repeat,
                        cx.tr(Text::ActionPlaybackLoop),
                        cx.preferences.playback_loop,
                    )
                })
                .inner;
            if r.clicked() {
                actions.push(Action::Command("playback.loop".into()));
            }
            remember(cx, actions, "playback.loop", r);
        });
        let mute_rect =
            Rect::from_center_size(egui::pos2(mute_x, y), Vec2::splat(kit::size::CONTROL));
        at(ui, mute_rect, "player.mute", |ui| {
            let r = ui
                .add_enabled_ui(!empty, |ui| {
                    kit::icon_button(
                        ui,
                        "preview.volume.mute",
                        if muted {
                            kit::Icon::SpeakerSlash
                        } else {
                            kit::Icon::SpeakerHigh
                        },
                        cx.tr(if muted {
                            Text::AudioUnmuted
                        } else {
                            Text::AudioMuted
                        }),
                        muted,
                    )
                })
                .inner;
            if r.clicked() {
                actions.push(Action::MasterSet {
                    volume,
                    muted: !muted,
                });
            }
            remember(cx, actions, "preview.volume.mute", r);
        });
        // The slider needs the most room: it drops first when the footer is narrow.
        if slider_left > play_right + kit::space::MD {
            let slider_rect = Rect::from_center_size(
                egui::pos2(slider_left + kit::size::ZOOM_SLIDER / 2.0, y),
                Vec2::new(kit::size::ZOOM_SLIDER, kit::size::CONTROL),
            );
            at(ui, slider_rect, "player.volume", |ui| {
                let range = f64::from(*project::VOLUME_DB_RANGE.start())
                    ..=f64::from(*project::VOLUME_DB_RANGE.end());
                let (r, value) = ui
                    .add_enabled_ui(!empty, |ui| {
                        kit::rail_slider(
                            ui,
                            "preview.volume",
                            f64::from(volume),
                            range,
                            kit::size::ZOOM_SLIDER,
                        )
                    })
                    .inner;
                // The engine stores master volume on a hundredth-dB grid.
                let next = ((value * 100.0).round() / 100.0) as f32;
                if (r.drag_started() || r.dragged()) && next != volume {
                    actions.push(Action::MasterPreview {
                        volume: next,
                        muted,
                    });
                }
                if r.drag_stopped() {
                    actions.push(Action::MasterEnd { commit: true });
                } else if r.clicked() && next != volume {
                    actions.push(Action::MasterSet {
                        volume: next,
                        muted,
                    });
                }
                remember(cx, actions, "preview.volume", r);
            });
        }
    }
    let right = footer.right() - kit::space::XS;
    let full = Rect::from_center_size(
        egui::pos2(right - kit::size::CONTROL / 2.0, y),
        Vec2::splat(kit::size::CONTROL),
    );
    at(ui, full, "player.fullscreen", |ui| {
        let r = kit::icon_button(
            ui,
            "preview.fullscreen",
            kit::Icon::CornersOut,
            cx.tr(if state.fullscreen {
                Text::ExitFullscreen
            } else {
                Text::Fullscreen
            }),
            false,
        );
        if r.clicked() {
            actions.push(Action::Command("preview.fullscreen".into()));
        }
        remember(cx, actions, "preview.fullscreen", r);
    });
    let ratio = Rect::from_center_size(
        egui::pos2(right - kit::size::PLAYER_RATIO_X, y),
        egui::vec2(kit::size::PLAYER_RATIO_WIDTH, kit::size::FIELD),
    );
    at(ui, ratio, "player.ratio", |ui| {
        let r = kit::badge_button(ui, "preview.ratio", cx.tr(Text::PlayerRatio));
        egui::Popup::menu(&r).show(|ui| {
            let c = canvas_size(cx);
            ui.label(format!("{} × {}", c[0], c[1]));
        });
        remember(cx, actions, "preview.ratio", r);
    });
    let zoom = Rect::from_center_size(
        egui::pos2(right - kit::size::PLAYER_ZOOM_X, y),
        Vec2::splat(kit::size::CONTROL),
    );
    at(ui, zoom, "player.zoom", |ui| {
        let scale = state.view.scale(state.viewport, canvas_size(cx));
        let r = kit::icon_button(
            ui,
            "preview.zoom",
            kit::Icon::MagnifyingGlassPlus,
            &format!("{:.0}%", scale * 100.0),
            false,
        );
        egui::Popup::menu(&r).show(|ui| {
            if empty {
                menu(
                    ui,
                    cx,
                    actions,
                    "preview.zoom.fit",
                    Text::ZoomFit,
                    true,
                    "preview.zoom.fit",
                    None,
                );
            }
            for percent in preview::PRESETS {
                let id = format!("preview.zoom.{percent}");
                let r = ui.selectable_label(
                    !state.view.fit && (state.view.zoom - *percent as f32 / 100.0).abs() < 1e-4,
                    format!("{percent}%"),
                );
                if r.clicked() {
                    actions.push(Action::Command(id.clone()));
                    ui.close();
                }
                remember(cx, actions, &id, r);
            }
            ui.separator();
            menu(
                ui,
                cx,
                actions,
                "preview.zoom.out",
                Text::ZoomOut,
                true,
                "preview.zoom.out",
                None,
            );
            menu(
                ui,
                cx,
                actions,
                "preview.zoom.in",
                Text::ZoomIn,
                true,
                "preview.zoom.in",
                None,
            );
        });
        remember(cx, actions, "preview.zoom", r);
    });
    let quality = Rect::from_center_size(
        egui::pos2(right - kit::size::PLAYER_QUALITY_X, y),
        egui::vec2(kit::size::PLAYER_RATIO_WIDTH, kit::size::FIELD),
    );
    at(ui, quality, "player.quality", |ui| {
        let r = kit::badge_button(ui, "preview.quality", cx.tr(Text::PlayerFull));
        egui::Popup::menu(&r).show(|ui| {
            ui.label(cx.tr(Text::PlayerFull));
        });
        remember(cx, actions, "preview.quality", r);
    });
    if has_status {
        let status = Rect::from_center_size(
            if footer.width() > (kit::size::PLAYER_SCENES_X + kit::size::CONTROL) * 2.0 {
                egui::pos2(footer.left() + kit::size::PLAYER_SCENES_X, y)
            } else {
                egui::pos2(
                    footer.right() - kit::size::CONTROL * 2.0,
                    ui.max_rect().top() + kit::size::TAB_STRIP / 2.0,
                )
            },
            Vec2::splat(kit::size::CONTROL),
        );
        at(ui, status, "player.status", |ui| {
            let text = state.timecode_error.unwrap_or(if state.snapshot_failed {
                Text::SnapshotFailed
            } else if state.failed {
                Text::Unavailable
            } else {
                Text::Offline
            });
            let retry = state.failed || state.snapshot_failed || state.timecode_error.is_some();
            let id = if retry {
                "preview.retry"
            } else {
                "preview.audio_status"
            };
            // Both states are one control: recover on retry, report offline
            // otherwise. A stable widget id keeps a press that starts just
            // before the preview recovers paired with its release; `remember`
            // still reports the state's own automation id below.
            let r = ui
                .add_enabled_ui(retry, |ui| {
                    kit::icon_button(
                        ui,
                        "preview.status",
                        if retry {
                            kit::Icon::ArrowClockwise
                        } else {
                            kit::Icon::SpeakerSlash
                        },
                        cx.tr(text),
                        false,
                    )
                })
                .inner;
            if r.clicked() {
                actions.push(Action::Retry);
            }
            remember(cx, actions, id, r);
        });
    }
}
fn canvas(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    stage: Rect,
    empty: bool,
    actions: &mut Vec<Action>,
) {
    let r = ui.interact(
        stage,
        ui.make_persistent_id("preview.canvas"),
        Sense::click_and_drag(),
    );
    r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Image,
            true,
            cx.tr(if empty {
                Text::ProjectEmpty
            } else if source_missing_at_playhead(cx) {
                Text::MissingClipHint
            } else if cx.playback.frame().is_none() {
                if state.failed {
                    Text::Unavailable
                } else {
                    Text::Loading
                }
            } else {
                Text::Preview
            }),
        )
    });
    let size = canvas_size(cx);
    if r.hovered() && !empty {
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta()));
        if (zoom - 1.0).abs() > 1e-4 {
            actions.push(Action::Zoom(zoom));
        } else if scroll != Vec2::ZERO {
            state.view.pan(scroll, stage.size(), size);
            if state.view.can_pan() {
                ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
    }
    let (middle, delta) = ui.input(|i| {
        (
            i.pointer.button_down(egui::PointerButton::Middle),
            i.pointer.delta(),
        )
    });
    if middle && (r.hovered() || r.dragged()) {
        state.view.pan(delta, stage.size(), size);
    }
    if state.key_color_pick.is_none() {
        r.context_menu(|ui| {
            menu(
                ui,
                cx,
                actions,
                "preview.menu.snapshot",
                Text::SnapshotSave,
                !empty,
                "preview.snapshot",
                None,
            );
            menu(
                ui,
                cx,
                actions,
                "preview.menu.copy",
                Text::SnapshotCopy,
                !empty,
                "preview.copy_snapshot",
                None,
            );
            menu(
                ui,
                cx,
                actions,
                "preview.menu.fullscreen",
                Text::Fullscreen,
                true,
                "preview.fullscreen",
                None,
            );
            ui.separator();
            let scene = cx.staged_project().data().scene();
            for (widget, id, text, direction) in [
                (
                    "preview.menu.jump.next",
                    "playback.jump_bookmark.next",
                    Text::ActionPlaybackJumpBookmarkNext,
                    BookmarkDirection::Next,
                ),
                (
                    "preview.menu.jump.previous",
                    "playback.jump_bookmark.previous",
                    Text::ActionPlaybackJumpBookmarkPrevious,
                    BookmarkDirection::Previous,
                ),
            ] {
                menu(
                    ui,
                    cx,
                    actions,
                    widget,
                    text,
                    bookmark_target(scene, cx.playhead, direction).is_some(),
                    id,
                    cx.shortcut(id),
                );
            }
            ui.separator();
            let visibility = cx.preferences.preview_overlays;
            let mut notes = visibility.notes;
            let response = ui.checkbox(&mut notes, cx.tr(Text::PreviewNotes));
            if response.changed() {
                actions.push(Action::SetPreference {
                    key: "preview_overlays",
                    value: serde_json::json!(OverlayVisibility {
                        notes,
                        ..visibility
                    }),
                });
            }
            remember(cx, actions, "preview.menu.notes", response);
        });
    }
    remember(cx, actions, "preview.canvas", r.clone());
    if empty {
        state.texture.clear();
        return;
    }
    let scale = state.view.scale(stage.size(), size);
    state.view.offset = if state.view.fit {
        Vec2::ZERO
    } else {
        preview::clamp_offset(state.view.offset, stage.size(), size, scale)
    };
    let displayed = Rect::from_center_size(
        stage.center() + state.view.offset,
        egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale),
    );
    if source_missing_at_playhead(cx) {
        // The clip under the playhead has no file: the canvas is the missing red with one
        // centred line, as the reference's player shows (editor.md "Missing media").
        state.texture.clear();
        let shown = displayed.intersect(stage);
        ui.painter()
            .rect_filled(shown, 0.0, kit::missing_canvas(cx.dark));
        ui.painter().text(
            shown.center(),
            Align2::CENTER_CENTER,
            cx.tr(Text::MissingClipHint),
            FontId::proportional(kit::type_size::BODY),
            kit::on_missing(cx.dark),
        );
    } else if let Some(frame) = cx.playback.frame() {
        if let Some(texture) = state.texture.update(ui.ctx(), "preview.frame", frame) {
            ui.painter().with_clip_rect(stage).image(
                texture,
                displayed,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                kit::image_tint(),
            );
        }
    } else {
        state.texture.clear();
        kit::tinted(
            kit::Icon::ArrowClockwise,
            kit::icon::DENSE,
            kit::secondary(cx.dark),
        )
        .paint_at(
            ui,
            Rect::from_center_size(stage.center(), Vec2::splat(kit::icon::DENSE)),
        );
    }
    overlays(cx, ui, stage, displayed);
    // Either eyedropper samples the frame; the chroma key's pick and the colour popover's
    // pick differ only in the action they report.
    if state.key_color_pick.is_some() || cx.colour_sampling {
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if r.clicked()
            && let Some(pos) =
                ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()))
            && let Some(frame) = cx.playback.frame()
            && let Some(color) = preview::sample_color(&frame.image, displayed, pos)
        {
            actions.push(if state.key_color_pick.is_some() {
                Action::PickKeyColor(color)
            } else {
                Action::PickScreenColour(color)
            });
        }
    } else {
        super::transform::show(
            &mut state.transform,
            &mut state.mask_path,
            cx,
            ui,
            displayed,
            &r,
            cx.playhead,
            actions,
        );
    }
}

/// The main-track clip under the playhead names a source whose file is gone.
fn source_missing_at_playhead(cx: &Context<'_>) -> bool {
    let data = cx.staged_project().data();
    data.scene().timeline.main.items.iter().any(|item| {
        item.end()
            .is_ok_and(|end| item.start <= cx.playhead && cx.playhead < end)
            && item
                .source_asset
                .and_then(|id| data.assets.iter().find(|asset| asset.id() == id))
                .is_some_and(|asset| {
                    cx.visuals.availability(asset) == media::AssetAvailability::Missing
                })
    })
}
