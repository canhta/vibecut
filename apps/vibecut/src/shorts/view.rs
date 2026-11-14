//! Draws the shorts window: the pick screen, the working screen and the review.
//!
//! Geometry is the measured desktop reference, in logical px from the window's
//! content origin, at the 1718×989 window; the page keeps everything centred.

use crate::i18n;
mod options;
mod pick;
mod progress;
mod recent;
mod results;
mod review;
mod toast;
mod tools;
mod widgets;

use super::{Action, Context, state::State};
use crate::{Error, egui, i18n::Text, kit};
use std::time::Duration;

pub(super) use widgets::{author, named};

/// The right-hand tools of the review.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    Captions,
    Trim,
    Layout,
}

impl State {
    pub(super) fn tr(&self, key: Text) -> &'static str {
        i18n::text(self.preferences.locale, key)
    }
}

pub(super) fn show(state: &mut State, cx: &Context, ui: &mut egui::Ui) -> Vec<Action> {
    if !state.configured {
        kit::install_fonts(ui.ctx());
        kit::install(ui.ctx());
        state.configured = true;
    }
    kit::install_theme(ui.ctx(), state.preferences.dark);
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
    if state.generating.is_some() && !state.away {
        progress::show(state, ui, rect);
    } else if state.source.is_some() && state.editing {
        review::show(state, cx, ui, rect, &mut actions);
    } else if state.source.is_some() {
        results::show(state, ui, rect);
    } else {
        pick::show(state, ui, rect);
    }
    toast::show(state, ui);
    if let Some(kind) = state.pick_requested.take() {
        actions.push(Action::Pick(kind));
    }
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
