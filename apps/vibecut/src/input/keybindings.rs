//! Persisted binding overrides, import profiles and their validation.

use super::{Chord, ChordError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use thiserror::Error;

/// Longest single chord text accepted from storage or a recording UI.
pub const MAX_CHORD_TEXT: usize = 64;
/// Most bindings one action may have.
pub const MAX_BINDINGS_PER_ACTION: usize = 8;
/// Most binding entries a preference document may hold.
pub const MAX_ACTIONS: usize = 96;
/// Largest keybinding profile accepted by import.
pub const MAX_PROFILE_BYTES: usize = 16 * 1024;
/// Import/export profile format and version. Unsupported versions are refused.
pub const KEYBINDING_FORMAT: &str = "vibecut-keybindings";
pub const KEYBINDING_VERSION: u64 = 1;
/// App-local commands used by the shortcuts dialog (p1-parity/04).
pub const KEYBINDINGS_SET_COMMAND: &str = "ui.keybindings.set";
pub const KEYBINDINGS_RESET_COMMAND: &str = "ui.keybindings.reset";

/// Persisted binding overrides and per-action `isCustomized` markers. Defaults
/// and metadata stay in the window action table, so this document only names
/// deltas.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeybindingPreferences {
    pub bindings: BTreeMap<String, Vec<String>>,
    #[serde(rename = "isCustomized")]
    pub is_customized: BTreeSet<String>,
}

impl KeybindingPreferences {
    /// Structural bounds and parseability. Unknown action ids are retained so
    /// the dispatcher can report them instead of silently deleting user data.
    pub fn validate(&self) -> Result<(), KeybindingError> {
        if self.bindings.len() > MAX_ACTIONS || self.is_customized.len() > MAX_ACTIONS {
            return Err(KeybindingError::TooManyActions);
        }
        for (action, chords) in &self.bindings {
            if !valid_action_id(action) {
                return Err(KeybindingError::InvalidActionId(action.clone()));
            }
            if chords.len() > MAX_BINDINGS_PER_ACTION {
                return Err(KeybindingError::TooManyBindings(action.clone()));
            }
            for text in chords {
                Chord::parse(text).map_err(|error| KeybindingError::InvalidChord {
                    text: text.clone(),
                    error,
                })?;
            }
        }
        for action in &self.is_customized {
            if !valid_action_id(action) {
                return Err(KeybindingError::InvalidActionId(action.clone()));
            }
            if !self.bindings.contains_key(action) {
                return Err(KeybindingError::CustomizedWithoutBinding(action.clone()));
            }
        }
        Ok(())
    }

    /// Startup repair for a damaged binding map: drop only invalid entries and
    /// keep the rest of the user's preferences. Returns whether anything changed.
    pub fn sanitize(mut self) -> (Self, bool) {
        let mut changed = false;
        let mut bindings = BTreeMap::new();
        for (action, chords) in std::mem::take(&mut self.bindings) {
            if bindings.len() >= MAX_ACTIONS
                || !valid_action_id(&action)
                || chords.len() > MAX_BINDINGS_PER_ACTION
                || chords.iter().any(|text| Chord::parse(text).is_err())
            {
                changed = true;
                continue;
            }
            bindings.insert(action, chords);
        }
        self.bindings = bindings;
        let mut customized = BTreeSet::new();
        for action in std::mem::take(&mut self.is_customized) {
            if customized.len() >= MAX_ACTIONS
                || !valid_action_id(&action)
                || !self.bindings.contains_key(&action)
            {
                changed = true;
                continue;
            }
            customized.insert(action);
        }
        self.is_customized = customized;
        (self, changed)
    }

    /// Restore every action to its registry defaults.
    pub fn reset_all(&mut self) -> bool {
        let changed = !self.bindings.is_empty() || !self.is_customized.is_empty();
        self.bindings.clear();
        self.is_customized.clear();
        changed
    }
}

/// One of the five named binding profiles the shortcuts dialog offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum KeybindingPreset {
    #[default]
    One,
    Two,
    Three,
    Four,
    Five,
}

impl KeybindingPreset {
    /// Every preset, in dialog order.
    pub const ALL: [Self; 5] = [Self::One, Self::Two, Self::Three, Self::Four, Self::Five];

    /// The 1-based number shown in the dialog ("Shortcut 1" to "Shortcut 5").
    pub const fn number(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
            Self::Five => 5,
        }
    }
}

impl TryFrom<u8> for KeybindingPreset {
    type Error = KeybindingError;

    fn try_from(index: u8) -> Result<Self, Self::Error> {
        match index {
            0 => Ok(Self::One),
            1 => Ok(Self::Two),
            2 => Ok(Self::Three),
            3 => Ok(Self::Four),
            4 => Ok(Self::Five),
            _ => Err(KeybindingError::UnknownPreset(index)),
        }
    }
}

impl From<KeybindingPreset> for u8 {
    fn from(preset: KeybindingPreset) -> Self {
        preset.number() - 1
    }
}

/// The keybinding preference: five named binding profiles and the active one.
/// Reads and edits reach the active profile through `Deref`, so every rule on
/// [`KeybindingPreferences`] (defaults, conflicts, overrides) applies to it.
/// The other profiles are untouched by those edits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeybindingPresets {
    pub active: KeybindingPreset,
    pub profiles: [KeybindingPreferences; 5],
}

impl KeybindingPresets {
    /// The active profile.
    pub fn active(&self) -> &KeybindingPreferences {
        let [one, two, three, four, five] = &self.profiles;
        match self.active {
            KeybindingPreset::One => one,
            KeybindingPreset::Two => two,
            KeybindingPreset::Three => three,
            KeybindingPreset::Four => four,
            KeybindingPreset::Five => five,
        }
    }

    /// The active profile, for edits.
    pub fn active_mut(&mut self) -> &mut KeybindingPreferences {
        let [one, two, three, four, five] = &mut self.profiles;
        match self.active {
            KeybindingPreset::One => one,
            KeybindingPreset::Two => two,
            KeybindingPreset::Three => three,
            KeybindingPreset::Four => four,
            KeybindingPreset::Five => five,
        }
    }

    /// A named profile, whether or not it is active.
    pub fn profile(&self, preset: KeybindingPreset) -> &KeybindingPreferences {
        let [one, two, three, four, five] = &self.profiles;
        match preset {
            KeybindingPreset::One => one,
            KeybindingPreset::Two => two,
            KeybindingPreset::Three => three,
            KeybindingPreset::Four => four,
            KeybindingPreset::Five => five,
        }
    }

    /// Structural bounds of every profile.
    pub fn validate(&self) -> Result<(), KeybindingError> {
        let [one, two, three, four, five] = &self.profiles;
        for profile in [one, two, three, four, five] {
            profile.validate()?;
        }
        Ok(())
    }

    /// Startup repair: each profile keeps its valid entries. Returns whether
    /// anything changed.
    pub fn sanitize(self) -> (Self, bool) {
        let Self { active, profiles } = self;
        let sanitized = profiles.map(KeybindingPreferences::sanitize);
        let changed = sanitized.iter().any(|(_, changed)| *changed);
        let profiles = sanitized.map(|(profile, _)| profile);
        (Self { active, profiles }, changed)
    }
}

impl std::ops::Deref for KeybindingPresets {
    type Target = KeybindingPreferences;

    fn deref(&self) -> &Self::Target {
        self.active()
    }
}

impl std::ops::DerefMut for KeybindingPresets {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.active_mut()
    }
}

/// How an imported profile combines with the bindings already in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    /// Profile entries replace the same actions; other customizations stay.
    Merge,
    /// Every current customization is dropped; only the profile's entries remain.
    Replace,
}

/// A validated profile applied over the current bindings. Unknown actions are
/// left out of `preferences` and named here for the caller to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportOutcome {
    pub preferences: KeybindingPreferences,
    pub unknown_actions: Vec<String>,
}

impl KeybindingPreferences {
    /// Applies a profile that `KeybindingProfile::import` already validated.
    /// `known` answers whether an action id exists in the registry. The
    /// receiver is untouched, so a refusal leaves the current bindings in place.
    pub fn apply_profile(
        &self,
        profile: KeybindingPreferences,
        mode: ImportMode,
        known: impl Fn(&str) -> bool,
    ) -> Result<ImportOutcome, KeybindingError> {
        let mut next = match mode {
            ImportMode::Merge => self.clone(),
            ImportMode::Replace => Self::default(),
        };
        let mut unknown_actions = Vec::new();
        for (action, chords) in profile.bindings {
            if !known(action.as_str()) {
                unknown_actions.push(action);
                continue;
            }
            if profile.is_customized.contains(&action) {
                next.is_customized.insert(action.clone());
            } else {
                next.is_customized.remove(&action);
            }
            next.bindings.insert(action, chords);
        }
        next.validate()?;
        Ok(ImportOutcome {
            preferences: next,
            unknown_actions,
        })
    }
}

/// A portable binding profile for the import/export entry points in p1-parity/04.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeybindingProfile {
    pub format: String,
    pub version: u64,
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
    #[serde(rename = "isCustomized", default)]
    pub is_customized: BTreeSet<String>,
}

impl KeybindingProfile {
    pub fn from_preferences(preferences: KeybindingPreferences) -> Self {
        Self {
            format: KEYBINDING_FORMAT.to_owned(),
            version: KEYBINDING_VERSION,
            bindings: preferences.bindings,
            is_customized: preferences.is_customized,
        }
    }

    /// Import rejects unsupported schemas and oversized documents before any
    /// state changes. Unknown actions survive for the resolver to report.
    pub fn import(bytes: &[u8]) -> Result<KeybindingPreferences, KeybindingError> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(KeybindingError::TooLarge);
        }
        let profile: Self =
            serde_json::from_slice(bytes).map_err(|_| KeybindingError::Malformed)?;
        if profile.format != KEYBINDING_FORMAT {
            return Err(KeybindingError::WrongFormat(profile.format));
        }
        if profile.version != KEYBINDING_VERSION {
            return Err(KeybindingError::UnsupportedVersion {
                found: profile.version,
                expected: KEYBINDING_VERSION,
            });
        }
        let preferences = KeybindingPreferences {
            bindings: profile.bindings,
            is_customized: profile.is_customized,
        };
        preferences.validate()?;
        Ok(preferences)
    }

    pub fn export(preferences: KeybindingPreferences) -> Result<Vec<u8>, KeybindingError> {
        preferences.validate()?;
        let profile = Self::from_preferences(preferences);
        let bytes = serde_json::to_vec(&profile).map_err(|_| KeybindingError::Malformed)?;
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(KeybindingError::TooLarge);
        }
        Ok(bytes)
    }
}

/// Why binding storage or import was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum KeybindingError {
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("keybinding preset {0} does not exist")]
    UnknownPreset(u8),
    #[error("invalid chord `{text}`: {error}")]
    InvalidChord { text: String, error: ChordError },
    #[error("binding `{chord}` is already used by `{existing}`")]
    Conflict {
        chord: String,
        existing: String,
        action: String,
    },
    #[error("binding `{0}` is listed twice")]
    DuplicateBinding(String),
    #[error("too many bindings for `{0}`")]
    TooManyBindings(String),
    #[error("too many keybinding entries")]
    TooManyActions,
    #[error("`{0}` is marked customized without a binding entry")]
    CustomizedWithoutBinding(String),
    #[error("invalid action id `{0}`")]
    InvalidActionId(String),
    #[error("keybinding profile is too large")]
    TooLarge,
    #[error("keybinding profile format `{0}` is unsupported")]
    WrongFormat(String),
    #[error("keybinding profile version {found} is unsupported; expected {expected}")]
    UnsupportedVersion { found: u64, expected: u64 },
    #[error("keybinding profile is malformed")]
    Malformed,
}

/// A non-fatal binding problem reported by the resolver. Unknown actions are
/// kept in the document and surfaced here rather than silently ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeybindingIssue {
    UnknownAction(String),
    InvalidChord {
        action: String,
        text: String,
    },
    Conflict {
        chord: String,
        first: String,
        second: String,
    },
    Duplicate {
        action: String,
        chord: String,
    },
}

impl fmt::Display for KeybindingIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAction(action) => {
                write!(formatter, "unknown keybinding action `{action}`")
            }
            Self::InvalidChord { action, text } => {
                write!(formatter, "invalid chord `{text}` for `{action}`")
            }
            Self::Conflict {
                chord,
                first,
                second,
            } => write!(
                formatter,
                "chord `{chord}` is bound to both `{first}` and `{second}`"
            ),
            Self::Duplicate { action, chord } => {
                write!(formatter, "chord `{chord}` is listed twice for `{action}`")
            }
        }
    }
}

pub(crate) fn valid_action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
