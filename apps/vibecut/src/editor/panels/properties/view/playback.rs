//! Renders a clip's playback controls: the Audio tab's volume and fades, and the constant
//! speed rows. Mute lives on the track header and the clip menu.
use super::Panel;
use crate::editor::properties::workflow::loudness::{Failed, scope_target};
use crate::editor::properties::{
    AUDIO_GAIN_SCRUB, Action, Icon, ItemId, SPEED_SCRUB, ScrubSpec, Text, egui, read, theme,
};
use crate::i18n;
use crate::kit;
use engine::loudness::LoudnessResult;
use project::{LoudnessScope, LoudnessTarget};

/// The reference's fade range, in seconds.
const FADE_MAX_SECONDS: f64 = 10.0;
/// Drafts of the two loudness target fields, by field key.
const LUFS_DRAFT: &str = "loudness_target";
const PEAK_DRAFT: &str = "loudness_true_peak";
const LOUDNESS_FIELD_WIDTH: f32 = 96.0;
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
            let bar = kit::progress_bar(&mut value_ui, "inspector.loudness.progress", progress);
            self.remember("inspector.loudness.progress", &bar);
            let cancel = value_ui
                .push_id("inspector.loudness.cancel", |ui| {
                    ui.add(egui::Button::new(self.tr(Text::LoudnessCancel)))
                })
                .inner;
            self.remember("inspector.loudness.cancel", &cancel);
            if cancel.clicked() {
                self.actions.push(Action::CancelLoudness);
            }
        }
        for line in self.loudness_status(&frame) {
            ui.label(
                egui::RichText::new(line)
                    .size(theme::type_size::BODY)
                    .color(self.secondary()),
            );
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
        let mut next = self
            .state
            .drafts
            .get(draft)
            .cloned()
            .unwrap_or_else(|| format!("{shown}{suffix}"));
        let response = kit::value_field(
            &mut value_ui,
            id,
            &mut next,
            LOUDNESS_FIELD_WIDTH,
            None,
            self.tr(label),
        );
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
    /// the result for the current gains, or the unmeasured hint.
    fn loudness_status(&self, frame: &LoudnessFrame) -> Vec<String> {
        let locale = frame.locale;
        let tr = |key: Text| i18n::text(locale, key).to_owned();
        if let Some(progress) = frame.progress {
            let percent = format!("{:.0}", progress.clamp(0.0, 1.0) * 100.0);
            return vec![i18n::format(
                locale,
                Text::LoudnessMeasuring,
                &[("percent", &percent)],
            )];
        }
        match &frame.failed {
            Some(Failed::Stale) => return vec![tr(Text::LoudnessStale)],
            Some(Failed::Measurement(reason)) => {
                let reason = if reason.is_empty() {
                    tr(Text::LoudnessNoReason)
                } else {
                    reason.clone()
                };
                return vec![i18n::format(
                    locale,
                    Text::LoudnessFailed,
                    &[("reason", &reason)],
                )];
            }
            None => {}
        }
        if !self
            .typed_loudness(frame.shown)
            .is_some_and(|t| t.is_valid())
        {
            return vec![tr(Text::LoudnessInvalid)];
        }
        if !frame.clip_on && !frame.master_on {
            return vec![tr(Text::LoudnessNoScope)];
        }
        let Some(result) = &frame.result else {
            return vec![tr(Text::LoudnessUnmeasured)];
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
            lines.push(loudness_line(locale, Text::LoudnessClip, &detail));
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
            lines.push(loudness_line(locale, Text::LoudnessMaster, &detail));
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

    /// One fade edge's scrub contract: 0 to 10 s, capped at the room the clip
    /// leaves beside the other fade.
    fn fade_scrub(&self, item: ItemId, edge: project::FadeEdge) -> ScrubSpec {
        let room = read::fade_room_seconds(self.cx.session.project(), item, edge)
            .unwrap_or(FADE_MAX_SECONDS);
        let (field, label) = match edge {
            project::FadeEdge::In => ("audio_fade_in", Text::FadeIn),
            project::FadeEdge::Out => ("audio_fade_out", Text::FadeOut),
        };
        ScrubSpec {
            field,
            label,
            min: 0.0,
            max: room.min(FADE_MAX_SECONDS),
            step: 0.1,
            pixels_per_step: 8.0,
            decimals: 1,
            suffix: "s",
            scale: 1.0,
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
        self.scrub_field(ui, item, SPEED_SCRUB);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::SpeedDuration));
        if let Some((source, output)) = read::retime_seconds(project, item) {
            let response = value_ui.label(
                egui::RichText::new(format!("{source:.2}s → {output:.2}s"))
                    .size(theme::type_size::BODY)
                    .color(self.secondary()),
            );
            self.remember("inspector.speed.duration", &response);
        }
        value_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let reset = self.icon_only(
                ui,
                "inspector.speed.reset",
                Icon::ArrowCounterClockwise,
                self.tr(Text::SpeedReset),
                None,
                current != 1.0,
            );
            if reset {
                self.set_speed(item, 1.0, maintain_pitch);
            }
        });
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
            // "Change pitch" on lets pitch follow speed, so it is the inverse of maintain_pitch.
            let mut follows = !maintain_pitch;
            let response = value_ui.add(egui::Checkbox::without_text(&mut follows));
            let name = self.tr(Text::ChangePitch);
            response
                .ctx
                .accesskit_node_builder(response.id, |node| node.set_label(name));
            self.remember("inspector.speed.pitch", &response);
            if response.changed() {
                self.set_speed(item, current, !follows);
            }
        }
    }
}
