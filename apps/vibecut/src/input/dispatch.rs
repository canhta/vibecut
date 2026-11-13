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
    Action {
        action: &'static str,
        chord: Chord,
    },
}

/// Opaque custody token handed only to the text reader service.
#[derive(Clone)]
pub struct TextPasteRequest {
    viewport: crate::egui::ViewportId,
    target: crate::egui::Id,
    owner: crate::egui::Id,
    issued: f64,
    serial: u64,
}
#[derive(Clone)]
struct PendingText {
    request: TextPasteRequest,
    serviced: bool,
    stale: bool,
    completion: Option<Option<std::sync::Arc<str>>>,
}
fn request_id(viewport: crate::egui::ViewportId) -> crate::egui::Id {
    crate::egui::Id::unique(("input.text_clipboard", viewport))
}
/// Claim one queued read in the current viewport; repeated/discard passes cannot reread it.
pub fn take_text_paste(ctx: &crate::egui::Context) -> Option<TextPasteRequest> {
    let id = request_id(ctx.viewport_id());
    ctx.data_mut(|data| {
        let mut p = data.get_temp::<PendingText>(id)?;
        if p.serviced {
            return None;
        }
        p.serviced = true;
        let request = (!p.stale).then(|| p.request.clone());
        if p.stale {
            p.completion = Some(None);
        }
        data.insert_temp(id, p);
        request
    })
}
pub(crate) fn cancel_text_paste(ctx: &crate::egui::Context, viewport: crate::egui::ViewportId) {
    ctx.data_mut(|data| data.remove::<PendingText>(request_id(viewport)));
}
/// Complete the actual read, including an empty/error result, under its original custody.
/// Completion is tagged data, never an unowned egui Paste event in a later input frame.
pub fn complete_text_paste(
    ctx: &crate::egui::Context,
    request: TextPasteRequest,
    text: Option<String>,
) {
    let id = request_id(request.viewport);
    ctx.data_mut(|data| {
        if let Some(mut p) = data.get_temp::<PendingText>(id)
            && p.serviced
            && p.request.serial == request.serial
            && p.request.target == request.target
            && p.request.owner == request.owner
            && p.completion.is_none()
        {
            p.completion = Some(text.map(std::sync::Arc::from));
            data.insert_temp(id, p);
        }
    });
    ctx.request_repaint_of(request.viewport);
}
/// Bridge raw native text chords before widgets. The service claims/completes reads;
/// root and immediate children use the same boundary, independent of eframe action drains.
pub fn text_clipboard(ctx: &crate::egui::Context, owner: crate::egui::Id) {
    text_clipboard_for(ctx, owner, super::Platform::current());
}
fn text_clipboard_for(
    ctx: &crate::egui::Context,
    owner: crate::egui::Id,
    platform: super::Platform,
) {
    use crate::egui::{Event, Key};
    let viewport = ctx.viewport_id();
    let id = request_id(viewport);
    let focus = ctx
        .memory(|m| m.focused())
        .filter(|_| ctx.text_edit_focused());
    let mut pending = ctx.data_mut(|data| data.get_temp::<PendingText>(id));
    let (time, focused, pointer, closing) = ctx.input(|i| {
        (
            i.time,
            i.focused,
            i.pointer.any_pressed(),
            i.viewport().close_requested(),
        )
    });
    let chord_frame = crate::egui::Id::unique(("input.text_paste_chord", ctx.viewport_id()));
    let frame = ctx.cumulative_frame_nr();
    let serial = ctx.cumulative_pass_nr();
    let mut last_chord = ctx.data_mut(|data| data.get_temp::<u64>(chord_frame));
    let mut delivery = None;
    if let Some(p) = &mut pending {
        p.stale |= focus != Some(p.request.target)
            || owner != p.request.owner
            || !focused
            || pointer
            || closing
            || time - p.request.issued >= 2.0;
        if let Some(completion) = p.completion.take() {
            if !p.stale {
                delivery = completion;
            }
            pending = None;
        }
    }
    ctx.input_mut(|input| {
        let events = std::mem::take(&mut input.events);
        if let Some(text) = delivery {
            input.events.push(Event::Paste(text.to_string()));
        }
        for event in events {
            if let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
                && focus.is_some()
                && focused
                && !closing
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
                        && last_chord != Some(frame)
                        && let Some(target) = focus
                    {
                        last_chord = Some(frame);
                        pending = Some(PendingText {
                            request: TextPasteRequest {
                                viewport,
                                target,
                                owner,
                                issued: time,
                                serial,
                            },
                            serviced: false,
                            stale: false,
                            completion: None,
                        });
                    }
                    continue;
                }
            }
            // Genuine context-menu/native Paste and IME are independent, unowned events.
            input.events.push(event);
        }
    });
    ctx.data_mut(|data| {
        if let Some(frame) = last_chord {
            data.insert_temp(chord_frame, frame);
        }
        if let Some(pending) = pending {
            data.insert_temp(id, pending);
        } else {
            data.remove::<PendingText>(id);
        }
    });
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
                let requests = std::cell::RefCell::new(Vec::new());
                let closing = std::cell::Cell::new(false);
                let mut frame =
                    |events: Vec<Event>, time: f64, owner: egui::Id, field: egui::Id| {
                        let mut raw = egui::RawInput {
                            viewport_id: viewport,
                            events,
                            time: Some(time),
                            focused: true,
                            ..Default::default()
                        };
                        if closing.get() {
                            raw.viewports
                                .entry(viewport)
                                .or_default()
                                .events
                                .push(egui::ViewportEvent::Close);
                        } else {
                            raw.viewports.entry(viewport).or_default();
                        }
                        let mut output = ctx.run_ui(raw, |ui| {
                            text_clipboard_for(ui.ctx(), owner, platform);
                            ui.add(egui::TextEdit::singleline(&mut value).id(field))
                                .request_focus();
                            if let Some(request) = take_text_paste(ui.ctx()) {
                                requests.borrow_mut().push(request);
                            }
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
                assert!(
                    !output
                        .viewport_output
                        .values()
                        .flat_map(|v| &v.commands)
                        .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
                );
                let request = requests.borrow_mut().pop().expect("focused read queued");
                // Real read service may complete after an empty intervening input frame.
                frame(vec![], 0.25, owner, field);
                complete_text_paste(&ctx, request, Some(r#"{"version":1}"#.into()));
                frame(vec![], 0.3, owner, field);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    0.4,
                    owner,
                    field,
                );
                let request = requests.borrow_mut().pop().unwrap();
                frame(vec![], 0.45, egui::Id::unique("new scene"), field);
                complete_text_paste(&ctx, request, Some("stale".into()));
                frame(vec![], 0.5, owner, field);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    0.6,
                    owner,
                    field,
                );
                let request = requests.borrow_mut().pop().unwrap();
                frame(vec![], 2.9, owner, field);
                complete_text_paste(&ctx, request, Some("expired".into()));
                frame(vec![], 3.0, owner, field);
                // A genuine widget/context-menu Paste has no pending request.
                frame(vec![Event::Paste(" + menu".into())], 3.1, owner, field);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.2,
                    owner,
                    field,
                );
                let request = requests.borrow_mut().pop().unwrap();
                let replacement = egui::Id::unique("replacement field");
                ctx.memory_mut(|m| m.request_focus(replacement));
                frame(vec![], 3.25, owner, replacement);
                complete_text_paste(&ctx, request, Some("wrong focus".into()));
                frame(vec![], 3.3, owner, replacement);
                // Empty completion explicitly releases custody; no assumed frame count.
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.31,
                    owner,
                    replacement,
                );
                let empty = requests.borrow_mut().pop().unwrap();
                frame(vec![], 3.32, owner, replacement);
                complete_text_paste(&ctx, empty, None);
                frame(vec![], 3.33, owner, replacement);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.34,
                    owner,
                    replacement,
                );
                let last = requests.borrow_mut().pop().unwrap();
                // A duplicate old completion cannot complete the replacement request.
                complete_text_paste(&ctx, last.clone(), None);
                frame(vec![], 3.35, owner, replacement);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.351,
                    owner,
                    replacement,
                );
                let fresh = requests.borrow_mut().pop().unwrap();
                complete_text_paste(&ctx, last, Some("replayed".into()));
                complete_text_paste(&ctx, fresh, None);
                frame(vec![], 3.352, owner, replacement);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.36,
                    owner,
                    replacement,
                );
                let closed = requests.borrow_mut().pop().unwrap();
                closing.set(true);
                frame(vec![], 3.37, owner, replacement);
                closing.set(false);
                complete_text_paste(&ctx, closed, Some("closed window".into()));
                frame(vec![], 3.38, owner, replacement);
                frame(
                    vec![key(Key::V, egui::Modifiers::COMMAND)],
                    3.39,
                    owner,
                    replacement,
                );
                let removed = requests.borrow_mut().pop().unwrap();
                // Window removal may happen outside a child callback, without a Close input.
                cancel_text_paste(&ctx, viewport);
                complete_text_paste(&ctx, removed, Some("removed viewport".into()));
                frame(vec![], 3.395, owner, replacement);
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
        let requests = std::cell::RefCell::new(Vec::new());
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
                    if let Some(request) = take_text_paste(ui.ctx()) {
                        requests.borrow_mut().push(request);
                    }
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
        frame(vec![key(Key::Insert, egui::Modifiers::SHIFT)]);
        complete_text_paste(
            &ctx,
            requests.borrow_mut().pop().unwrap(),
            Some("Alias Việt".into()),
        );
        frame(vec![]);
        frame(vec![key(Key::A, egui::Modifiers::COMMAND)]);
        frame(vec![key(Key::Paste, egui::Modifiers::NONE)]);
        complete_text_paste(
            &ctx,
            requests.borrow_mut().pop().unwrap(),
            Some("Phím Paste".into()),
        );
        frame(vec![]);
        drop(frame);
        assert_eq!(text, "Phím Paste");
    }
}
