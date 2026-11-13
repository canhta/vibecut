//! Applies catalog choices at validated video cuts.
use super::super::{AssetsTab, Text, Tick, egui};
use super::State;
use crate::editor::Port;
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

pub(in crate::editor::panels::assets) fn cuts(session: &engine::Session) -> Vec<Cut> {
    session
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

pub(in crate::editor::panels::assets) fn refusal(
    session: &engine::Session,
    cut: Cut,
) -> Option<Text> {
    if session.transaction_active() {
        return Some(Text::TransitionBusy);
    }
    let data = session.project().data();
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
    session: &engine::Session,
    playhead: Option<Tick>,
    cuts: &[Cut],
) -> Option<Cut> {
    let selected = session.selection();
    if selected.len() == 2 {
        cuts.iter()
            .copied()
            .find(|cut| selected.contains(&cut.from) && selected.contains(&cut.to))
    } else if selected.len() <= 1 {
        let at = playhead?;
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
        port: &mut Port<'_>,
        kind: &str,
    ) {
        let cuts = cuts(&port.session);
        let selected = selected_cut(&port.session, port.read.playhead, &cuts);
        match selected {
            Some(cut) => self.add_transition_at_cut(port, kind, cut),
            None => {
                let text = port.tr(Text::TransitionSelectCut);
                port.toast(text, None);
            }
        }
    }

    fn add_transition_at_cut(&mut self, port: &mut Port<'_>, kind: &str, cut: Cut) {
        if let Some(reason) = refusal(&port.session, cut) {
            let text = port.tr(reason);
            port.toast(text, None);
            return;
        }
        let result = port.session.apply(engine::Command::AddTransition {
            track: cut.track,
            from: cut.from,
            to: cut.to,
            kind: kind.to_owned(),
            duration: Tick(TICKS_PER_SECOND / 2),
            alignment: TransitionAlignment::Center,
        });
        match result {
            Ok(_) => port.sync(),
            Err(error) => {
                tracing::warn!(category = "transition_apply", %error, "transition refused");
                let text = port.tr(Text::TransitionFailed);
                port.toast(text, None);
            }
        }
    }

    pub(in crate::editor) fn begin_transition_drag(
        &mut self,
        port: &mut Port<'_>,
        kind: &'static str,
    ) {
        self.transition_drag = Some((kind, port.session.project().clone()));
    }

    /// Drop accepts only the visible shared edge of two touching clips. No
    /// timeline gesture or project edit starts until the pointer is released.
    pub(in crate::editor) fn finish_transition_drag(
        &mut self,
        port: &mut Port<'_>,
        ctx: &egui::Context,
    ) {
        let Some((kind, before)) = self.transition_drag.as_ref() else {
            return;
        };
        let kind = *kind;
        let before = std::sync::Arc::clone(before);
        if port.read.preferences.assets_tab != AssetsTab::Transitions
            || !std::sync::Arc::ptr_eq(&before, port.session.project())
            || ctx.input(|input| !input.focused || input.key_pressed(egui::Key::Escape))
        {
            self.transition_drag = None;
            return;
        }
        let pointer = ctx.input(|input| input.pointer.interact_pos());
        let rects = port.read.item_rects;
        let target = pointer.and_then(|point| {
            cuts(&port.session).into_iter().find(|cut| {
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
            let valid = refusal(&port.session, cut).is_none();
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
                            crate::kit::accent(port.dark())
                        } else {
                            crate::kit::secondary(port.dark())
                        },
                    ),
                );
            }
        }
        if ctx.input(|input| input.pointer.any_released()) {
            self.transition_drag = None;
            if let Some(cut) = target {
                self.add_transition_at_cut(port, kind, cut);
            } else {
                let text = port.tr(Text::TransitionDropMiss);
                port.toast(text, None);
            }
        } else if !ctx.input(|input| input.pointer.primary_down()) {
            self.transition_drag = None;
        }
    }
}
