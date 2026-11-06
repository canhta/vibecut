//! Draws the shorts window: the pick screen, the working screen and the review.
//!
//! Geometry is the measured desktop reference, in logical px from the window's
//! content origin, at the 1718×989 window; the page keeps everything centred.

mod options;
mod pick;
mod progress;
mod results;
mod review;
mod toast;
mod tools;
mod widgets;

use super::{Action, Context, state::State};
use crate::{
    Editor, Error,
    catalog::{self, Text},
    egui,
    kit::{self},
};
use std::time::Duration;
use time::Tick;

pub(super) use widgets::{author, named};

/// The right-hand tools of the review.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    Captions,
    Trim,
    Layout,
}

/// The preview frame uploaded as a texture, keyed by the frame it shows.
pub(super) struct FrameTexture {
    pub(super) handle: egui::TextureHandle,
    pub(super) key: (u64, Tick),
}

impl State {
    pub(super) fn tr(&self, key: Text) -> &'static str {
        catalog::text(self.preferences.locale, key)
    }
}

pub(super) fn show(state: &mut State, cx: &Context, ui: &mut egui::Ui) -> Vec<Action> {
    if !state.configured {
        Editor::fonts(ui.ctx());
        kit::install(ui.ctx());
        state.configured = true;
    }
    Editor::theme(ui.ctx(), state.preferences.dark);
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, kit::window(state.preferences.dark));
    state.tick();
    let drops = ui.input(|i| i.raw.dropped_files.clone());
    if !drops.is_empty() {
        let result = match drops.as_slice() {
            [one] => state.pick_video(one.path().to_owned()),
            _ => Err(Error::Invalid("choose one video")),
        };
        if let Err(e) = result {
            state.fail(e, Text::ShortsError);
        }
    }
    let mut actions = Vec::new();
    if state.generating.is_some() {
        progress::show(state, ui, rect);
    } else if state.source.is_some() && state.editing {
        review::show(state, cx, ui, rect, &mut actions);
    } else if state.source.is_some() {
        results::show(state, ui, rect);
    } else {
        pick::show(state, ui, rect);
    }
    toast::show(state, ui);
    ui.ctx().request_repaint_after(Duration::from_millis(16));
    actions
}

/// The uploaded poster of suggestion `index`, creating its texture on first use.
pub(super) fn poster(
    thumbs: &mut super::state::Thumbs,
    ctx: &egui::Context,
    index: usize,
) -> Option<egui::TextureId> {
    if let Some(image) = thumbs.images.get_mut(index).and_then(Option::take) {
        let texture = ctx.load_texture(
            format!("shorts.poster.{index}"),
            image,
            egui::TextureOptions::LINEAR,
        );
        *thumbs.textures.get_mut(index)? = Some(texture);
    }
    thumbs
        .textures
        .get(index)?
        .as_ref()
        .map(egui::TextureHandle::id)
}
