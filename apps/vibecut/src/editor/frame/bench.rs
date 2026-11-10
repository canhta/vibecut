//! Drives one editor area alone over a state harness.
use super::{Context, Desk};
use crate::editor::{Editor, egui};
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};

/// Where the area lives between frames: beside the editor (an area with its
/// own state field) or inside it (a state the editor's facade also writes).
pub(crate) struct Host<A> {
    take: fn(&mut Editor) -> A,
    put: fn(&mut Editor, A),
}

impl<A> Clone for Host<A> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<A> Copy for Host<A> {}

/// One area mounted in a window-sized harness; every returned request is
/// routed through the shared desk before the next frame.
pub(crate) struct Bench<A, M> {
    harness: Harness<'static, Scene<A, M>>,
}

pub(crate) struct Scene<A, M> {
    pub(crate) editor: Editor,
    area: Option<A>,
    host: Option<Host<A>>,
    show: fn(&mut A, &Context<'_>, &mut egui::Ui) -> Vec<M>,
    route: fn(&mut Desk<'_>, Vec<M>),
    configured: bool,
}

impl<A, M> Scene<A, M> {
    fn paint(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            Editor::fonts(ui.ctx());
            self.configured = true;
        }
        if self.host.is_some() {
            // A dialog state the editor's facade writes runs the editor loop
            // that advances its jobs, exactly as the frame does before painting.
            self.editor.tick();
            self.editor.widgets.clear();
        }
        if self.area.is_none()
            && let Some(host) = self.host
        {
            self.area = Some((host.take)(&mut self.editor));
        }
        let Some(mut area) = self.area.take() else {
            return;
        };
        let actions = {
            let cx = self.editor.context();
            (self.show)(&mut area, &cx, ui)
        };
        match self.host {
            Some(host) => (host.put)(&mut self.editor, area),
            None => self.area = Some(area),
        }
        {
            let mut desk = self.editor.desk();
            (self.route)(&mut desk, actions);
        }
        self.editor.drain_requests();
    }
}

impl<A: 'static, M: 'static> Bench<A, M> {
    /// Mount an area the caller keeps beside the editor.
    pub(crate) fn new(
        editor: Editor,
        area: A,
        show: fn(&mut A, &Context<'_>, &mut egui::Ui) -> Vec<M>,
        route: fn(&mut Desk<'_>, Vec<M>),
        size: egui::Vec2,
    ) -> Self {
        Self::mount(editor, Some(area), None, show, route, size)
    }

    /// Mount an area the editor's own facade also writes between frames.
    pub(crate) fn hosted(
        editor: Editor,
        take: fn(&mut Editor) -> A,
        put: fn(&mut Editor, A),
        show: fn(&mut A, &Context<'_>, &mut egui::Ui) -> Vec<M>,
        route: fn(&mut Desk<'_>, Vec<M>),
        size: egui::Vec2,
    ) -> Self {
        Self::mount(editor, None, Some(Host { take, put }), show, route, size)
    }

    fn mount(
        editor: Editor,
        area: Option<A>,
        host: Option<Host<A>>,
        show: fn(&mut A, &Context<'_>, &mut egui::Ui) -> Vec<M>,
        route: fn(&mut Desk<'_>, Vec<M>),
        size: egui::Vec2,
    ) -> Self {
        let scene = Scene {
            editor,
            area,
            host,
            show,
            route,
            configured: false,
        };
        let mut harness =
            Harness::new_ui_state(|ui, scene: &mut Scene<A, M>| scene.paint(ui), scene);
        harness.set_size(size);
        harness.run_steps(3);
        Self { harness }
    }

    pub(crate) fn editor(&self) -> &Editor {
        &self.harness.state().editor
    }

    pub(crate) fn editor_mut(&mut self) -> &mut Editor {
        &mut self.harness.state_mut().editor
    }

    /// The kittest harness, for queries and clipboard output.
    pub(crate) fn harness(&self) -> &Harness<'static, Scene<A, M>> {
        &self.harness
    }

    /// The kittest harness, for direct frame stepping.
    pub(crate) fn harness_mut(&mut self) -> &mut Harness<'static, Scene<A, M>> {
        &mut self.harness
    }

    /// Preview generations from the shared transport; one committed edit
    /// resyncs once, a refused edit does not.
    pub(crate) fn generation(&self) -> u64 {
        self.editor().playback.generation()
    }

    pub(crate) fn step(&mut self) {
        self.harness.run_steps(1);
    }

    /// Click one automation control by its author id.
    pub(crate) fn click(&mut self, id: &str) {
        self.harness
            .get(By::new().predicate(|node| node.author_id() == Some(id)))
            .click();
        self.harness.run_steps(3);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::Bench;
    use crate::editor::{BTreeSet, Editor, ItemId, egui, properties};
    use crate::playback::{Files, Playback};
    use crate::preferences::Preferences;
    use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session, TrackPlacement};
    use project::{ItemParams, Project, ProjectId, TrackKind};
    use std::sync::Arc;
    use time::{SourceRange, TICKS_PER_SECOND, Tick};

    const SECOND: i64 = TICKS_PER_SECOND;

    fn audio_editor() -> (Editor, ItemId) {
        let mut session =
            Session::new(Project::empty(ProjectId(1), "Bench".into()).unwrap(), 50).unwrap();
        let track = match session
            .apply(Command::AddTrack {
                placement: TrackPlacement::Auto,
                name: "Graphic".into(),
                kind: TrackKind::Video,
            })
            .unwrap()
            .created
            .first()
        {
            Some(CreatedId::Track(id)) => *id,
            _ => panic!("track"),
        };
        let item = match session
            .apply(Command::InsertItem {
                placement: Placement::Track { track },
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, Tick(SECOND)).unwrap(),
                    start: Tick::ZERO,
                    content: ItemParams::Overlay(project::OverlayParams {
                        color: project::Color([10, 20, 30, 255]),
                    }),
                },
            })
            .unwrap()
            .created
            .first()
        {
            Some(CreatedId::Item(id)) => *id,
            _ => panic!("item"),
        };
        let playback = Playback::new(
            &session,
            ByteBudget::new(64 << 20).unwrap(),
            Arc::new(Files),
        )
        .unwrap();
        let mut editor = Editor::new(session, playback, Preferences::default());
        editor.selection = BTreeSet::from([item]);
        editor
            .set_preference(
                "properties_tabs",
                serde_json::json!({"graphic": "blending"}),
            )
            .unwrap();
        (editor, item)
    }

    fn bench(editor: Editor) -> Bench<properties::State, properties::Action> {
        Bench::new(
            editor,
            properties::State::default(),
            properties::show,
            properties::apply,
            egui::vec2(360.0, 900.0),
        )
    }

    fn keyframes(editor: &Editor, item: ItemId) -> usize {
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item)
            .unwrap()
            .animations
            .key_count()
    }

    #[test]
    fn one_widget_edit_resyncs_the_preview_once() {
        let (editor, item) = audio_editor();
        let mut bench = bench(editor);
        let history = bench.editor().session.history_len();
        let generation = bench.generation();
        bench.click("inspector.keyframe.opacity");
        assert_eq!(
            keyframes(bench.editor(), item),
            1,
            "the diamond commits a key"
        );
        assert_eq!(
            bench.generation(),
            generation + 1,
            "one committed edit resyncs the preview once"
        );
        assert_eq!(bench.editor().session.history_len().0, history.0 + 1);
        assert!(bench.editor().error.is_none());
    }

    #[test]
    fn a_refused_field_edit_does_not_resync() {
        let (editor, item) = audio_editor();
        let mut bench = bench(editor);
        let generation = bench.generation();
        assert!(
            bench
                .editor_mut()
                .set_shared_field("opacity", "loud")
                .is_err()
        );
        bench.step();
        assert_eq!(
            bench.generation(),
            generation,
            "a refused edit leaves the preview alone"
        );
        assert_eq!(keyframes(bench.editor(), item), 0);
    }
}
