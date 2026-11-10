//! Renders bounded illustrative transition thumbnails through the shared compositor.
use super::{Text, egui, theme};
use engine::{ByteBudget, ByteBudgetHooks};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Transition,
    TransitionAlignment, TransitionDefinition, TransitionId,
};
use time::{SourceRange, Tick};

pub(super) fn name(definition: &TransitionDefinition) -> Text {
    match definition.kind {
        "cross_dissolve" => Text::TransitionCrossDissolve,
        "dip_to_black" => Text::TransitionDipBlack,
        "dip_to_white" => Text::TransitionDipWhite,
        _ => Text::Transitions,
    }
}
pub(super) fn family(definition: &TransitionDefinition) -> Text {
    match definition.family {
        "Dissolve" => Text::TransitionDissolve,
        _ => Text::AssetsAll,
    }
}

pub(super) struct Preview {
    project: Project,
    settings: render::RenderSettings,
    budget: ByteBudget,
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) failed: bool,
    frame: Option<u32>,
    started: Option<f64>,
    pub(super) dark: bool,
}
impl Preview {
    pub(super) fn new(kind: &str, dark: bool) -> crate::Result<Self> {
        let mut data = Project::empty(ProjectId(0), String::new().into())?.into_data();
        data.canvas.width = 48;
        data.canvas.height = 48;
        data.scene_mut().timeline.main.items = [
            (1, 0, theme::accent(dark)),
            (2, 100, theme::video_border(dark)),
        ]
        .into_iter()
        .map(|(id, start, color)| {
            Ok(Item {
                hidden: false,
                id: ItemId(id),
                source_asset: None,
                source_range: SourceRange::new(Tick(0), Tick(100))?,
                start: Tick(start),
                content: ItemParams::Overlay(OverlayParams {
                    color: Color(color.to_array()),
                }),
                retime: Default::default(),
                animations: Default::default(),
                effects: vec![],
                masks: vec![],
                blend_mode: Default::default(),
            })
        })
        .collect::<project::Result<Vec<_>>>()?;
        let track = data.scene().timeline.main.id;
        data.scene_mut().timeline.transitions.push(Transition {
            id: TransitionId(1),
            track,
            from: ItemId(1),
            to: ItemId(2),
            from_pieces: vec![],
            to_pieces: vec![],
            kind: kind.into(),
            start: Tick(70),
            duration: Tick(60),
            alignment: TransitionAlignment::Center,
        });
        let project = Project::new(data)?;
        let mut settings = render::RenderSettings::from_project(&project);
        for (id, x) in [(1, 0.12), (2, 0.38)] {
            settings.placements.insert(
                ItemId(id),
                render::Placement {
                    destination: render::Rect {
                        x,
                        y: 0.25,
                        width: 0.5,
                        height: 0.5,
                    },
                    ..Default::default()
                },
            );
        }
        Ok(Self {
            project,
            settings,
            budget: ByteBudget::new(1 << 20)?,
            texture: None,
            failed: false,
            frame: None,
            started: None,
            dark,
        })
    }
    /// One 48 px frame on entry, then at most 12 frames/s while hovered/focused.
    pub(super) fn update(&mut self, ctx: &egui::Context, kind: &str, active: bool) {
        let now = ctx.input(|input| input.time);
        let frame = if active {
            let started = *self.started.get_or_insert(now);
            (((now - started).max(0.0) * 12.0) as u32) % 24
        } else {
            self.started = None;
            9
        };
        if self.frame == Some(frame) || self.failed {
            return;
        }
        self.frame = Some(frame);
        let at = Tick(60 + i64::from(frame) * 4);
        let hooks = ByteBudgetHooks::new(&self.budget, None);
        let result = render::Renderer::new(&self.project, self.settings.clone())
            .and_then(|mut renderer| {
                renderer.render_frame(at, &mut render::FileSources::default(), &hooks)
            })
            .and_then(|image| image.rgba8(13, &hooks));
        match result {
            Ok(image) => {
                let image = egui::ColorImage::from_rgba_unmultiplied([48, 48], &image.bytes);
                if let Some(texture) = &mut self.texture {
                    texture.set(image, Default::default());
                } else {
                    self.texture = Some(ctx.load_texture(
                        format!("transition.{kind}"),
                        image,
                        Default::default(),
                    ));
                }
            }
            Err(error) => {
                tracing::warn!(category = "transition_preview", %error, "preview failed");
                self.failed = true;
            }
        }
    }
}
