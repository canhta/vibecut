//! Own-drawn text bubbles and own-authored text effects, the two looks a text or caption item picks.

use serde::{Deserialize, Serialize};

/// A bubble shape painted behind text. Its colour follows the text fill (a light bubble behind
/// dark text, a dark one behind light text), so no colour is stored.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextBubble {
    Round,
    Pill,
    Oval,
    SpeechDown,
    SpeechLeft,
    Banner,
    Tag,
    Burst,
    Cloud,
    Ticket,
    Frame,
}

impl TextBubble {
    /// Every bubble in grid order.
    pub const ALL: [Self; 11] = [
        Self::Round,
        Self::Pill,
        Self::Oval,
        Self::SpeechDown,
        Self::SpeechLeft,
        Self::Banner,
        Self::Tag,
        Self::Burst,
        Self::Cloud,
        Self::Ticket,
        Self::Frame,
    ];
    /// The stable wire word.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Round => "round",
            Self::Pill => "pill",
            Self::Oval => "oval",
            Self::SpeechDown => "speech_down",
            Self::SpeechLeft => "speech_left",
            Self::Banner => "banner",
            Self::Tag => "tag",
            Self::Burst => "burst",
            Self::Cloud => "cloud",
            Self::Ticket => "ticket",
            Self::Frame => "frame",
        }
    }
}

/// A fill, outline, shadow and glow recipe that replaces the item's own while chosen; Reset
/// clears it.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextEffect {
    Neon,
    Pop,
    Comic,
    Hollow,
    Retro,
    Sticker,
    Fire,
    Ice,
    LongShadow,
    Gold,
    Soft,
    Glitch,
}

impl TextEffect {
    /// Every effect in grid order.
    pub const ALL: [Self; 12] = [
        Self::Neon,
        Self::Pop,
        Self::Comic,
        Self::Hollow,
        Self::Retro,
        Self::Sticker,
        Self::Fire,
        Self::Ice,
        Self::LongShadow,
        Self::Gold,
        Self::Soft,
        Self::Glitch,
    ];
    /// The stable wire word.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Neon => "neon",
            Self::Pop => "pop",
            Self::Comic => "comic",
            Self::Hollow => "hollow",
            Self::Retro => "retro",
            Self::Sticker => "sticker",
            Self::Fire => "fire",
            Self::Ice => "ice",
            Self::LongShadow => "long_shadow",
            Self::Gold => "gold",
            Self::Soft => "soft",
            Self::Glitch => "glitch",
        }
    }
}
