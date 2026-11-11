//! Services focused text reads under viewport request custody.
use crate::{egui, input};

/// The app's read-only native boundary; injectable without touching the OS clipboard.
/// Copy/Cut output stays with egui; keyframe snapshots never enter this reader.
pub struct TextClipboard {
    read: Box<dyn FnMut() -> Option<String>>,
    viewports: std::collections::BTreeSet<egui::ViewportId>,
}
impl Default for TextClipboard {
    fn default() -> Self {
        Self::new(|| arboard::Clipboard::new().ok()?.get_text().ok())
    }
}
impl TextClipboard {
    pub fn new(read: impl FnMut() -> Option<String> + 'static) -> Self {
        Self {
            read: Box::new(read),
            viewports: Default::default(),
        }
    }
    pub fn bridge(&mut self, ctx: &egui::Context, owner: egui::Id) {
        self.viewports.insert(ctx.viewport_id());
        input::text_clipboard(ctx, owner);
        self.service(ctx);
    }
    /// Drop custody when composition removes/hides a viewport without another callback.
    pub(crate) fn retain_viewports(&mut self, ctx: &egui::Context, active: &[egui::ViewportId]) {
        self.viewports.retain(|viewport| {
            if active.contains(viewport) {
                true
            } else {
                input::cancel_text_paste(ctx, *viewport);
                false
            }
        });
    }
    /// Service after request issuance, in root or any immediate child callback.
    /// Completion survives empty intervening frames until that viewport classifies it.
    pub fn service(&mut self, ctx: &egui::Context) {
        if let Some(request) = input::take_text_paste(ctx) {
            let text = (self.read)().map(|text| text.replace("\r\n", "\n"));
            input::complete_text_paste(ctx, request, text);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        reason = "injected native reader assertions"
    )]
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[test]
    fn native_text_service_completes_in_root_and_both_immediate_children_without_framework_actions()
    {
        for viewport in [
            egui::ViewportId::ROOT,
            egui::ViewportId::from_hash_of("editor.window"),
            egui::ViewportId::from_hash_of("shorts.window"),
        ] {
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let child_input = Rc::new(RefCell::new(egui::RawInput::default()));
            let visited = Rc::new(Cell::new(0));
            let input = child_input.clone();
            let visits = visited.clone();
            egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
                visits.set(visits.get() + 1);
                let mut raw = input.borrow_mut().take();
                raw.viewport_id = viewport.ids.this;
                raw.viewports.entry(viewport.ids.this).or_default();
                let mut output = ctx.run_ui(raw, |ui| (viewport.viewport_ui_cb)(ui));
                assert!(
                    !output
                        .viewport_output
                        .values()
                        .flat_map(|v| &v.commands)
                        .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
                );
                output.textures_delta.clear();
            });
            let reads = Rc::new(Cell::new(0));
            let count = reads.clone();
            let mut clipboard = TextClipboard::new(move || {
                count.set(count.get() + 1);
                match count.get() {
                    1 => Some("Việt\r\nđẹp".into()),
                    2 => None,
                    _ => Some("late".into()),
                }
            });
            let mut text = String::new();
            let field = egui::Id::unique("actual-field");
            let owner = egui::Id::unique("owner");
            let mut frame = |events: Vec<egui::Event>, time, owner, discard, retire| {
                let raw = egui::RawInput {
                    events,
                    time: Some(time),
                    focused: true,
                    ..Default::default()
                };
                let mut draw = |ui: &mut egui::Ui| {
                    assert_eq!(ui.ctx().viewport_id(), viewport);
                    clipboard.bridge(ui.ctx(), owner);
                    ui.add(egui::TextEdit::multiline(&mut text).id(field))
                        .request_focus();
                    if retire {
                        clipboard.retain_viewports(ui.ctx(), &[]);
                    }
                    if discard && ui.ctx().current_pass_index() == 0 {
                        ui.ctx().request_discard("native completion custody");
                    }
                };
                let mut output = if viewport == egui::ViewportId::ROOT {
                    ctx.run_ui(raw, draw)
                } else {
                    *child_input.borrow_mut() = raw;
                    ctx.run_ui(Default::default(), |ui| {
                        ui.ctx().show_viewport_immediate(
                            viewport,
                            Default::default(),
                            |ui, class| {
                                assert!(class == egui::ViewportClass::Immediate);
                                draw(ui);
                            },
                        );
                    })
                };
                assert!(
                    !output
                        .viewport_output
                        .values()
                        .flat_map(|v| &v.commands)
                        .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
                );
                output.textures_delta.clear();
            };
            let paste = || {
                vec![egui::Event::Key {
                    key: egui::Key::V,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }]
            };
            frame(vec![], 0.0, owner, false, false);
            frame(paste(), 0.1, owner, true, false);
            frame(vec![], 0.2, owner, false, false);
            assert_eq!(reads.get(), 1, "discard passes must not repeat native IO");
            frame(paste(), 0.3, owner, false, false);
            frame(vec![], 0.4, owner, false, false); // Explicit empty completion.
            frame(paste(), 0.5, owner, false, false);
            frame(
                vec![],
                0.6,
                egui::Id::unique("replacement-owner"),
                false,
                false,
            );
            frame(vec![], 0.7, owner, false, false);
            frame(paste(), 0.8, owner, false, true);
            frame(vec![], 0.9, owner, false, false);
            drop(frame);
            assert_eq!(reads.get(), 4);
            assert_eq!(text, "Việt\nđẹp");
            if viewport != egui::ViewportId::ROOT {
                assert!(visited.get() >= 8);
            }
        }
    }
}
