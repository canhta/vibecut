//! Renders a clip's playback controls: the Audio tab's volume and fades, and the constant
//! speed rows. Mute lives on the track header and the clip menu.
use super::Panel;
use crate::editor::properties::{
    AUDIO_GAIN_SCRUB, Action, Icon, ItemId, SPEED_SCRUB, ScrubSpec, Text, egui, read, theme,
};
use crate::kit;

/// The reference's fade range, in seconds.
const FADE_MAX_SECONDS: f64 = 10.0;

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
            },
        );
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

    /// Voice enhancement: one switch over the preset's cleanup effects, and while it is on a
    /// Strength row of three dry/wet mixes. The row is [new]: the reference capture has no
    /// equivalent. The preset's parameters stay ordinary chain effects reachable through
    /// `audio.*_cleanup_effect`.
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
        let levels = [
            ("light", Text::VoiceEnhancementLight, 40.0),
            ("medium", Text::VoiceEnhancementMedium, 70.0),
            ("full", Text::VoiceEnhancementFull, 100.0),
        ];
        let ids: Vec<String> = levels
            .iter()
            .map(|(name, _, _)| format!("inspector.voice_enhance.strength.{name}"))
            .collect();
        let labels: Vec<&str> = levels.iter().map(|(_, text, _)| self.tr(*text)).collect();
        let selected = levels
            .iter()
            .position(|(_, _, wet)| strength.is_some_and(|mix| (mix - wet).abs() < 0.5))
            .unwrap_or(usize::MAX);
        let cells = kit::segmented(&mut value_ui, &ids, &labels, selected);
        for (index, ((id, cell), (_, _, wet_percent))) in
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
        let segments = read::speed_segments(project, item);
        self.speed_tabs(ui, item, segments.is_some(), maintain_pitch);
        if segments.is_some() || self.state.speed_curve_view {
            self.speed_curve_controls(ui, item, segments, maintain_pitch, has_audio);
            return;
        }
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

/// Automation word and label of a speed ramp preset.
fn preset_entry(preset: project::SpeedPreset) -> (&'static str, Text) {
    match preset {
        project::SpeedPreset::SlowInFastOut => ("slow_in_fast_out", Text::SpeedPresetSlowInFastOut),
        project::SpeedPreset::FastInSlowOut => ("fast_in_slow_out", Text::SpeedPresetFastInSlowOut),
        project::SpeedPreset::SlowMiddle => ("slow_middle", Text::SpeedPresetSlowMiddle),
        project::SpeedPreset::FastMiddle => ("fast_middle", Text::SpeedPresetFastMiddle),
    }
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

    /// The Curve view: ramp presets, then one speed field per stretch of the
    /// item's curve. Every commit rewrites the whole curve through one command.
    fn speed_curve_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        segments: Option<Vec<project::SpeedSegment>>,
        maintain_pitch: bool,
        has_audio: bool,
    ) {
        // Two equal cells per row, measured from the full width once so every cell
        // in the grid shares one width and the columns line up across rows.
        let gap = theme::space::XS;
        let cell = (ui.available_width() - gap) / 2.0;
        for pair in project::SpeedPreset::ALL.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for preset in pair {
                    let (word, label) = preset_entry(*preset);
                    let selected = segments.as_deref() == Some(preset.segments().as_slice());
                    let response = ui.add_sized(
                        [cell, theme::size::CONTROL],
                        egui::Button::new(self.tr(label)).selected(selected),
                    );
                    self.remember(format!("inspector.speed.preset.{word}"), &response);
                    if response.clicked() && !selected {
                        self.actions.push(Action::ApplySpeedPreset {
                            item,
                            preset: *preset,
                            maintain_pitch,
                        });
                    }
                }
            });
        }
        let Some(segments) = segments else {
            return;
        };
        ui.add_space(theme::space::XS);
        let pairs = |segments: &[project::SpeedSegment]| -> Vec<(u16, f64)> {
            segments
                .iter()
                .map(|stretch| (stretch.span, stretch.speed.as_f64()))
                .collect()
        };
        let mut start = 0u32;
        for (index, segment) in segments.iter().enumerate() {
            let end = start + u32::from(segment.span);
            let label = format!(
                "{:.0}-{:.0}%",
                f64::from(start) / 10.0,
                f64::from(end) / 10.0
            );
            start = end;
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, &label);
            let before = segment.speed.as_f64();
            let mut value = before;
            let response = value_ui.add(
                egui::DragValue::new(&mut value)
                    .range(0.0..=10.0)
                    .speed(0.01)
                    .max_decimals(3),
            );
            self.remember(format!("inspector.speed.segment.{index}"), &response);
            if ((response.changed() && !response.dragged()) || response.drag_stopped())
                && (value - before).abs() > f64::EPSILON
            {
                let mut edited = pairs(&segments);
                if let Some(slot) = edited.get_mut(index) {
                    slot.1 = value;
                }
                self.actions.push(Action::SetSpeedCurve {
                    item,
                    segments: edited,
                    maintain_pitch,
                });
            }
        }
        if has_audio {
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::ChangePitch));
            let mut follows = !maintain_pitch;
            let response = value_ui.add(egui::Checkbox::without_text(&mut follows));
            let name = self.tr(Text::ChangePitch);
            response
                .ctx
                .accesskit_node_builder(response.id, |node| node.set_label(name));
            self.remember("inspector.speed.pitch", &response);
            if response.changed() {
                self.actions.push(Action::SetSpeedCurve {
                    item,
                    segments: pairs(&segments),
                    maintain_pitch: !follows,
                });
            }
        }
    }
}
