//! The graph editor's persisted section: popover tab and saved easing presets.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which preset list the graph popover shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EasingTab {
    #[default]
    Presets,
    Saved,
}

/// One user preset: a name and its normalized `[x1, y1, x2, y2]` curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedEasing {
    pub name: String,
    pub curve: [f64; 4],
}

/// The popover's persisted tab and the user's saved curves, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EasingPreferences {
    pub tab: EasingTab,
    pub saved: Vec<SavedEasing>,
}
pub const MAX_SAVED_EASINGS: usize = 64;
const MAX_EASING_NAME_BYTES: usize = 64;

impl EasingPreferences {
    pub(super) fn validate(&self) -> Result<()> {
        if self.saved.len() > MAX_SAVED_EASINGS
            || self
                .saved
                .iter()
                .any(|preset| preset.name.is_empty() || preset.name.len() > MAX_EASING_NAME_BYTES)
            || self
                .saved
                .iter()
                .map(|preset| preset.name.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                != self.saved.len()
            || self
                .saved
                .iter()
                .any(|preset| project::EasingCurve(preset.curve).validate().is_err())
        {
            return Err(Error::Invalid("saved easing presets"));
        }
        Ok(())
    }
    /// The next free `Custom N` name, as the reference numbers saved curves.
    pub fn next_name(&self) -> String {
        (self.saved.len() + 1..)
            .map(|n| format!("Custom {n}"))
            .find(|name| self.saved.iter().all(|preset| &preset.name != name))
            .unwrap_or_default()
    }
}
