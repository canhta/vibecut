//! Stars and unstars catalogue tiles in the app preference; projects are untouched.

use super::State;
use crate::Result;
use crate::editor::Port;

impl State {
    /// Persist one star change through the preference document, validated before the write.
    pub(in crate::editor) fn toggle_favorite(
        &mut self,
        port: &mut Port<'_>,
        family: &str,
        id: &str,
        favorite: bool,
    ) -> Result<()> {
        let next = port
            .read
            .preferences
            .catalog_favorites
            .with(family, id, favorite)?;
        port.store_preference_result("catalog_favorites", serde_json::to_value(&next)?)
    }
}
