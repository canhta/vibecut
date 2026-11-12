//! Applies saved colour-adjustment presets to the selected visual clip.

use crate::editor::assets::{Editor, Result, Text};
use crate::editor::context;

impl Editor {
    /// Apply one app-owned preset to the selected video or image item as one
    /// undo step. Locked tracks and non-visual selections refuse with a toast.
    pub(in crate::editor) fn apply_adjustment_preset(&mut self, index: usize) {
        let Some(item) = context::single(&self.selection) else {
            let text = self.tr(Text::EffectSelectClip).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(preset) = self.preferences.adjustment_presets.get(index).cloned() else {
            return;
        };
        let project = self.session.project();
        let Some(target) = project
            .data()
            .scene()
            .timeline
            .iter()
            .find(|track| track.items.iter().any(|candidate| candidate.id == item))
            .and_then(|track| {
                track
                    .items
                    .iter()
                    .find(|candidate| candidate.id == item)
                    .map(|candidate| (track.locked, candidate))
            })
        else {
            return;
        };
        let (locked, target) = target;
        if locked || !matches!(target.content, project::ItemParams::Video(_)) {
            return;
        }
        let existing = target
            .effects
            .iter()
            .find(|effect| effect.kind == "color_grade")
            .map(|effect| effect.id.clone());
        let commands: Vec<engine::Command> = if let Some(effect) = existing {
            preset
                .params
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .update()
                        .map(|value| engine::Command::UpdateEffectParam {
                            item,
                            effect: effect.clone(),
                            key: key.clone(),
                            value,
                        })
                })
                .collect()
        } else {
            let Some(id) = next_effect_id(project, item) else {
                return;
            };
            let mut instance = match project::effect_definition("color_grade") {
                Ok(definition) => definition.instance(id),
                Err(_) => return,
            };
            instance.params = preset.params;
            vec![engine::Command::AddEffect {
                item,
                effect: instance,
            }]
        };
        let command = if commands.len() == 1 {
            match commands.into_iter().next() {
                Some(command) => command,
                None => return,
            }
        } else {
            engine::Command::Batch { commands }
        };
        let _ = self.desk().apply_result(command, false);
    }

    /// Delete one app-owned preset. The preference write changes no project data.
    pub(in crate::editor) fn delete_adjustment_preset(&mut self, index: usize) -> Result<()> {
        let mut presets = self.preferences.adjustment_presets.clone();
        if index >= presets.len() {
            return Ok(());
        }
        presets.remove(index);
        self.preferences
            .set("adjustment_presets", serde_json::to_value(presets)?)
    }
}

/// The next free colour-grade instance id on one item.
fn next_effect_id(project: &project::Project, item: project::ItemId) -> Option<String> {
    let effects = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|candidate| candidate.id == item)?
        .effects
        .iter()
        .map(|effect| effect.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    (1..=256).find_map(|index| {
        let candidate = if index == 1 {
            "color_grade".to_owned()
        } else {
            format!("color_grade{index}")
        };
        (candidate.len() <= 64 && !effects.contains(candidate.as_str())).then_some(candidate)
    })
}
