//! Applies catalog choices at validated video cuts.
use super::super::{AssetsTab, Text, Tick, egui};
use super::State;
use crate::editor::{Desk, Editor};
use project::{ItemId, TrackId, Transition, TransitionAlignment, TransitionId};
use time::TICKS_PER_SECOND;

/// A shared cut identity; times are resolved again before publication.
#[derive(Clone, Copy)]
pub(in crate::editor::panels::assets) struct Cut {
    pub track: TrackId,
    pub from: ItemId,
    pub to: ItemId,
    pub at: Tick,
}

pub(in crate::editor::panels::assets) fn cuts(editor: &Editor) -> Vec<Cut> {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == project::TrackKind::Video)
        .flat_map(|track| {
            track.items.windows(2).filter_map(move |pair| {
                let (a, b) = (pair.first()?, pair.get(1)?);
                (a.end().ok()? == b.start
                    && [a, b].iter().all(|item| {
                        matches!(
                            item.content,
                            project::ItemParams::Video(_) | project::ItemParams::Overlay(_)
                        )
                    }))
                .then_some(Cut {
                    track: track.id,
                    from: a.id,
                    to: b.id,
                    at: b.start,
                })
            })
        })
        .collect()
}

pub(in crate::editor::panels::assets) fn refusal(editor: &Editor, cut: Cut) -> Option<Text> {
    if editor.session.transaction_active() {
        return Some(Text::TransitionBusy);
    }
    let data = editor.session.project().data();
    let timeline = &data.scene().timeline;
    let Some(track) = timeline.iter().find(|track| track.id == cut.track) else {
        return Some(Text::TransitionSelectCut);
    };
    if track.locked {
        return Some(Text::TransitionLocked);
    }
    if timeline
        .transitions
        .iter()
        .any(|tr| tr.from == cut.from && tr.to == cut.to)
    {
        return Some(Text::TransitionPresent);
    }
    let mut probe = Transition {
        id: TransitionId(0),
        track: cut.track,
        from: cut.from,
        to: cut.to,
        from_pieces: vec![],
        to_pieces: vec![],
        kind: "cross_dissolve".into(),
        start: Tick::ZERO,
        duration: Tick::ZERO,
        alignment: TransitionAlignment::Center,
        params: Default::default(),
    };
    let Ok(bounds) = probe.bounds(timeline, data) else {
        return Some(Text::TransitionHandles);
    };
    if probe
        .set_duration(Tick(TICKS_PER_SECOND / 2), bounds)
        .is_err()
    {
        return Some(Text::TransitionHandles);
    }
    if timeline.transitions.iter().any(|tr| {
        tr.track == cut.track
            && tr.end().is_ok_and(|end| end > probe.start)
            && probe.end().is_ok_and(|end| end > tr.start)
    }) {
        return Some(Text::TransitionOverlap);
    }
    None
}

pub(in crate::editor::panels::assets) fn selected_cut(
    editor: &Editor,
    cuts: &[Cut],
) -> Option<Cut> {
    let selected = &editor.selection;
    if selected.len() == 2 {
        cuts.iter()
            .copied()
            .find(|cut| selected.contains(&cut.from) && selected.contains(&cut.to))
    } else if selected.len() <= 1 {
        let at = editor.playback.position().ok()?;
        let mut matches = cuts.iter().copied().filter(|cut| {
            (selected.is_empty() || selected.contains(&cut.from) || selected.contains(&cut.to))
                && cut.at == at
        });
        let cut = matches.next()?;
        matches.next().is_none().then_some(cut)
    } else {
        None
    }
}

impl State {
    pub(in crate::editor) fn add_transition_from_selection(
        &mut self,
        desk: &mut Desk<'_>,
        kind: &str,
    ) {
        let cuts = cuts(desk.editor());
        let selected = selected_cut(desk.editor(), &cuts);
        match selected {
            Some(cut) => self.add_transition_at_cut(desk, kind, cut),
            None => {
                let text =
                    crate::i18n::text(desk.editor().preferences.locale, Text::TransitionSelectCut);
                desk.toast(text, None);
            }
        }
    }

    fn add_transition_at_cut(&mut self, desk: &mut Desk<'_>, kind: &str, cut: Cut) {
        if let Some(reason) = refusal(desk.editor(), cut) {
            let text = crate::i18n::text(desk.editor().preferences.locale, reason);
            desk.toast(text, None);
            return;
        }
        desk.editor().session.end_merge();
        let result = desk.editor().session.apply(engine::Command::AddTransition {
            track: cut.track,
            from: cut.from,
            to: cut.to,
            kind: kind.to_owned(),
            duration: Tick(TICKS_PER_SECOND / 2),
            alignment: TransitionAlignment::Center,
        });
        match result {
            Ok(_) => desk.sync(),
            Err(error) => {
                tracing::warn!(category = "transition_apply", %error, "transition refused");
                let text =
                    crate::i18n::text(desk.editor().preferences.locale, Text::TransitionFailed);
                desk.toast(text, None);
            }
        }
    }

    pub(in crate::editor) fn begin_transition_drag(
        &mut self,
        desk: &mut Desk<'_>,
        kind: &'static str,
    ) {
        self.transition_drag = Some((kind, desk.editor().session.project().clone()));
    }

    /// Drop accepts only the visible shared edge of two touching clips. No
    /// timeline gesture or project edit starts until the pointer is released.
    pub(in crate::editor) fn finish_transition_drag(
        &mut self,
        desk: &mut Desk<'_>,
        ctx: &egui::Context,
    ) {
        let Some((kind, before)) = self.transition_drag.as_ref() else {
            return;
        };
        let kind = *kind;
        let before = std::sync::Arc::clone(before);
        if desk.editor().preferences.assets_tab != AssetsTab::Transitions
            || !std::sync::Arc::ptr_eq(&before, desk.editor().session.project())
            || ctx.input(|input| !input.focused || input.key_pressed(egui::Key::Escape))
        {
            self.transition_drag = None;
            return;
        }
        let pointer = ctx.input(|input| input.pointer.interact_pos());
        let rects = desk.editor().timeline.item_rects().to_vec();
        let target = pointer.and_then(|point| {
            cuts(desk.editor()).into_iter().find(|cut| {
                let (Some((_, a)), Some((_, b))) = (
                    rects.iter().find(|(id, _)| *id == cut.from),
                    rects.iter().find(|(id, _)| *id == cut.to),
                ) else {
                    return false;
                };
                (a.right() - b.left()).abs() < 2.0
                    && (point.x - b.left()).abs() <= 12.0
                    && point.y >= a.top().max(b.top())
                    && point.y <= a.bottom().min(b.bottom())
            })
        });
        if let Some(cut) = target {
            let valid = refusal(desk.editor(), cut).is_none();
            if let Some((_, rect)) = rects.iter().find(|(id, _)| *id == cut.to) {
                ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::unique("transition.drop"),
                ))
                .line_segment(
                    [rect.left_top(), rect.left_bottom()],
                    egui::Stroke::new(
                        3.0,
                        if valid {
                            desk.editor().accent()
                        } else {
                            desk.editor().secondary()
                        },
                    ),
                );
            }
        }
        if ctx.input(|input| input.pointer.any_released()) {
            self.transition_drag = None;
            if let Some(cut) = target {
                self.add_transition_at_cut(desk, kind, cut);
            } else {
                let text =
                    crate::i18n::text(desk.editor().preferences.locale, Text::TransitionDropMiss);
                desk.toast(text, None);
            }
        } else if !ctx.input(|input| input.pointer.primary_down()) {
            self.transition_drag = None;
        }
    }
}
