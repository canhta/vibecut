//! Inserts adjustment layers at the playhead and applies saved colour-adjustment presets to
//! the selected visual clip.

use super::super::{
    BTreeSet, Command, CreatedId, ItemParams, NewItem, SourceRange, TICKS_PER_SECOND, Tick,
    error_detail,
};
use super::State;
use crate::editor::assets::{Result, Text};
use crate::editor::{Port, context};
use crate::i18n;

impl State {
    /// Insert one adjustment layer at the playhead, five seconds long as text and graphics
    /// are, and select it. The layer has no picture and filters nothing until an effect is
    /// added to it. A refused insert changes nothing.
    pub(in crate::editor) fn insert_adjustment_layer(&mut self, port: &mut Port<'_>) {
        if !port.read.insertion_available {
            return;
        }
        let result = (|| -> crate::Result<project::ItemId> {
            let start = port.playhead()?;
            let outcome = port.session.apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, Tick(5 * TICKS_PER_SECOND))?,
                    start,
                    content: ItemParams::Effect(project::EffectParams::default()),
                },
            })?;
            outcome
                .created
                .into_iter()
                .find_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                })
                .ok_or(crate::Error::Invalid("adjustment allocation"))
        })();
        match result {
            Ok(item) => {
                port.select(BTreeSet::from([item]));
                port.reset_selection(true);
            }
            Err(error) => {
                self.import_notice = Some(error_detail(&error));
                let text = i18n::text(port.locale(), Text::InsertFailed);
                port.toast(text, None);
            }
        }
    }

    /// Apply one app-owned preset to the selected video or image item as one
    /// undo step. Locked tracks and non-visual selections refuse with a toast.
    pub(in crate::editor) fn apply_adjustment_preset(&mut self, port: &mut Port<'_>, index: usize) {
        let Some(item) = context::single(port.session.selection()) else {
            let text = port.tr(Text::EffectSelectClip);
            port.toast(text, None);
            return;
        };
        let Some(preset) = port.read.preferences.adjustment_presets.get(index).cloned() else {
            return;
        };
        let project = port.session.project();
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
        let _ = port.apply_result(command);
    }

    /// Delete one app-owned preset. The preference write changes no project data.
    pub(in crate::editor) fn delete_adjustment_preset(
        &mut self,
        port: &mut Port<'_>,
        index: usize,
    ) -> Result<()> {
        let mut presets = port.read.preferences.adjustment_presets.clone();
        if index >= presets.len() {
            return Ok(());
        }
        presets.remove(index);
        port.store_preference("adjustment_presets", serde_json::to_value(presets)?);
        Ok(())
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
