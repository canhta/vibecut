//! Renders a clip's playback controls: the Audio tab's volume and fades, and the constant
//! speed rows. Mute lives on the track header and the clip menu.
use super::Panel;
use crate::editor::properties::workflow::jobs::loudness::{Cause, Failed, scope_target};
use crate::editor::properties::{
    AUDIO_GAIN_SCRUB, Action, Color32, ItemId, Rect, SPEED_SCRUB, ScrubSpec, Text, Vec2, egui,
    read, theme,
};
use crate::i18n;
use crate::kit;
use engine::loudness::LoudnessResult;
use project::{LoudnessScope, LoudnessTarget};

mod curve;

/// Drafts of the two loudness target fields, by field key.
const LUFS_DRAFT: &str = "loudness_target";
const PEAK_DRAFT: &str = "loudness_true_peak";
const LUFS_SUFFIX: &str = " LUFS";
const PEAK_SUFFIX: &str = " dBTP";

/// The loudness state one frame of the Audio tab reads, copied out of the project and the
/// workflow so the rows can push actions while they draw.
struct LoudnessFrame {
    locale: i18n::Locale,
    clip_on: bool,
    master_on: bool,
    shown: LoudnessTarget,
    progress: Option<f32>,
    failed: Option<Failed>,
    result: Option<LoudnessResult>,
}

/// A typed loudness value with its unit, such as `-14 LUFS`; `None` for anything else.
fn parse_loudness(text: &str, suffix: &str) -> Option<f32> {
    let value = text
        .trim()
        .trim_end_matches(suffix)
        .trim()
        .parse::<f32>()
        .ok()?;
    value.is_finite().then_some(value)
}

/// The phrase that names a failed measurement's cause.
const fn cause_text(cause: Cause) -> Text {
    match cause {
        Cause::Source => Text::LoudnessCauseSource,
        Cause::Decode => Text::LoudnessCauseDecode,
        Cause::Cancelled => Text::LoudnessCauseCancelled,
        Cause::Other => Text::LoudnessCauseOther,
    }
}

/// One result line: the scope's label, then its detail.
fn loudness_line(locale: i18n::Locale, scope: Text, detail: &str) -> String {
    let scope = i18n::text(locale, scope);
    i18n::format(
        locale,
        Text::LoudnessLine,
        &[("scope", scope), ("detail", detail)],
    )
}

impl Panel<'_, '_> {
    /// The Audio section of a video with source audio or an audio clip. Locked
    /// tracks show the same rows disabled.
    pub(super) fn audio_section(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let editable = !read::item_locked(self.cx.session.project(), item);
        self.panel_section(
            ui,
            "inspector.section.audio",
            "inspector.audio",
            Text::Audio,
            true,
            false,
            |panel, ui| {
                ui.add_enabled_ui(editable, |ui| {
                    panel.slider_row(ui, item, AUDIO_GAIN_SCRUB);
                    let fade_in = panel.fade_scrub(item, project::FadeEdge::In);
                    panel.slider_row(ui, item, fade_in);
                    let fade_out = panel.fade_scrub(item, project::FadeEdge::Out);
                    panel.slider_row(ui, item, fade_out);
                    panel.noise_reduction_row(ui, item);
                    panel.voice_enhancement_rows(ui, item);
                });
                panel.loudness_rows(ui, editable);
            },
        );
    }

    /// Normalize loudness: a switch per scope, the shared target fields, Apply, and one status
    /// line for progress, failure, a refused target, the result or the unmeasured state. Scope
    /// switches and target commits are one undo step each; Apply runs the measurement job.
    /// Measuring disables the switches, fields and Apply and shows the progress with Cancel.
    fn loudness_rows(&mut self, ui: &mut egui::Ui, editable: bool) {
        let frame = self.loudness_frame();
        let measuring = frame.progress.is_some();
        let mut committed = false;
        ui.add_enabled_ui(editable && !measuring, |ui| {
            let (row, _) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::LoudnessNormalize));
            self.loudness_scope_row(ui, LoudnessScope::Clip, &frame);
            self.loudness_scope_row(ui, LoudnessScope::Master, &frame);
            for (draft, id, label, suffix, shown) in [
                (
                    LUFS_DRAFT,
                    "inspector.loudness.target",
                    Text::LoudnessTarget,
                    LUFS_SUFFIX,
                    frame.shown.target_lufs,
                ),
                (
                    PEAK_DRAFT,
                    "inspector.loudness.true_peak",
                    Text::LoudnessTruePeak,
                    PEAK_SUFFIX,
                    frame.shown.true_peak_db,
                ),
            ] {
                committed |= self.loudness_field(ui, draft, id, label, suffix, shown);
            }
            let (_, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            let width = value_ui.available_width();
            let enabled = (frame.clip_on || frame.master_on)
                && self
                    .typed_loudness(frame.shown)
                    .is_some_and(|t| t.is_valid());
            let apply = value_ui
                .push_id("inspector.loudness.apply", |ui| {
                    ui.add_enabled(
                        enabled,
                        egui::Button::new(self.tr(Text::LoudnessApply))
                            .min_size(egui::vec2(width, theme::size::CONTROL)),
                    )
                })
                .inner;
            self.remember("inspector.loudness.apply", &apply);
            if apply.clicked() {
                self.actions.push(Action::MeasureLoudness);
            }
        });
        if committed {
            self.commit_loudness(frame.shown);
        }
        if let Some(progress) = frame.progress {
            let (_, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            // Cancel takes its width at the right first; the bar fills what is left of the row.
            let label = self.tr(Text::LoudnessCancel);
            let (cancel, bar) = value_ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cancel = ui
                        .push_id("inspector.loudness.cancel", |ui| {
                            ui.add(egui::Button::new(label))
                        })
                        .inner;
                    let bar = kit::progress_bar(ui, "inspector.loudness.progress", progress);
                    (cancel, bar)
                })
                .inner;
            self.remember("inspector.loudness.progress", &bar);
            self.remember("inspector.loudness.cancel", &cancel);
            if cancel.clicked() {
                self.actions.push(Action::CancelLoudness);
            }
        }
        // The status line takes a full property row, so it lines up with the rows above it.
        for (line, color) in self.loudness_status(&frame) {
            let status = ui
                .allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), theme::size::PROPERTY_ROW),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            egui::RichText::new(line)
                                .size(theme::type_size::BODY)
                                .color(color),
                        )
                    },
                )
                .inner;
            self.remember("inspector.loudness.status", &status);
        }
    }

    /// Copies the loudness state this frame draws from.
    fn loudness_frame(&self) -> LoudnessFrame {
        let project = self.cx.session.project();
        LoudnessFrame {
            locale: self.cx.locale,
            clip_on: scope_target(project, LoudnessScope::Clip).is_some(),
            master_on: scope_target(project, LoudnessScope::Master).is_some(),
            shown: self.state.loudness.shown_target(project),
            progress: self.state.loudness.progress(),
            failed: self.state.loudness.failure().cloned(),
            result: self.state.loudness.result_for(project).cloned(),
        }
    }

    /// One scope's switch: on sets the shown target on that scope, off clears it.
    fn loudness_scope_row(
        &mut self,
        ui: &mut egui::Ui,
        scope: LoudnessScope,
        frame: &LoudnessFrame,
    ) {
        let (id, label, on) = match scope {
            LoudnessScope::Clip => ("inspector.loudness.clip", Text::LoudnessClip, frame.clip_on),
            LoudnessScope::Master => (
                "inspector.loudness.master",
                Text::LoudnessMaster,
                frame.master_on,
            ),
        };
        let mut value = on;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(&mut value_ui, id, &mut value, self.tr(label));
        self.remember(id, &response);
        if response.clicked() {
            self.actions.push(Action::SetLoudnessScope {
                scope,
                enabled: !on,
            });
        }
    }

    /// One target field: a draft committed on Enter or blur and cancelled by Escape. Returns
    /// whether it committed this frame.
    fn loudness_field(
        &mut self,
        ui: &mut egui::Ui,
        draft: &'static str,
        id: &'static str,
        label: Text,
        suffix: &str,
        shown: f32,
    ) -> bool {
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        // The field spans the value column, to the right edge of the switches and Apply.
        let width = value_ui.available_width();
        let mut next = self
            .state
            .drafts
            .get(draft)
            .cloned()
            .unwrap_or_else(|| format!("{shown}{suffix}"));
        let response = kit::value_field(&mut value_ui, id, &mut next, width, None, self.tr(label));
        self.remember(id, &response);
        if response.has_focus() {
            self.state.focus = Some(draft.to_owned());
        }
        if response.changed() {
            self.state.drafts.insert(draft.to_owned(), next);
        }
        if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(draft);
            self.state.focus = None;
            response.surrender_focus();
            return false;
        }
        let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.state.focus = None;
            return self.state.drafts.contains_key(draft);
        }
        false
    }

    /// The target the fields would commit: each field's draft, else the shown value. `None`
    /// when a draft does not parse.
    fn typed_loudness(&self, shown: LoudnessTarget) -> Option<LoudnessTarget> {
        let target_lufs = match self.state.drafts.get(LUFS_DRAFT) {
            Some(text) => parse_loudness(text, LUFS_SUFFIX)?,
            None => shown.target_lufs,
        };
        let true_peak_db = match self.state.drafts.get(PEAK_DRAFT) {
            Some(text) => parse_loudness(text, PEAK_SUFFIX)?,
            None => shown.true_peak_db,
        };
        Some(LoudnessTarget {
            target_lufs,
            true_peak_db,
        })
    }

    /// Commit the typed fields as one target edit when they form a valid target. A refused
    /// draft stays in its field and the status line says why.
    fn commit_loudness(&mut self, shown: LoudnessTarget) {
        let Some(target) = self.typed_loudness(shown).filter(|t| t.is_valid()) else {
            return;
        };
        self.state.drafts.remove(LUFS_DRAFT);
        self.state.drafts.remove(PEAK_DRAFT);
        self.actions.push(Action::SetLoudnessTarget(target));
    }

    /// The one status line, in priority order: progress, failure, a refused target, no scope,
    /// the result for the current gains, or the unmeasured hint. Each line carries its colour:
    /// a failure paints the error token, a stale notice the warning token, the rest the
    /// secondary ink.
    fn loudness_status(&self, frame: &LoudnessFrame) -> Vec<(String, egui::Color32)> {
        let locale = frame.locale;
        let dark = self.cx.dark;
        let secondary = self.secondary();
        let tr = |key: Text| i18n::text(locale, key).to_owned();
        if let Some(progress) = frame.progress {
            let percent = format!("{:.0}", progress.clamp(0.0, 1.0) * 100.0);
            return vec![(
                i18n::format(locale, Text::LoudnessMeasuring, &[("percent", &percent)]),
                secondary,
            )];
        }
        match &frame.failed {
            Some(Failed::Stale) => {
                return vec![(tr(Text::LoudnessStale), kit::warning(dark))];
            }
            Some(Failed::Measurement(cause)) => {
                let reason = tr(cause_text(*cause));
                return vec![(
                    i18n::format(locale, Text::LoudnessFailed, &[("reason", &reason)]),
                    kit::error_text(dark),
                )];
            }
            None => {}
        }
        if !self
            .typed_loudness(frame.shown)
            .is_some_and(|t| t.is_valid())
        {
            return vec![(tr(Text::LoudnessInvalid), secondary)];
        }
        if !frame.clip_on && !frame.master_on {
            return vec![(tr(Text::LoudnessNoScope), secondary)];
        }
        let Some(result) = &frame.result else {
            return vec![(tr(Text::LoudnessUnmeasured), secondary)];
        };
        let mut lines = Vec::new();
        if frame.clip_on {
            let total = result.clips.len().to_string();
            let silent = result
                .clips
                .iter()
                .filter(|(_, measured)| measured.integrated_lufs.is_none())
                .count()
                .to_string();
            let detail = i18n::format(
                locale,
                Text::LoudnessClipSummary,
                &[("total", &total), ("silent", &silent)],
            );
            lines.push((
                loudness_line(locale, Text::LoudnessClip, &detail),
                secondary,
            ));
        }
        if let (true, Some(measured)) = (frame.master_on, result.master) {
            let detail = match (measured.integrated_lufs, measured.true_peak_db) {
                (Some(lufs), Some(peak)) => {
                    let lufs = format!("{lufs:.1}");
                    let peak = format!("{peak:.1}");
                    let gain = format!("{:+.2}", result.gains.master_db);
                    i18n::format(
                        locale,
                        Text::LoudnessMeasured,
                        &[("lufs", &lufs), ("peak", &peak), ("gain", &gain)],
                    )
                }
                _ => tr(Text::LoudnessSilent),
            };
            lines.push((
                loudness_line(locale, Text::LoudnessMaster, &detail),
                secondary,
            ));
        }
        lines
    }

    /// The reference's Noise reduction switch after the fades. The capture shows no strength
    /// control, so the switch applies the default strength; other strengths come through the
    /// `audio.*_cleanup_effect` commands and keep this switch on.
    fn noise_reduction_row(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let on = read::noise_reduction(self.cx.session.project(), item)
            .is_some_and(|effect| effect.enabled);
        let mut value = on;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::NoiseReduction));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(
            &mut value_ui,
            "inspector.noise_reduction.toggle",
            &mut value,
            self.tr(Text::NoiseReduction),
        );
        self.remember("inspector.noise_reduction.toggle", &response);
        if response.clicked() {
            self.actions
                .push(Action::SetNoiseReduction { item, enabled: !on });
        }
    }

    /// Voice enhancement: one switch over the preset's cleanup effects, and while it is on an
    /// Intensity row of four dry/wet mixes (25 to 100: the reference slider's 4 tick marks;
    /// new effects start at 75). The preset's parameters stay ordinary chain effects
    /// reachable through `audio.*_cleanup_effect`.
    fn voice_enhancement_rows(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let project = self.cx.session.project();
        let on = read::voice_enhancement_on(project, item);
        let strength = read::voice_enhancement_strength(project, item);
        let mut value = on;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::VoiceEnhancement));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(
            &mut value_ui,
            "inspector.voice_enhance.toggle",
            &mut value,
            self.tr(Text::VoiceEnhancement),
        );
        self.remember("inspector.voice_enhance.toggle", &response);
        if response.clicked() {
            self.actions
                .push(Action::SetVoiceEnhancement { item, enabled: !on });
        }
        if !on {
            return;
        }
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::VoiceEnhancementStrength));
        // The reference's Intensity slider shows 4 tick marks, every 25 up to 100.
        let levels: [(&str, f32); 4] = [("25", 25.0), ("50", 50.0), ("75", 75.0), ("100", 100.0)];
        let ids: Vec<String> = levels
            .iter()
            .map(|(name, _)| format!("inspector.voice_enhance.strength.{name}"))
            .collect();
        let labels: Vec<&str> = levels.iter().map(|(name, _)| *name).collect();
        let selected = levels
            .iter()
            .position(|(_, wet)| strength.is_some_and(|mix| (mix - wet).abs() < 0.5))
            .unwrap_or(usize::MAX);
        let cells = kit::segmented(&mut value_ui, &ids, &labels, selected);
        for (index, ((id, cell), (_, wet_percent))) in
            ids.iter().zip(&cells).zip(&levels).enumerate()
        {
            self.remember(id.clone(), cell);
            if cell.clicked() && index != selected {
                self.actions.push(Action::SetVoiceEnhancementStrength {
                    item,
                    wet_percent: *wet_percent,
                });
            }
        }
    }

    /// One fade edge's scrub contract: 0 to 10 s, capped at the room the clip
    /// leaves beside the other fade.
    fn fade_scrub(&self, item: ItemId, edge: project::FadeEdge) -> ScrubSpec {
        let room = read::fade_room_seconds(self.cx.session.project(), item, edge)
            .unwrap_or(project::MAX_FADE.seconds());
        let (field, label) = match edge {
            project::FadeEdge::In => ("audio_fade_in", Text::FadeIn),
            project::FadeEdge::Out => ("audio_fade_out", Text::FadeOut),
        };
        ScrubSpec {
            field,
            label,
            min: 0.0,
            max: room.min(project::MAX_FADE.seconds()),
            step: 0.1,
            pixels_per_step: 8.0,
            decimals: 1,
            suffix: "s",
            scale: 1.0,
            floor_label: None,
        }
    }
}

impl Panel<'_, '_> {
    /// Constant speed with reset and the pitch policy; both go through `timeline.set_speed`.
    pub(super) fn speed_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let project = self.cx.session.project();
        let Some((current, maintain_pitch, has_audio)) = read::retime_of(project, item) else {
            return;
        };
        let shape = read::speed_shape(project, item);
        self.speed_tabs(ui, item, shape.is_some(), maintain_pitch);
        if shape.is_some() || self.state.speed_curve_view {
            self.speed_curve_controls(ui, item, shape, maintain_pitch, has_audio);
            return;
        }
        self.slider_row(ui, item, SPEED_SCRUB);
        self.speed_duration_row(ui, item, current, maintain_pitch);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::SpeedReverse));
        let mut reversed = read::retime_reversed(project, item).unwrap_or(false);
        let response = value_ui.add(egui::Checkbox::without_text(&mut reversed));
        let name = self.tr(Text::SpeedReverse);
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label(name));
        self.remember("inspector.speed.reverse", &response);
        if response.changed() {
            self.set_reverse(item, reversed);
        }
        if has_audio {
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::ChangePitch));
            // "Change audio pitch" on lets pitch follow speed, so it is the inverse of
            // maintain_pitch. The switch sits at the row's right edge, as in the Audio tab.
            let mut follows = !maintain_pitch;
            let name = self.tr(Text::ChangePitch);
            let width = value_ui.available_width();
            value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
            let response = kit::toggle(&mut value_ui, "inspector.speed.pitch", &mut follows, name);
            self.remember("inspector.speed.pitch", &response);
            if response.clicked() {
                self.set_speed(item, current, !follows);
            }
        }
        self.speed_reset_row(ui, item, current, maintain_pitch);
    }

    /// The Standard view's Duration rows, laid out like the Speed slider above: the label on
    /// its own line, then the source length, a dashed arrow and the editable length. The
    /// length field and its stepper share the slider field's width and right edge. A typed
    /// length, or a stepper step of `SPEED_DURATION_STEP`, sets the speed to source over the
    /// length, one undo step; the speed clamps to the slider's range. Escape drops the draft.
    fn speed_duration_row(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        current: f64,
        maintain_pitch: bool,
    ) {
        let Some((source, output)) = read::retime_seconds(self.cx.session.project(), item) else {
            return;
        };
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), SPEED_DURATION_ROW),
            egui::Sense::hover(),
        );
        let label_line = Rect::from_min_size(row.min, Vec2::new(row.width(), SPEED_DURATION_LABEL));
        self.row_label(ui, label_line, self.tr(Text::SpeedDuration));
        let line = Rect::from_min_size(
            egui::pos2(row.left(), row.top() + SPEED_DURATION_LINE),
            Vec2::new(row.width(), theme::size::FIELD),
        );
        // The Speed field's right edge: the keyframe cluster's reserve sits to its right.
        let group_right = line.right() - super::keyframe_reserve(&[SPEED_SCRUB]);
        let group = Rect::from_min_max(
            egui::pos2(group_right - SPEED_DURATION_GROUP, line.top()),
            egui::pos2(group_right, line.bottom()),
        );
        let mut source_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(line)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let source_text = source_ui.label(
            egui::RichText::new(format!("{source:.2}s"))
                .size(theme::type_size::BODY)
                .color(self.secondary()),
        );
        self.remember("inspector.speed.duration", &source_text);
        let arrow_from = source_text.rect.right() + theme::space::XS;
        let arrow_to = group.left() - theme::space::XS;
        draw_dashed_arrow(
            ui.painter(),
            arrow_from,
            arrow_to,
            line.center().y,
            self.secondary(),
        );

        let key = SPEED_DURATION_DRAFT;
        let mut text = self
            .state
            .drafts
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("{output:.2}s"));
        let mut group_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(group)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        group_ui.spacing_mut().item_spacing.x = theme::space::XXS;
        let field = kit::value_field(
            &mut group_ui,
            "inspector.speed.duration.field",
            &mut text,
            SPEED_DURATION_WIDTH,
            None,
            self.tr(Text::SpeedDuration),
        );
        self.remember("inspector.speed.duration.field", &field);
        let (up, down) = kit::stepper(
            &mut group_ui,
            "inspector.speed.duration.field",
            self.tr(Text::SpeedDuration),
        );
        self.remember("inspector.speed.duration.field.increase", &up);
        self.remember("inspector.speed.duration.field.decrease", &down);
        if field.has_focus() {
            self.state.focus = Some(key.to_owned());
        }
        if field.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(key);
            self.state.focus = None;
            field.surrender_focus();
            return;
        }
        // The stepper lengthens (up) or shortens (down) the shown length from its committed
        // value; it wins over a pending draft, which it discards.
        let step = if up.clicked() {
            SPEED_DURATION_STEP
        } else if down.clicked() {
            -SPEED_DURATION_STEP
        } else {
            0.0
        };
        if step != 0.0 {
            self.state.focus = None;
            self.state.drafts.remove(key);
            let seconds = output + step;
            if seconds > 0.0 {
                self.commit_duration(item, source, seconds, current, maintain_pitch);
            }
            return;
        }
        if field.changed() {
            self.state.drafts.insert(key.to_owned(), text.clone());
        }
        let enter = field.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter || field.lost_focus() {
            self.state.focus = None;
            self.state.drafts.remove(key);
            if let Some(seconds) = parse_seconds(&text) {
                self.commit_duration(item, source, seconds, current, maintain_pitch);
            }
        }
    }

    /// Sets the speed that plays the source in `seconds`, when that differs from `current`.
    fn commit_duration(
        &mut self,
        item: ItemId,
        source: f64,
        seconds: f64,
        current: f64,
        maintain_pitch: bool,
    ) {
        let speed = SPEED_SCRUB.quantize(source / seconds);
        if (speed - current).abs() > f64::EPSILON {
            self.set_speed(item, speed, maintain_pitch);
        }
    }

    /// Reset in the panel's bottom-right corner: back to a constant 1x, one undo step. It sits
    /// on the visible bottom of the panel when the rows leave room, and right after the last
    /// row when they fill it.
    fn speed_reset_row(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        current: f64,
        maintain_pitch: bool,
    ) {
        let footer_top = ui.clip_rect().bottom() - SPEED_FOOTER_INSET - theme::size::FIELD;
        let room = footer_top - ui.next_widget_position().y;
        if room > 0.0 {
            ui.add_space(room);
        }
        let label = self.tr(Text::SpeedReset);
        let reset = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled(current != 1.0, egui::Button::new(label))
            })
            .inner;
        self.remember("inspector.speed.reset", &reset);
        if reset.clicked() {
            self.set_speed(item, 1.0, maintain_pitch);
        }
    }
}

/// The Duration rows: a label line, then a value line 14 px lower, as the Speed slider. The
/// length field keeps the slider field's 54 px width, 2 px gap and 16 px stepper.
const SPEED_DURATION_ROW: f32 = 44.0;
const SPEED_DURATION_LABEL: f32 = 18.0;
const SPEED_DURATION_LINE: f32 = 14.0;
const SPEED_DURATION_WIDTH: f32 = 54.0;
const SPEED_DURATION_GROUP: f32 = SPEED_DURATION_WIDTH + theme::space::XXS + theme::size::STEPPER;
/// The Duration field's draft key.
const SPEED_DURATION_DRAFT: &str = "speed_duration";
/// One stepper click in seconds. The reference shows no step, so this is a proposed value.
const SPEED_DURATION_STEP: f64 = 0.1;
/// The gap between the Reset footer and the panel's bottom edge. Proposed: unmeasured.
const SPEED_FOOTER_INSET: f32 = theme::space::SM;

/// A dashed line from `from` to `to` at height `y`, ending in a small open arrowhead. The dash,
/// gap and head sizes are proposed: the reference's arrow is not measured.
fn draw_dashed_arrow(painter: &egui::Painter, from: f32, to: f32, y: f32, color: Color32) {
    const DASH: f32 = 3.0;
    const GAP: f32 = 3.0;
    const HEAD: f32 = 4.0;
    let stroke = egui::Stroke::new(1.0, color);
    let mut x = from;
    while x + DASH <= to - HEAD {
        painter.line_segment([egui::pos2(x, y), egui::pos2(x + DASH, y)], stroke);
        x += DASH + GAP;
    }
    painter.line_segment(
        [egui::pos2(to - HEAD, y - HEAD / 2.0), egui::pos2(to, y)],
        stroke,
    );
    painter.line_segment(
        [egui::pos2(to - HEAD, y + HEAD / 2.0), egui::pos2(to, y)],
        stroke,
    );
}

/// A typed duration such as `4703.2s` in seconds; `None` for anything not a positive length.
fn parse_seconds(text: &str) -> Option<f64> {
    let value = text
        .trim()
        .trim_end_matches('s')
        .trim()
        .parse::<f64>()
        .ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}

impl Panel<'_, '_> {
    /// The Standard / Curve sub-tabs. Standard is the constant rate; Curve is the
    /// ramp editor. Returning to Standard from a curve resets to a constant 1x, one undo step.
    fn speed_tabs(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        has_curve: bool,
        maintain_pitch: bool,
    ) {
        let open = has_curve || self.state.speed_curve_view;
        let ids = [
            "inspector.speed.tab.standard".to_owned(),
            "inspector.speed.tab.curve".to_owned(),
        ];
        let labels = [self.tr(Text::SpeedStandard), self.tr(Text::SpeedCurveTab)];
        let cells = kit::segmented(ui, &ids, &labels, usize::from(open));
        for (id, cell) in ids.iter().zip(&cells) {
            self.remember(id.clone(), cell);
        }
        if let [standard, curve] = cells.as_slice() {
            if standard.clicked() && open {
                self.state.speed_curve_view = false;
                if has_curve {
                    self.set_speed(item, 1.0, maintain_pitch);
                }
            }
            if curve.clicked() && !open {
                self.state.speed_curve_view = true;
            }
        }
        ui.add_space(theme::space::XS);
    }
}
