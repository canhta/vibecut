//! Dispatch blocking reasons and the outcome of one key press.

use super::Chord;

/// Why global dispatch is inert. The editor still lets the focused widget see
/// the key event so text editing and Escape-to-blur keep working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedReason {
    Overlay,
    ProjectLoading,
    Recording,
    TextInput,
}

/// Everything outside the keymap that can block global dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DispatchContext {
    pub overlay_open: bool,
    pub project_loading: bool,
    pub recording: bool,
    pub text_input: bool,
    /// The in-app clipboard has an item or keyframe entry. Presence, not payload,
    /// is the dispatch contract; p1-parity/20 owns the payload.
    pub clipboard_entry: bool,
}

impl DispatchContext {
    pub fn blocked(&self) -> Option<BlockedReason> {
        if self.overlay_open {
            Some(BlockedReason::Overlay)
        } else if self.project_loading {
            Some(BlockedReason::ProjectLoading)
        } else if self.recording {
            Some(BlockedReason::Recording)
        } else if self.text_input {
            Some(BlockedReason::TextInput)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Blocked(BlockedReason),
    NoMatch,
    FellThrough(&'static str),
    Unavailable {
        action: &'static str,
        owner: &'static str,
    },
    Action {
        action: &'static str,
        chord: Chord,
    },
}

#[derive(Clone)]
struct TextRequest {
    target: crate::egui::Id,
    owner: crate::egui::Id,
    issued: f64,
    frame: u64,
    stale: bool,
}

/// Bridge raw native clipboard chords before widgets in each viewport.
/// The locked eframe Wgpu integration handles RequestPaste synchronously after painting
/// and queues its result for that viewport's next frame. No system clipboard is accessed
/// here. Pending ownership survives focus invalidation until that result is classified.
/// An empty native result completes at the next frame too; egui discard passes do not.
pub fn text_clipboard(ctx: &crate::egui::Context, owner: crate::egui::Id) {
    text_clipboard_for(ctx, owner, super::Platform::current());
}
fn text_clipboard_for(
    ctx: &crate::egui::Context,
    owner: crate::egui::Id,
    platform: super::Platform,
) {
    use crate::egui::{Event, Key};
    let id = crate::egui::Id::unique(("input.text_clipboard", ctx.viewport_id()));
    let frame = ctx.cumulative_frame_nr();
    let focus = ctx
        .memory(|m| m.focused())
        .filter(|_| ctx.text_edit_focused());
    let mut pending = ctx.data_mut(|data| data.get_temp::<TextRequest>(id));
    let (time, focused, pointer, closing) = ctx.input(|i| {
        (
            i.time,
            i.focused,
            i.pointer.any_pressed(),
            i.viewport().close_requested(),
        )
    });
    if let Some(p) = &mut pending {
        p.stale |= focus != Some(p.target)
            || owner != p.owner
            || !focused
            || pointer
            || closing
            || time - p.issued >= 2.0;
    }
    let mut request = false;
    ctx.input_mut(|input| {
        let events = std::mem::take(&mut input.events);
        for event in events {
            if matches!(&event, Event::Paste(_) | Event::PasteImage(_))
                && pending.as_ref().is_some_and(|p| p.stale)
            {
                // Classify the owned response before a replacement TextEdit can see it.
                pending = None;
                continue;
            }
            if matches!(&event, Event::Paste(_) | Event::PasteImage(_)) {
                pending = None;
            }
            if let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
                && focus.is_some()
                && focused
            {
                let command = modifiers.command_only();
                let windows = platform == super::Platform::Windows;
                let copy = *key == Key::Copy
                    || (command && *key == Key::C)
                    || (windows && modifiers.ctrl && *key == Key::Insert);
                let cut = *key == Key::Cut
                    || (command && *key == Key::X)
                    || (windows && modifiers.shift && *key == Key::Delete);
                let paste = *key == Key::Paste
                    || (command && *key == Key::V)
                    || (windows && modifiers.shift && *key == Key::Insert);
                if copy {
                    input.events.push(Event::Copy);
                    continue;
                }
                if cut {
                    input.events.push(Event::Cut);
                    continue;
                }
                if paste {
                    if pending.is_none()
                        && let Some(target) = focus
                    {
                        pending = Some(TextRequest {
                            target,
                            owner,
                            issued: time,
                            frame,
                            stale: false,
                        });
                        request = true;
                    }
                    continue;
                }
            }
            input.events.push(event);
        }
    });
    if pending.as_ref().is_some_and(|p| p.frame < frame) {
        pending = None;
    }
    ctx.data_mut(|data| {
        if let Some(pending) = pending {
            data.insert_temp(id, pending);
        } else {
            data.remove::<TextRequest>(id);
        }
    });
    if request {
        ctx.send_viewport_cmd(crate::egui::ViewportCommand::RequestPaste);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        egui::{self, Event, Key},
        input::Platform,
    };
    fn key(key: Key, modifiers: egui::Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    #[test]
    fn text_clipboard_uses_real_fields_and_drops_retargeted_expired_responses() {
        for platform in [Platform::Mac, Platform::Windows, Platform::Linux] {
            for viewport in [
                egui::ViewportId::ROOT,
                egui::ViewportId::from_hash_of("editor"),
                egui::ViewportId::from_hash_of("shorts"),
            ] {
                let ctx = egui::Context::default();
                let mut value = "Tiếng Việt".to_owned();
                let field = egui::Id::unique("field");
                let owner = egui::Id::unique("session.scene");
                let mut frame =
                    |events: Vec<Event>, time: f64, owner: egui::Id, field: egui::Id| {
                        let mut raw = egui::RawInput {
                            viewport_id: viewport,
                            events,
                            time: Some(time),
                            focused: true,
                            ..Default::default()
                        };
                        raw.viewports.entry(viewport).or_default();
                        let mut output = ctx.run_ui(raw, |ui| {
                            text_clipboard_for(ui.ctx(), owner, platform);
                            ui.add(egui::TextEdit::singleline(&mut value).id(field))
                                .request_focus();
                        });
                        output.textures_delta.clear();
                        output
                    };
                frame(vec![], 0.0, owner, field);
                let output = frame(
                    vec![
                        key(Key::A, egui::Modifiers::COMMAND),
                        key(Key::C, egui::Modifiers::COMMAND),
                    ],
                    0.1,
                    owner,
                    field,
                );
                assert!(
                    output
                        .platform_output
                        .commands
                        .iter()
                        .any(|c| matches!(c,egui::OutputCommand::CopyText(t) if t=="Tiếng Việt"))
                );
                let output = frame(
                    vec![
                        key(Key::V, egui::Modifiers::COMMAND),
                        key(Key::V, egui::Modifiers::COMMAND),
                    ],
                    0.2,
                    owner,
                    field,
                );
                assert_eq!(
                    output.viewport_output[&viewport]
                        .commands
                        .iter()
                        .filter(|c| matches!(c, egui::ViewportCommand::RequestPaste))
                        .count(),
                    1
                );
                frame(
                    vec![Event::Paste(r#"{"version":1}"#.into())],
                    0.3,
                    owner,
                    field,
                );
                // Stale owner is rejected before a replacement field consumes it.
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    0.4,
                    owner,
                    field,
                );
                frame(
                    vec![Event::Paste("stale".into())],
                    0.5,
                    egui::Id::unique("new scene"),
                    field,
                );
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    0.6,
                    owner,
                    field,
                );
                frame(vec![Event::Paste("expired".into())], 3.0, owner, field);
                // A genuine widget/context-menu Paste has no pending request.
                frame(vec![Event::Paste(" + menu".into())], 3.1, owner, field);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.2,
                    owner,
                    field,
                );
                let replacement = egui::Id::unique("replacement field");
                ctx.memory_mut(|m| m.request_focus(replacement));
                frame(
                    vec![Event::Paste("wrong focus".into())],
                    3.3,
                    owner,
                    replacement,
                );
                frame(
                    vec![
                        Event::Ime(egui::ImeEvent::Preedit {
                            text: "Việt".into(),
                            active_range_chars: None,
                        }),
                        Event::Ime(egui::ImeEvent::Commit("Việt".into())),
                    ],
                    3.4,
                    owner,
                    replacement,
                );
                drop(frame);
                assert_eq!(value, "{\"version\":1} + menuViệt");
            }
        }
    }
    #[test]
    fn raw_nontext_paste_never_requests_native_data_and_windows_aliases_edit_text() {
        for platform in [Platform::Mac, Platform::Windows, Platform::Linux] {
            let ctx = egui::Context::default();
            let owner = egui::Id::unique("owner");
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![key(Key::V, egui::Modifiers::COMMAND)],
                    ..Default::default()
                },
                |ui| {
                    text_clipboard_for(ui.ctx(), owner, platform);
                    assert!(ui.input(|i| {
                        i.events
                            .iter()
                            .any(|e| matches!(e, Event::Key { key: Key::V, .. }))
                    }));
                },
            );
            output.textures_delta.clear();
            assert!(
                !output
                    .viewport_output
                    .values()
                    .flat_map(|o| &o.commands)
                    .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
            );
        }
        let ctx = egui::Context::default();
        let owner = egui::Id::unique("owner");
        let mut text = "Tiếng Việt".to_owned();
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    text_clipboard_for(ui.ctx(), owner, Platform::Windows);
                    ui.add(egui::TextEdit::singleline(&mut text).id_salt("text"))
                        .request_focus();
                },
            );
            output.textures_delta.clear();
            output
        };
        frame(vec![]);
        frame(vec![key(Key::A, egui::Modifiers::COMMAND)]);
        let copied = frame(vec![key(Key::Insert, egui::Modifiers::CTRL)]);
        assert!(
            copied
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c,egui::OutputCommand::CopyText(t) if t=="Tiếng Việt"))
        );
        frame(vec![key(Key::Delete, egui::Modifiers::SHIFT)]);
        drop(frame);
        assert_eq!(text, "");
    }
}
