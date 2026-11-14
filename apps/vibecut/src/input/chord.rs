//! Canonical chord text, modifier sets and per-platform display glyphs.

use crate::egui;
use std::fmt;
use thiserror::Error;

/// Longest single chord text accepted from storage or a recording UI.
pub const MAX_CHORD_TEXT: usize = 64;

/// The platform decides display glyphs. Mapping is compile-time in production
/// and explicit in tests, so macOS and Windows behaviour is checked on Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub const fn is_mac(self) -> bool {
        matches!(self, Self::Mac)
    }
}

/// The portable modifier set. `command` is Cmd on macOS and Ctrl elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mods {
    pub command: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key with modifiers. Chords compare and persist by canonical text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: egui::Key,
}

impl Chord {
    pub const fn new(mods: Mods, key: egui::Key) -> Self {
        Self { mods, key }
    }

    pub const fn plain(key: egui::Key) -> Self {
        Self {
            mods: Mods {
                command: false,
                alt: false,
                shift: false,
            },
            key,
        }
    }

    /// Reads canonical text and common aliases: modifier words in any order and
    /// case, macOS glyphs (`⌘`, `⌥`, `⇧`), glued or with `+`, and the key names
    /// [`egui::Key`] reads. `Ctrl++` is the Plus key.
    pub fn parse(text: &str) -> Result<Self, ChordError> {
        if text.len() > MAX_CHORD_TEXT {
            return Err(ChordError::TooLong);
        }
        let mut rest = text.trim();
        if rest.is_empty() {
            return Err(ChordError::Empty);
        }
        let mut mods = Mods::default();
        loop {
            if let Some((modifier, after)) = strip_glyph(rest) {
                add_modifier(&mut mods, modifier, rest)?;
                rest = match after.strip_prefix('+') {
                    Some(tail) if !tail.is_empty() => tail,
                    _ => after,
                };
                if rest.trim().is_empty() {
                    return Err(ChordError::MissingKey);
                }
                continue;
            }
            let first = rest.chars().next().map_or(0, char::len_utf8);
            let separator = rest
                .get(first..)
                .and_then(|tail| tail.find('+'))
                .map(|index| index + first);
            match separator {
                Some(index) => {
                    let token = rest.get(..index).unwrap_or_default().trim();
                    let after = rest.get(index + 1..).unwrap_or_default();
                    if after.trim().is_empty() {
                        return Err(ChordError::MissingKey);
                    }
                    let modifier = modifier_named(token)
                        .ok_or_else(|| ChordError::UnknownModifier(token.to_owned()))?;
                    add_modifier(&mut mods, modifier, token)?;
                    rest = after;
                }
                None => {
                    let token = rest.trim();
                    if modifier_named(token).is_some() {
                        return Err(ChordError::MissingKey);
                    }
                    let key =
                        parse_key(token).ok_or_else(|| ChordError::UnknownKey(token.to_owned()))?;
                    return Ok(Self { mods, key });
                }
            }
        }
    }

    /// Canonical text: `Ctrl+Alt+Shift+Key`.
    pub fn canonical(self) -> String {
        self.to_string()
    }

    /// Menus and the shortcuts dialog: `⌥⇧⌘K` on macOS, `Ctrl+Alt+Shift+K`
    /// elsewhere. The canonical chord never changes with the platform.
    pub fn display(self, platform: Platform) -> String {
        let mut text = String::new();
        if platform.is_mac() {
            if self.mods.alt {
                text.push('⌥');
            }
            if self.mods.shift {
                text.push('⇧');
            }
            if self.mods.command {
                text.push('⌘');
            }
            text.push_str(mac_key(self.key));
        } else {
            if self.mods.command {
                text.push_str("Ctrl+");
            }
            if self.mods.alt {
                text.push_str("Alt+");
            }
            if self.mods.shift {
                text.push_str("Shift+");
            }
            text.push_str(self.key.symbol_or_name());
        }
        text
    }

    /// Per-key display tokens for the shortcuts dialog. macOS uses the
    /// reference glyphs; other platforms spell modifiers out. Named keys such
    /// as Space and Esc stay word tokens so the UI can draw them as key caps.
    pub fn parts(self, platform: Platform) -> Vec<String> {
        let mut parts = Vec::with_capacity(4);
        if self.mods.command {
            parts.push(if platform.is_mac() {
                "⌘".to_owned()
            } else {
                "Ctrl".to_owned()
            });
        }
        if self.mods.alt {
            parts.push(if platform.is_mac() {
                "⌥".to_owned()
            } else {
                "Alt".to_owned()
            });
        }
        if self.mods.shift {
            parts.push(if platform.is_mac() {
                "⇧".to_owned()
            } else {
                "Shift".to_owned()
            });
        }
        parts.push(match (platform.is_mac(), self.key) {
            (_, egui::Key::ArrowDown) => "↓".to_owned(),
            (_, egui::Key::ArrowLeft) => "←".to_owned(),
            (_, egui::Key::ArrowRight) => "→".to_owned(),
            (_, egui::Key::ArrowUp) => "↑".to_owned(),
            (true, egui::Key::Escape) => "esc".to_owned(),
            (true, egui::Key::Tab) => "⇥".to_owned(),
            (true, egui::Key::Backspace) => "⌫".to_owned(),
            (true, egui::Key::Delete) => "⌦".to_owned(),
            (false, egui::Key::Escape) => "Esc".to_owned(),
            _ => self.key.symbol_or_name().to_owned(),
        });
        parts
    }

    /// Exact match, unlike egui's `consume_key` which ignores extra Shift/Alt.
    /// On macOS a physical Control key is never folded into Command.
    pub fn matches(self, platform: Platform, modifiers: egui::Modifiers) -> bool {
        if self.mods.command != modifiers.command
            || self.mods.alt != modifiers.alt
            || self.mods.shift != modifiers.shift
        {
            return false;
        }
        if platform.is_mac() && modifiers.ctrl {
            return false;
        }
        true
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.command {
            formatter.write_str("Ctrl+")?;
        }
        if self.mods.alt {
            formatter.write_str("Alt+")?;
        }
        if self.mods.shift {
            formatter.write_str("Shift+")?;
        }
        formatter.write_str(self.key.name())
    }
}

impl std::str::FromStr for Chord {
    type Err = ChordError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

#[derive(Clone, Copy)]
enum Modifier {
    Command,
    Alt,
    Shift,
}

fn modifier_named(name: &str) -> Option<Modifier> {
    const NAMES: &[(&str, Modifier)] = &[
        ("cmd", Modifier::Command),
        ("command", Modifier::Command),
        ("meta", Modifier::Command),
        ("super", Modifier::Command),
        ("mod", Modifier::Command),
        ("ctrl", Modifier::Command),
        ("control", Modifier::Command),
        ("ctl", Modifier::Command),
        ("⌘", Modifier::Command),
        ("⌃", Modifier::Command),
        ("alt", Modifier::Alt),
        ("opt", Modifier::Alt),
        ("option", Modifier::Alt),
        ("⌥", Modifier::Alt),
        ("shift", Modifier::Shift),
        ("⇧", Modifier::Shift),
    ];
    NAMES
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
        .map(|(_, modifier)| *modifier)
}

fn strip_glyph(text: &str) -> Option<(Modifier, &str)> {
    let mut chars = text.chars();
    let modifier = match chars.next()? {
        '⌘' | '⌃' => Modifier::Command,
        '⌥' => Modifier::Alt,
        '⇧' => Modifier::Shift,
        _ => return None,
    };
    Some((modifier, chars.as_str()))
}

fn add_modifier(mods: &mut Mods, modifier: Modifier, written: &str) -> Result<(), ChordError> {
    let slot = match modifier {
        Modifier::Command => &mut mods.command,
        Modifier::Alt => &mut mods.alt,
        Modifier::Shift => &mut mods.shift,
    };
    if *slot {
        return Err(ChordError::DuplicateModifier(written.to_owned()));
    }
    *slot = true;
    Ok(())
}

fn parse_key(name: &str) -> Option<egui::Key> {
    let alias = match name.to_ascii_lowercase().as_str() {
        "esc" => Some(egui::Key::Escape),
        "return" => Some(egui::Key::Enter),
        "del" => Some(egui::Key::Delete),
        "pgup" => Some(egui::Key::PageUp),
        "pgdn" => Some(egui::Key::PageDown),
        "spacebar" => Some(egui::Key::Space),
        "ins" => Some(egui::Key::Insert),
        "arrowup" => Some(egui::Key::ArrowUp),
        "arrowdown" => Some(egui::Key::ArrowDown),
        "arrowleft" => Some(egui::Key::ArrowLeft),
        "arrowright" => Some(egui::Key::ArrowRight),
        _ => None,
    };
    alias.or_else(|| egui::Key::from_name(name)).or_else(|| {
        egui::Key::ALL
            .iter()
            .copied()
            .find(|key| key.name().eq_ignore_ascii_case(name))
    })
}

fn mac_key(key: egui::Key) -> &'static str {
    match key {
        egui::Key::ArrowDown => "↓",
        egui::Key::ArrowLeft => "←",
        egui::Key::ArrowRight => "→",
        egui::Key::ArrowUp => "↑",
        egui::Key::Escape => "⎋",
        egui::Key::Tab => "⇥",
        egui::Key::Backspace => "⌫",
        egui::Key::Enter => "⏎",
        egui::Key::Delete => "⌦",
        other => other.symbol_or_name(),
    }
}

/// Why a chord was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ChordError {
    #[error("empty shortcut")]
    Empty,
    #[error("shortcut longer than {MAX_CHORD_TEXT} bytes")]
    TooLong,
    #[error("shortcut has no key")]
    MissingKey,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("unknown modifier `{0}`")]
    UnknownModifier(String),
    #[error("modifier `{0}` given twice")]
    DuplicateModifier(String),
}
