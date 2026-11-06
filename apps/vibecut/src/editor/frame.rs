//! Owns editor frame composition.
mod input;
mod layout;
mod shortcuts;
mod topbar;
mod view;
mod workflow;

use crate::{
    catalog::{Locale, Text},
    egui,
};

/// Read-only composition inputs shared by editor areas.
pub(crate) struct Context<'a> {
    pub(crate) session: &'a engine::Session,
    #[allow(dead_code)] // Consumed by the parallel area interface migrations.
    pub(crate) selection: &'a std::collections::BTreeSet<project::ItemId>,
    pub(crate) locale: Locale,
    pub(crate) dark: bool,
    pub(crate) preferences: &'a crate::catalog::Preferences,
    /// What the selection currently scopes: items, mask points or keyframes.
    pub(crate) scope: crate::selection::SelectionScope,
    pub(super) save_status: Text,
}

/// Transient editor-window chrome, independent of every other window.
#[derive(Default)]
pub(super) struct State {
    pub(super) layout_drag: Option<[f32; 3]>,
    pub(super) rename: Option<String>,
    pub(super) rename_focus: bool,
    widgets: Vec<(&'static str, Option<&'static str>, egui::Response)>,
}

pub(super) enum Action {
    Activate(&'static str),
    Rename(String),
}

pub(super) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    topbar::show(state, cx, ui)
}
