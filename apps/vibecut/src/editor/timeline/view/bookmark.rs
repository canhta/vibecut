//! Draws the marker editor opened from a bookmark on the ruler.

use crate::editor::timeline::{
    Action, Align2, Color32, Context, Id, Rect, Rounding, State, Text, Tick, Vec2, egui, theme,
};
use crate::kit;

impl State {
    /// Draw the open marker editor and the colour window it owns. The panel
    /// returns engine commands; it never writes the project itself.
    pub(in crate::editor::timeline) fn show_bookmark_popover(
        &mut self,
        cx: &Context<'_>,
        ctx: &egui::Context,
    ) {
        let Some(mut open) = self.bookmark_popover.take() else {
            return;
        };
        let Some(bookmark) = cx
            .session
            .project()
            .data()
            .scene()
            .bookmarks
            .iter()
            .find(|bookmark| bookmark.id == open.bookmark)
            .cloned()
        else {
            return;
        };
        let id = bookmark.id;
        open.note_base = bookmark.note.as_str().to_owned();
        open.duration_base = bookmark.duration;
        let mut actions = Vec::new();
        let mut widgets = Vec::new();
        let mut close = false;
        let window = egui::Window::new(super::tr(cx, Text::BookmarkToggle))
            .id(Id::unique("timeline.bookmark.popover"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .pivot(Align2::LEFT_TOP)
            .fixed_pos(open.anchor)
            .frame(
                egui::Frame::window(&ctx.style_of(if cx.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                }))
                .inner_margin(egui::Margin::same(theme::space::LG as i8)),
            )
            .constrain_to(ctx.content_rect().shrink(theme::space::MD));
        window.show(ctx, |ui| {
            ui.set_width(256.0);
            ui.spacing_mut().item_spacing.y = theme::space::XS;

            let note_label = ui.label(egui::RichText::new(super::tr(cx, Text::BookmarkNote)));
            let mut note = open
                .note
                .clone()
                .unwrap_or_else(|| bookmark.note.as_str().to_owned());
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut note)
                        .id(Id::unique("timeline.bookmark.note"))
                        .hint_text(super::tr(cx, Text::BookmarkNoteHint))
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(note_label.id);
            record(&mut widgets, "timeline.bookmark.note", &response);
            if response.changed() {
                open.note = Some(note.clone());
            }
            let commit = response.has_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                || response.lost_focus();
            if commit {
                let edited = open.note.take().unwrap_or(note);
                if edited != open.note_base {
                    actions.push(Action::Command(Box::new(engine::Command::UpdateBookmark {
                        bookmark: id,
                        note: Some(edited.into()),
                        color: None,
                        duration: None,
                    })));
                }
            }

            ui.add_space(theme::space::MD);
            ui.label(egui::RichText::new(super::tr(cx, Text::BookmarkColor)));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::space::XS;
                let reset = bookmark.color != engine::DEFAULT_BOOKMARK_COLOR;
                let swatch_width = ui.available_width()
                    - if reset {
                        theme::size::CONTROL + theme::space::XS
                    } else {
                        0.0
                    };
                let [r, g, b, a] = bookmark.color.0;
                let swatch = ui.add(
                    egui::Button::new(String::new())
                        .fill(Color32::from_rgba_unmultiplied(r, g, b, a))
                        .min_size(Vec2::new(swatch_width, theme::size::FIELD)),
                );
                swatch.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        true,
                        super::tr(cx, Text::BookmarkColor),
                    )
                });
                record(&mut widgets, "timeline.bookmark.color", &swatch);
                if swatch.clicked() {
                    open.picker = None;
                    open_picker(&mut open, bookmark.color, swatch.rect);
                }
                if reset {
                    let reset = kit::icon_button(
                        ui,
                        "timeline.bookmark.color.reset",
                        kit::Icon::ArrowCounterClockwise,
                        super::tr(cx, Text::BookmarkColorReset),
                        false,
                    );
                    record(&mut widgets, "timeline.bookmark.color.reset", &reset);
                    if reset.clicked() {
                        actions.push(Action::Command(Box::new(engine::Command::UpdateBookmark {
                            bookmark: id,
                            note: None,
                            color: Some(engine::DEFAULT_BOOKMARK_COLOR),
                            duration: None,
                        })));
                    }
                }
            });

            ui.add_space(theme::space::MD);
            ui.label(egui::RichText::new(super::tr(cx, Text::BookmarkDuration)));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::space::XXS;
                let mut text = open
                    .duration
                    .clone()
                    .unwrap_or_else(|| format!("{:.1}", bookmark.duration.seconds()));
                let label = super::tr(cx, Text::BookmarkDuration);
                let response = kit::value_field(
                    ui,
                    "timeline.bookmark.duration",
                    &mut text,
                    theme::size::FIELD * 2.0 + 14.0,
                    None,
                    label,
                );
                record(&mut widgets, "timeline.bookmark.duration", &response);
                if response.changed() {
                    open.duration = Some(text.clone());
                }
                let (up, down) = kit::stepper(ui, "timeline.bookmark.duration", label);
                record(&mut widgets, "timeline.bookmark.duration.increase", &up);
                record(&mut widgets, "timeline.bookmark.duration.decrease", &down);
                for (clicked, sign) in [(up.clicked(), 1.0), (down.clicked(), -1.0)] {
                    if clicked {
                        let seconds = open
                            .duration
                            .as_deref()
                            .and_then(|text| text.trim().parse::<f64>().ok())
                            .unwrap_or_else(|| bookmark.duration.seconds());
                        if let Some(action) =
                            duration_action(id, seconds + sign * 0.1, open.duration_base)
                        {
                            actions.push(action);
                        }
                        open.duration = None;
                    }
                }
                let commit = response.has_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    || response.lost_focus();
                if commit {
                    let typed = open.duration.take();
                    if let Some(action) = typed.as_deref().and_then(|text| {
                        duration_action(id, text.trim().parse().ok()?, open.duration_base)
                    }) {
                        actions.push(action);
                    }
                }
            });

            ui.add_space(theme::space::MD);
            let delete = ui.add_sized(
                [ui.available_width(), theme::size::FIELD],
                egui::Button::image_and_text(
                    kit::tinted(
                        kit::Icon::Trash,
                        theme::icon::DENSE,
                        theme::error_text(cx.dark),
                    ),
                    super::tr(cx, Text::BookmarkDelete),
                ),
            );
            record(&mut widgets, "timeline.bookmark.delete", &delete);
            if delete.clicked() {
                actions.push(Action::Command(Box::new(engine::Command::RemoveBookmark {
                    bookmark: id,
                })));
                close = true;
            }
        });

        if let Some(mut picker) = open.picker.take() {
            let event = picker.ui(
                ctx,
                cx.locale,
                &cx.session.project().data().recent_colors,
            );
            for (id, response) in event.widgets {
                record(&mut widgets, &id, &response);
            }
            if !event.cancel {
                if event.commit && picker.color != picker.original {
                    actions.push(Action::Command(Box::new(engine::Command::Batch {
                        commands: vec![
                            engine::Command::UpdateBookmark {
                                bookmark: id,
                                note: None,
                                color: Some(picker.color),
                                duration: None,
                            },
                            engine::Command::RecordRecentColour {
                                color: picker.color,
                            },
                        ],
                    })));
                    picker.original = picker.color;
                }
                open.picker = Some(picker);
            }
        }

        for (id, response) in widgets {
            self.remember(id, &response);
        }
        self.outbox.extend(actions);
        if !close {
            self.bookmark_popover = Some(open);
        }
    }
}

/// One duration update; the engine clamps it to the scene tail. A value equal
/// to the project's current duration would be a refused no-op, so it is dropped.
fn duration_action(bookmark: project::BookmarkId, seconds: f64, base: Tick) -> Option<Action> {
    let tick = Tick::from_seconds_f64(seconds.max(0.0), Rounding::Nearest).ok()?;
    (tick != base).then(|| {
        Action::Command(Box::new(engine::Command::UpdateBookmark {
            bookmark,
            note: None,
            color: None,
            duration: Some(tick),
        }))
    })
}

fn open_picker(
    open: &mut super::super::state::BookmarkPopover,
    color: project::Color,
    swatch: Rect,
) {
    let mut picker = crate::services::color_picker::Picker::new(color);
    picker.anchor = Some(egui::pos2(
        swatch.right(),
        swatch.bottom() + theme::space::XS,
    ));
    open.picker = Some(picker);
}

fn record(widgets: &mut Vec<(String, egui::Response)>, id: &str, response: &egui::Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
    widgets.push((id.to_owned(), response.clone()));
}
