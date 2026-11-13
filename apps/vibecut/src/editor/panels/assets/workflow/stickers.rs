//! Inserts bundled stickers from the Stickers tab and keeps the recent list.

use super::super::{CreatedId, SelectionScopes, TICKS_PER_SECOND, Text, Tick};
use super::State;
use crate::editor::Desk;
use crate::i18n;
use engine::Placement;
use libraries::stickers::Recents;
use std::collections::BTreeSet;

/// Stickers take the app's five-second insert length, as graphics and text do.
const STICKER_DURATION: Tick = Tick(5 * TICKS_PER_SECOND);

impl State {
    /// Insert one bundled sticker at the playhead through the same catalog edit `stickers.insert`
    /// applies, select it and move it to the front of the recent list. A refused edit records
    /// nothing.
    pub(in crate::editor) fn insert_sticker(&mut self, desk: &mut Desk<'_>, sticker_id: &str) {
        if !desk.editor().insertion_available() {
            return;
        }
        let at = match desk.editor().playback.position() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(desk.editor().preferences.locale, Text::InsertFailed);
                desk.toast(text, None);
                return;
            }
        };
        let outcome = libraries::stickers::insert_sticker(
            &mut desk.editor().session,
            sticker_id,
            Placement::Auto,
            at,
            STICKER_DURATION,
        );
        let created = outcome.ok().and_then(|outcome| {
            outcome.created.into_iter().find_map(|id| match id {
                CreatedId::Item(item) => Some(item),
                _ => None,
            })
        });
        let Some(item) = created else {
            tracing::warn!(category = "sticker_insert", "insertion failed");
            let text = i18n::text(desk.editor().preferences.locale, Text::InsertFailed);
            desk.toast(text, None);
            return;
        };
        desk.select(BTreeSet::from([item]));
        desk.editor().selection_scopes = SelectionScopes::default();
        desk.editor().properties.clear_drafts();
        let mut recents = Recents::from_ids(desk.editor().preferences.sticker_recents.clone());
        recents.record(sticker_id);
        if let Err(error) =
            desk.store_preference("sticker_recents", serde_json::json!(recents.ids()))
        {
            desk.fail(error.to_string());
        }
    }

    /// Empty the saved recent list. Projects are untouched.
    pub(in crate::editor) fn clear_sticker_recents(&mut self, desk: &mut Desk<'_>) {
        if let Err(error) = desk.store_preference("sticker_recents", serde_json::json!([])) {
            desk.fail(error.to_string());
        }
    }
}
