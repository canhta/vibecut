//! Paints the agent chat: title strip, transcript, consent sheet, changes card and input.
use super::{
    mention::{self, Target},
    state::{Action, Pending, State},
    transcript::{Counts, Decision, Entry},
};
use crate::{
    editor::Context,
    egui,
    i18n::{self, Locale, Text},
    kit::{self as theme, Icon},
};
use agent::Provider;
use ai::text::conversation::default_model;
use time::TICKS_PER_SECOND;

/// Height of the spend line above the input box.
const SPEND_ROW: f32 = 22.0;
/// The chat footer: spend line, 4 px gap, the 84 px input box and 10 px below it.
const FOOTER: f32 = SPEND_ROW + 4.0 + theme::size::AGENT_INPUT + theme::space::ROW_INSET;
/// Space kept free beside a bubble so the two speakers stay apart.
const BUBBLE_GAP: f32 = 28.0;
const BUBBLE_RADIUS: u8 = 8;
/// Automation id and widget salt of the service chooser.
const PROVIDER_ID: &str = "agent.provider";
/// Most candidates the `@` list shows.
const MENTIONS: usize = 6;

fn note(actions: &mut Vec<Action>, id: &str, response: &egui::Response) {
    actions.push(Action::Widget(id.to_owned(), response.clone()));
}

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "Anthropic",
        Provider::OpenAi => "OpenAI",
    }
}

/// Paint the panel into the slot of `ui` and return what the creator asked for.
pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    let mut actions = Vec::new();
    state.ctx = Some(ui.ctx().clone());
    if state.consent.is_some()
        && !state.sheet
        && !state.running()
        && state.pending.is_none()
        && let Some(prompt) = state.queued.take()
    {
        actions.push(Action::Send {
            display: prompt.clone(),
            sent: prompt,
        });
    }
    let panel = ui.max_rect();
    let stage = state.stage();
    header(state, cx, ui, panel, &mut actions);
    let footer = if matches!(stage, "ready" | "running" | "decide") {
        FOOTER
    } else {
        0.0
    };
    let body = egui::Rect::from_min_max(
        egui::pos2(panel.left(), panel.top() + theme::size::AGENT_HEADER),
        egui::pos2(panel.right(), panel.bottom() - footer),
    );
    let mut body_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    body_ui.set_clip_rect(body);
    match stage {
        "no_key" => no_key(cx, &mut body_ui),
        "consent" => sheet(state, cx, &mut body_ui, &mut actions),
        _ => list(state, cx, &mut body_ui, &mut actions),
    }
    if footer > 0.0 {
        let foot =
            egui::Rect::from_min_max(egui::pos2(panel.left(), panel.bottom() - footer), panel.max);
        input(state, cx, ui, foot, &mut actions);
    }
    actions
}

fn cell(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    enabled: bool,
    id: &str,
    glyph: Icon,
    label: &str,
) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            theme::icon_button(ui, id, glyph, label, false)
        })
        .inner
    })
    .inner
}

fn header(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    panel: egui::Rect,
    actions: &mut Vec<Action>,
) {
    let dark = cx.dark;
    let strip = egui::Rect::from_min_size(
        panel.min,
        egui::vec2(panel.width(), theme::size::AGENT_HEADER),
    );
    let title = ui.painter().text(
        egui::pos2(strip.left() + theme::space::LG, strip.center().y),
        egui::Align2::LEFT_CENTER,
        cx.tr(Text::AgentTitle),
        egui::FontId::proportional(theme::type_size::BODY),
        theme::text(dark),
    );
    ui.painter().text(
        egui::pos2(title.right() + theme::space::SM, strip.center().y),
        egui::Align2::LEFT_CENTER,
        cx.tr(Text::AgentBeta),
        egui::FontId::proportional(theme::type_size::BADGE),
        theme::secondary(dark),
    );
    ui.painter().hline(
        strip.x_range(),
        strip.bottom(),
        egui::Stroke::new(theme::space::HAIRLINE, theme::separator(dark)),
    );
    let side = theme::size::CONTROL;
    let close = egui::Rect::from_center_size(
        egui::pos2(
            strip.right() - theme::space::MD - side / 2.0,
            strip.center().y,
        ),
        egui::Vec2::splat(side),
    );
    let clear = close.translate(egui::vec2(-(side + theme::space::XS), 0.0));
    let idle = !state.running() && state.pending.is_none();
    let response = cell(
        ui,
        clear,
        idle && !state.entries.is_empty(),
        "agent.clear",
        Icon::Trash,
        cx.tr(Text::AgentClear),
    );
    if response.clicked() {
        actions.push(Action::Clear);
    }
    note(actions, "agent.clear", &response);
    let response = cell(
        ui,
        close,
        true,
        "agent.close",
        Icon::X,
        cx.tr(Text::AgentClose),
    );
    if response.clicked() {
        state.close();
    }
    note(actions, "agent.close", &response);
}

fn no_key(cx: &Context<'_>, ui: &mut egui::Ui) {
    egui::Frame::new()
        .inner_margin(egui::Margin::same(theme::space::LG as i8))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(cx.tr(Text::AgentNoKey))
                    .size(theme::type_size::BODY)
                    .color(theme::secondary(cx.dark)),
            );
        });
}

fn sheet(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let dark = cx.dark;
    egui::ScrollArea::vertical()
        .id_salt("agent.sheet")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(theme::space::LG as i8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = theme::space::MD;
                    ui.label(
                        egui::RichText::new(cx.tr(Text::AgentConsentTitle))
                            .strong()
                            .size(theme::type_size::CHROME),
                    );
                    ui.label(cx.tr(Text::AgentConsentIntro));
                    ui.horizontal(|ui| {
                        ui.label(cx.tr(Text::AgentService));
                        let combo = egui::ComboBox::from_id_salt(PROVIDER_ID)
                            .selected_text(provider_name(state.settings.provider))
                            .show_ui(ui, |ui| {
                                for provider in [Provider::Anthropic, Provider::OpenAi] {
                                    let row = ui.selectable_value(
                                        &mut state.settings.provider,
                                        provider,
                                        provider_name(provider),
                                    );
                                    note(
                                        actions,
                                        &format!("agent.provider.{}", provider.id()),
                                        &row,
                                    );
                                }
                            });
                        note(actions, PROVIDER_ID, &combo.response);
                    });
                    ui.horizontal(|ui| {
                        ui.label(cx.tr(Text::AgentVersion));
                        ui.label(
                            egui::RichText::new(
                                default_model(state.settings.provider.id()).unwrap_or_default(),
                            )
                            .color(theme::secondary(dark)),
                        );
                    });
                    let mut always = true;
                    ui.add_enabled(
                        false,
                        egui::Checkbox::new(&mut always, cx.tr(Text::AgentDataSummary)),
                    );
                    let names = ui.checkbox(&mut state.settings.names, cx.tr(Text::AgentDataNames));
                    note(actions, "agent.consent.names", &names);
                    let transcript = ui.checkbox(
                        &mut state.settings.transcript,
                        cx.tr(Text::AgentDataTranscript),
                    );
                    note(actions, "agent.consent.transcript", &transcript);
                    let frames =
                        ui.checkbox(&mut state.settings.frames, cx.tr(Text::AgentDataFrames));
                    note(actions, "agent.consent.frames", &frames);
                    ui.horizontal(|ui| {
                        ui.label(cx.tr(Text::AgentLimit));
                        let limit = ui.add(
                            egui::DragValue::new(&mut state.settings.limit_usd)
                                .range(0.1..=100.0)
                                .speed(0.05)
                                .fixed_decimals(2),
                        );
                        note(actions, "agent.consent.limit", &limit);
                    });
                    ui.label(
                        egui::RichText::new(cx.tr(Text::AgentConsentNote))
                            .size(theme::type_size::SMALL_LABEL)
                            .color(theme::secondary(dark)),
                    );
                    let keyed = state.backend.has_key(state.settings.provider);
                    if !keyed {
                        ui.label(
                            egui::RichText::new(cx.tr(Text::AgentNoKey))
                                .color(theme::warning(dark)),
                        );
                    }
                    if let Some(problem) = state.notice {
                        ui.label(
                            egui::RichText::new(cx.tr(problem)).color(theme::error_text(dark)),
                        );
                    }
                    ui.horizontal(|ui| {
                        let accept = ui
                            .add_enabled_ui(keyed, |ui| {
                                theme::dialog_button(
                                    ui,
                                    "agent.consent.accept",
                                    cx.tr(Text::AgentAccept),
                                    true,
                                )
                            })
                            .inner;
                        if accept.clicked() {
                            actions.push(Action::Accept);
                        }
                        note(actions, "agent.consent.accept", &accept);
                        let decline = theme::dialog_button(
                            ui,
                            "agent.consent.decline",
                            cx.tr(Text::AgentDecline),
                            false,
                        );
                        if decline.clicked() {
                            state.decline();
                        }
                        note(actions, "agent.consent.decline", &decline);
                    });
                });
        });
}

fn list(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let dark = cx.dark;
    let empty = state.entries.is_empty() && state.pending.is_none() && !state.running();
    egui::ScrollArea::vertical()
        .id_salt("agent.list")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(
                    theme::space::LG as i8,
                    theme::space::MD as i8,
                ))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = theme::space::SM;
                    if empty {
                        empty_state(state, cx, ui, actions);
                        return;
                    }
                    for entry in &state.entries {
                        entry_row(cx, ui, entry);
                    }
                    if state.running() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new(cx.tr(Text::AgentWorking))
                                    .color(theme::secondary(dark)),
                            );
                        });
                    }
                    if let Some(pending) = &state.pending {
                        card(pending, state.previewing, cx, ui, actions);
                    }
                    if let Some(problem) = state.notice {
                        ui.label(
                            egui::RichText::new(cx.tr(problem)).color(theme::error_text(dark)),
                        );
                    }
                });
        });
}

fn empty_state(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let dark = cx.dark;
    ui.horizontal(|ui| {
        let (glyph, _) =
            ui.allocate_exact_size(egui::Vec2::splat(theme::icon::DENSE), egui::Sense::hover());
        theme::tinted(Icon::Sparkle, theme::icon::DENSE, theme::accent(dark)).paint_at(ui, glyph);
        ui.label(
            egui::RichText::new(cx.tr(Text::AgentEmptyTitle))
                .strong()
                .size(theme::type_size::CHROME),
        );
    });
    ui.label(
        egui::RichText::new(cx.tr(Text::AgentEmpty))
            .size(theme::type_size::BODY)
            .color(theme::secondary(dark)),
    );
    ui.add_space(theme::space::SM);
    ui.spacing_mut().item_spacing.y = theme::space::MD;
    let chips = [
        Text::AgentChipCaptions,
        Text::AgentChipSilence,
        Text::AgentChipTidy,
    ];
    for (index, chip) in chips.into_iter().enumerate() {
        let label = cx.tr(chip);
        let response = ui.add(
            egui::Button::new(egui::RichText::new(label).size(theme::type_size::BODY))
                .fill(theme::hover(dark))
                .corner_radius(theme::size::AGENT_CHIP / 2.0)
                .min_size(egui::vec2(0.0, theme::size::AGENT_CHIP)),
        );
        if response.clicked() {
            state.input = label.to_owned();
            state.mentions.clear();
        }
        note(actions, &format!("agent.chip.{index}"), &response);
    }
}

fn bubble(ui: &mut egui::Ui, text: &str, fill: egui::Color32, ink: egui::Color32, right: bool) {
    let layout = egui::Layout::top_down(if right {
        egui::Align::Max
    } else {
        egui::Align::Min
    });
    ui.with_layout(layout, |ui| {
        ui.set_max_width((ui.available_width() - BUBBLE_GAP).max(0.0));
        egui::Frame::new()
            .fill(fill)
            .corner_radius(BUBBLE_RADIUS)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(text)
                        .size(theme::type_size::BODY)
                        .color(ink),
                );
            });
    });
}

fn seconds(locale: Locale, ticks: i64) -> String {
    format!(
        "{} s",
        i18n::decimal(locale, ticks as f64 / TICKS_PER_SECOND as f64, 1)
    )
}

/// The change summary as short lines: counts that are not zero, then the length when it moved.
fn summary_lines(locale: Locale, counts: &Counts) -> Vec<String> {
    let mut lines = Vec::new();
    for (value, key) in [
        (counts.added, Text::AgentChangesAdded),
        (counts.removed, Text::AgentChangesRemoved),
        (counts.moved, Text::AgentChangesMoved),
        (counts.trimmed, Text::AgentChangesTrimmed),
        (counts.retimed, Text::AgentChangesRetimed),
        (counts.edited, Text::AgentChangesEdited),
        (counts.captions, Text::AgentChangesCaptions),
    ] {
        if value > 0 {
            lines.push(i18n::format(
                locale,
                key,
                &[("n", &i18n::number(locale, u64::from(value)))],
            ));
        }
    }
    if counts.before != counts.after {
        lines.push(i18n::format(
            locale,
            Text::AgentChangesDuration,
            &[
                ("before", &seconds(locale, counts.before)),
                ("after", &seconds(locale, counts.after)),
            ],
        ));
    }
    if lines.is_empty() {
        lines.push(i18n::text(locale, Text::AgentChangesNone).to_owned());
    }
    lines
}

fn ended_text(locale: Locale, reason: &str) -> String {
    let text = |key| i18n::text(locale, key).to_owned();
    match reason {
        "cancelled" => text(Text::AgentEndCancelled),
        "step_limit" => text(Text::AgentEndSteps),
        "spend_limit" => text(Text::AgentEndSpend),
        "context_limit" => text(Text::AgentEndContext),
        "consent_required" => text(Text::AgentEndConsent),
        other => i18n::format(
            locale,
            Text::AgentEndProvider,
            &[("reason", other.strip_prefix("provider:").unwrap_or(other))],
        ),
    }
}

fn entry_row(cx: &Context<'_>, ui: &mut egui::Ui, entry: &Entry) {
    let dark = cx.dark;
    let small = |text: String, color| {
        egui::RichText::new(text)
            .size(theme::type_size::SMALL_LABEL)
            .color(color)
    };
    match entry {
        Entry::User { text } => bubble(ui, text, theme::hover(dark), theme::text(dark), true),
        Entry::Assistant { text } => {
            bubble(ui, text, theme::field(dark), theme::text(dark), false);
        }
        Entry::Step { name, ok, .. } => {
            let (state, color) = match ok {
                Some(true) => (Text::AgentStepOk, theme::secondary(dark)),
                Some(false) => (Text::AgentStepFailed, theme::error_text(dark)),
                None => (Text::AgentStepRunning, theme::secondary(dark)),
            };
            ui.label(small(format!("{name} · {}", cx.tr(state)), color));
        }
        Entry::Progress { percent, .. } => {
            ui.label(small(
                i18n::format(
                    cx.locale,
                    Text::AgentJob,
                    &[("percent", &percent.to_string())],
                ),
                theme::secondary(dark),
            ));
        }
        Entry::Ended { reason } => {
            ui.label(small(ended_text(cx.locale, reason), theme::warning(dark)));
        }
        Entry::Changes { counts, decision } => {
            let decided = match decision {
                Decision::Applied => Text::AgentApplied,
                Decision::Discarded => Text::AgentDiscarded,
            };
            let lines = summary_lines(cx.locale, counts).join(", ");
            ui.label(small(
                format!(
                    "{} · {lines} · {}",
                    cx.tr(Text::AgentChanges),
                    cx.tr(decided)
                ),
                theme::secondary(dark),
            ));
        }
    }
}

/// The changes card: the summary, then Preview, Apply and Discard, or Retry when stale.
fn card(
    pending: &Pending,
    previewing: bool,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
) {
    let dark = cx.dark;
    egui::Frame::new()
        .fill(theme::field(dark))
        .stroke(egui::Stroke::new(
            theme::space::HAIRLINE,
            theme::border(dark),
        ))
        .corner_radius(BUBBLE_RADIUS)
        .inner_margin(egui::Margin::same(theme::space::LG as i8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = theme::space::SM;
            ui.label(
                egui::RichText::new(cx.tr(Text::AgentChanges))
                    .strong()
                    .size(theme::type_size::BODY),
            );
            if !pending.complete {
                ui.label(
                    egui::RichText::new(cx.tr(Text::AgentPartial)).color(theme::warning(dark)),
                );
            }
            for line in summary_lines(cx.locale, &pending.counts) {
                ui.label(egui::RichText::new(line).size(theme::type_size::BODY));
            }
            if pending.stale {
                ui.label(egui::RichText::new(cx.tr(Text::AgentStale)).color(theme::warning(dark)));
            }
            ui.horizontal_wrapped(|ui| {
                let live = pending.fork.is_some();
                if live {
                    let label = if previewing {
                        Text::AgentPreviewExit
                    } else {
                        Text::AgentPreview
                    };
                    let preview = theme::dialog_button(ui, "agent.preview", cx.tr(label), false);
                    if preview.clicked() {
                        actions.push(Action::Preview(!previewing));
                    }
                    note(actions, "agent.preview", &preview);
                    let apply =
                        theme::dialog_button(ui, "agent.apply", cx.tr(Text::AgentApply), true);
                    if apply.clicked() {
                        actions.push(Action::Apply);
                    }
                    note(actions, "agent.apply", &apply);
                }
                if pending.stale {
                    let retry =
                        theme::dialog_button(ui, "agent.retry", cx.tr(Text::AgentRetry), true);
                    if retry.clicked() {
                        actions.push(Action::Retry);
                    }
                    note(actions, "agent.retry", &retry);
                }
                let discard =
                    theme::dialog_button(ui, "agent.discard", cx.tr(Text::AgentDiscard), false);
                if discard.clicked() {
                    actions.push(Action::Discard);
                }
                note(actions, "agent.discard", &discard);
            });
        });
}

/// The spend line, the measured 84 px input box with its send or stop button, and the
/// `@` list above it.
fn input(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    foot: egui::Rect,
    actions: &mut Vec<Action>,
) {
    let dark = cx.dark;
    let running = state.running();
    let deciding = state.pending.is_some();
    let spend = egui::Rect::from_min_size(
        egui::pos2(foot.left() + theme::space::LG, foot.top()),
        egui::vec2(foot.width() - theme::space::LG * 2.0, SPEND_ROW),
    );
    let limit = state
        .consent
        .as_ref()
        .map_or(0, |consent| consent.scope().limit.micros());
    let dollars = |micros: u64| format!("${}", i18n::decimal(cx.locale, micros as f64 / 1e6, 2));
    let line = i18n::format(
        cx.locale,
        Text::AgentSpend,
        &[
            ("spent", &dollars(state.spent_micros)),
            ("limit", &dollars(limit)),
        ],
    );
    let mut spend_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(spend)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    spend_ui.label(
        egui::RichText::new(line)
            .size(theme::type_size::SMALL_LABEL)
            .color(theme::secondary(dark)),
    );
    spend_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let change = ui.add_enabled(
            !running && !deciding,
            egui::Button::new(
                egui::RichText::new(cx.tr(Text::AgentSettings))
                    .size(theme::type_size::SMALL_LABEL)
                    .color(theme::secondary(dark)),
            )
            .frame(false),
        );
        if change.clicked() {
            state.sheet = true;
        }
        note(actions, "agent.settings", &change);
    });

    let box_rect = egui::Rect::from_min_max(
        egui::pos2(foot.left() + theme::space::MD, foot.top() + SPEND_ROW + 4.0),
        egui::pos2(
            foot.right() - theme::space::MD,
            foot.bottom() - theme::space::ROW_INSET,
        ),
    );
    let id = egui::Id::new("agent.input");
    let focused = ui.memory(|memory| memory.has_focus(id));
    let outline = if focused || running {
        theme::accent(dark)
    } else {
        theme::border(dark)
    };
    ui.painter()
        .rect_filled(box_rect, BUBBLE_RADIUS, theme::field(dark));
    ui.painter().rect_stroke(
        box_rect,
        BUBBLE_RADIUS,
        egui::Stroke::new(theme::space::HAIRLINE, outline),
        egui::StrokeKind::Inside,
    );

    // The `@` search: what the creator is typing after the last `@`, and what it matches.
    let query = mention::active_query(&state.input, &state.mentions).map(str::to_owned);
    let candidates = match &query {
        Some(_) if focused => mention::candidates(cx.staged_project(), cx.locale),
        _ => Vec::new(),
    };
    let shown: Vec<mention::Mention> = query
        .as_deref()
        .map(|query| {
            mention::matching(&candidates, query, MENTIONS)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let typing = !running && !deciding;
    let shift = ui.input(|input| input.modifiers.shift);
    let enter = typing
        && focused
        && !shift
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let mut pick = None;
    if enter && let Some(first) = shown.first() {
        pick = Some(first.clone());
    }

    let text_rect = egui::Rect::from_min_max(
        box_rect.min + egui::vec2(theme::space::MD, theme::space::MD),
        egui::pos2(
            box_rect.right() - theme::space::MD,
            box_rect.bottom() - 34.0,
        ),
    );
    let mut text_ui = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
    text_ui.set_clip_rect(text_rect);
    {
        // The box paints its own frame, so the field draws none.
        let visuals = text_ui.visuals_mut();
        visuals.extreme_bg_color = egui::Color32::TRANSPARENT;
        visuals.selection.stroke = egui::Stroke::NONE;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    }
    let hint = if deciding {
        Text::AgentDecide
    } else {
        Text::AgentPlaceholder
    };
    let edit = text_ui.add_enabled(
        typing,
        egui::TextEdit::multiline(&mut state.input)
            .id(id)
            .hint_text(cx.tr(hint))
            .desired_rows(2)
            .char_limit(8 * 1024)
            .desired_width(text_rect.width()),
    );
    note(actions, "agent.input", &edit);

    let button = egui::Rect::from_min_size(
        egui::pos2(
            box_rect.right() - theme::space::MD - 64.0,
            box_rect.bottom() - 30.0,
        ),
        egui::vec2(64.0, theme::size::CONTROL),
    );
    if running {
        let stop = ui.put(button, egui::Button::new(cx.tr(Text::AgentStop)));
        if stop.clicked() {
            actions.push(Action::Stop);
        }
        note(actions, "agent.stop", &stop);
    } else {
        let ready = typing && !state.input.trim().is_empty();
        let label = egui::RichText::new(cx.tr(Text::AgentSend)).color(theme::on_accent(dark));
        let send = ui.put(
            button,
            egui::Button::new(label)
                .fill(theme::accent(dark))
                .sense(if ready {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                }),
        );
        note(actions, "agent.send", &send);
        if (send.clicked() || (enter && pick.is_none())) && ready {
            let display = state.input.trim().to_owned();
            let sent = mention::resolve(&display, &state.mentions);
            actions.push(Action::Send { display, sent });
            state.input.clear();
            state.mentions.clear();
        }
    }

    if !shown.is_empty() && focused {
        let popup = egui::Area::new(egui::Id::new("agent.mentions"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(box_rect.left_top())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    for (index, item) in shown.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let row = ui.selectable_label(false, item.label.as_str());
                            let tag = match item.target {
                                Target::Item => Text::AgentMentionItem,
                                Target::Media => Text::AgentMentionAsset,
                            };
                            ui.weak(cx.tr(tag));
                            note(actions, &format!("agent.mention.{index}"), &row);
                            if row.clicked() {
                                pick = Some(item.clone());
                            }
                        });
                    }
                });
            });
        let _ = popup;
    }
    if let Some(picked) = pick {
        mention::insert(&mut state.input, &picked);
        state.mentions.push(picked);
        ui.memory_mut(|memory| memory.request_focus(id));
    }
}
