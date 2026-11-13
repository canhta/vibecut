//! Inserts bundled stickers from the Stickers tab and keeps the recent list.

use super::super::stickers;
use super::super::{CreatedId, TICKS_PER_SECOND, Text, Tick};
use super::State;
use crate::editor::Port;
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
    pub(in crate::editor) fn insert_sticker(&mut self, port: &mut Port<'_>, sticker_id: &str) {
        if !port.read.insertion_available {
            return;
        }
        let at = match port.playhead() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(port.locale(), Text::InsertFailed);
                port.toast(text, None);
                return;
            }
        };
        let outcome = libraries::stickers::insert_sticker(
            &mut *port.session,
            libraries::stickers::native_store(),
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
            let text = i18n::text(port.locale(), Text::InsertFailed);
            port.toast(text, None);
            return;
        };
        port.select(BTreeSet::from([item]));
        port.reset_selection(true);
        self.remember_sticker(port, sticker_id);
    }

    /// Move one inserted id to the front of the saved recents. Stale saved ids drop out here.
    pub(in crate::editor) fn remember_sticker(&mut self, port: &mut Port<'_>, id: &str) {
        let saved = port.read.preferences.sticker_recents.clone();
        let mut recents = Recents::from_ids(stickers::known_ids(&saved));
        recents.record(id);
        port.store_preference("sticker_recents", serde_json::json!(recents.ids()));
    }

    /// Empty the saved recent list. Projects are untouched.
    pub(in crate::editor) fn clear_sticker_recents(&mut self, port: &mut Port<'_>) {
        port.store_preference("sticker_recents", serde_json::json!([]));
    }
}
