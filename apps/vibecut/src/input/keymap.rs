//! Resolved chords, non-fatal binding issues and global key dispatch.

use super::{Chord, DispatchContext};
use crate::egui;
use std::collections::BTreeMap;

/// What dispatch needs to know about one action without naming its payload.
#[derive(Clone, Copy, Debug)]
pub struct DispatchInfo {
    pub bindable: bool,
    pub available: bool,
    pub owner: Option<&'static str>,
    pub requires_clipboard: bool,
}

/// Resolved chords, their action metadata and the non-fatal problems found
/// while resolving them.
#[derive(Clone, Debug)]
pub struct Keymap {
    pub(crate) bindings: BTreeMap<Chord, &'static str>,
    pub(crate) specs: BTreeMap<&'static str, DispatchInfo>,
    pub(crate) issues: Vec<super::KeybindingIssue>,
}

impl Keymap {
    pub(crate) fn new(
        bindings: BTreeMap<Chord, &'static str>,
        specs: BTreeMap<&'static str, DispatchInfo>,
        issues: Vec<super::KeybindingIssue>,
    ) -> Self {
        Self {
            bindings,
            specs,
            issues,
        }
    }

    pub fn issues(&self) -> &[super::KeybindingIssue] {
        &self.issues
    }

    pub fn take(&self, input: &mut egui::InputState, context: DispatchContext) -> super::Dispatch {
        self.take_on_platform(input, context, super::Platform::current())
    }

    pub(crate) fn take_on_platform(
        &self,
        input: &mut egui::InputState,
        context: DispatchContext,
        platform: super::Platform,
    ) -> super::Dispatch {
        use super::{Dispatch, Mods};
        if let Some(reason) = context.blocked() {
            return Dispatch::Blocked(reason);
        }
        let mut found = None;
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            {
                let chord = Chord {
                    mods: Mods {
                        command: modifiers.command,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                    },
                    key: *key,
                };
                if chord.matches(platform, *modifiers)
                    && let Some(action) = self.bindings.get(&chord).copied()
                {
                    found = Some((*key, chord, action));
                    break;
                }
            }
        }
        let Some((key, chord, action)) = found else {
            return Dispatch::NoMatch;
        };
        let Some(spec) = self.specs.get(action).filter(|spec| spec.bindable) else {
            return Dispatch::NoMatch;
        };
        if spec.requires_clipboard && !context.clipboard_entry {
            return Dispatch::FellThrough(action);
        }
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key {
                    key: pressed_key,
                    pressed: true,
                    modifiers,
                    ..
                } if *pressed_key == key && chord.matches(platform, *modifiers)
            )
        });
        if spec.available {
            Dispatch::Action { action, chord }
        } else {
            Dispatch::Unavailable {
                action,
                owner: spec.owner.unwrap_or("another ticket"),
            }
        }
    }
}
