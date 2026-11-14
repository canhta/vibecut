//! Applies the Text tab's choices: a look onto the selected text items, a template as a new item.
use super::super::{CreatedId, TICKS_PER_SECOND, Text, Tick};
use super::State;
use crate::editor::Port;
use crate::i18n;
use std::collections::BTreeSet;

/// A template item is five seconds long, as new text, graphics and stickers are.
const TEMPLATE_DURATION: Tick = Tick(5 * TICKS_PER_SECOND);

impl State {
    /// Copy one built-in look onto the selected text items as one undo step
    /// (`text.apply_preset`). With no text item selected it explains what to select and
    /// changes nothing; a refusal (a locked track) is already on the error line.
    pub(in crate::editor) fn apply_text_look(&mut self, port: &mut Port<'_>, look: project::TextLook) {
        let items: Vec<project::ItemId> = port
            .session
            .selection()
            .iter()
            .copied()
            .filter(|id| {
                port.session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| &track.items)
                    .any(|item| {
                        item.id == *id && matches!(item.content, project::ItemParams::Text(_))
                    })
            })
            .collect();
        if items.is_empty() {
            let text = port.tr(Text::TextLibrarySelectText);
            port.toast(text, None);
            return;
        }
        let _ = port.apply_result(engine::Command::ApplyTextPreset {
            items,
            preset: engine::TextPresetApply::Look { look },
        });
    }

    /// Insert one built-in template at the playhead on a track the engine places, as one undo
    /// step (`text.insert_template`), and select the new item. The words are the template's
    /// sample in the interface language.
    pub(in crate::editor) fn insert_text_template(
        &mut self,
        port: &mut Port<'_>,
        template: project::TextTemplate,
    ) {
        if !port.read.insertion_available {
            return;
        }
        let Ok(at) = port.playhead() else {
            let text = i18n::text(port.locale(), Text::TextInsertFailed);
            port.toast(text, None);
            return;
        };
        let content = i18n::text(port.locale(), i18n::text_template_sample(template));
        // A refusal is already on the error line through `apply_result`.
        let Ok(outcome) = port.apply_result(engine::Command::InsertTextTemplate {
            placement: engine::Placement::Auto,
            template,
            content: Some(content.into()),
            start: at,
            duration: TEMPLATE_DURATION,
        }) else {
            return;
        };
        let created = outcome.created.into_iter().find_map(|id| match id {
            CreatedId::Item(item) => Some(item),
            _ => None,
        });
        let Some(item) = created else {
            return;
        };
        port.select(BTreeSet::from([item]));
        port.reset_selection(true);
    }
}
