//! Prepares the area's read-only workflow projection before rendering.
use super::{
    AssetRow, AssetState, AssetsTab, BTreeMap, Editor, EffectRow, Icon, MediaKind, Model, Row,
    Text, asset_row, effect_preview_instance, egui, file_name,
};

pub(in crate::editor) fn prepare(editor: &mut Editor, ctx: &egui::Context) {
    let rows = if !matches!(
        editor.preferences.assets_tab,
        AssetsTab::Text | AssetsTab::Effects | AssetsTab::Transitions | AssetsTab::Captions
    ) {
        editor.assets.refresh_media_sizes(&editor.session);
        let sizes = editor.assets.media_size_map();
        let order = editor
            .assets
            .media_order(&editor.session, &editor.preferences);
        let by_id: BTreeMap<_, _> = editor
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| (asset.id(), asset))
            .collect();
        let mut rows: Vec<_> = order
            .into_iter()
            .filter_map(|id| {
                let asset = by_id.get(&id)?;
                let status = editor
                    .assets
                    .asset_states
                    .get(&id)
                    .copied()
                    .unwrap_or(AssetState::Probing);
                Some(Row {
                    display: asset_row(
                        asset,
                        editor.preferences.locale,
                        sizes.get(&id).copied().flatten(),
                        status,
                    ),
                    kind: MediaKind::of(asset),
                })
            })
            .collect();
        rows.extend(editor.assets.pending_imports.iter().map(|(id, path)| Row {
            display: AssetRow {
                id: *id,
                name: file_name(path),
                info: String::new(),
                icon: Icon::UploadSimple,
                state: AssetState::Probing,
                duration: None,
            },
            kind: MediaKind::Other,
        }));
        rows
    } else {
        Vec::new()
    };
    let thumbnails = rows
        .iter()
        .filter_map(|row| {
            editor
                .assets
                .thumbnail_texture(ctx, row.display.id)
                .map(|texture| (row.display.id, texture))
        })
        .collect();
    let mut effects = Vec::new();
    if let Some(previews) = &mut editor.effect_previews {
        previews.update(
            editor.preferences.assets_tab == AssetsTab::Effects,
            editor.clock.now(),
        );
    }
    if editor.preferences.assets_tab == AssetsTab::Effects {
        ctx.request_repaint_after(std::time::Duration::from_nanos(1_000_000_000 / 30));
        for definition in project::effect_definitions()
            .iter()
            .filter(|definition| definition.shelf.in_library())
        {
            let params = effect_preview_instance(definition);
            if let (Some(previews), Some(params)) = (&mut editor.effect_previews, params.as_ref()) {
                previews.ensure(definition.id, params, &project::Animations::default());
            }
            let status = editor
                .effect_previews
                .as_ref()
                .and_then(|previews| previews.status(definition.id));
            let texture = editor
                .effect_previews
                .as_mut()
                .and_then(|previews| previews.texture(ctx, definition.id));
            effects.push(EffectRow {
                kind: definition.id,
                name: crate::i18n::effect_label(definition.id, None)
                    .map_or(definition.name, |key| editor.tr(key)),
                shelf: definition.shelf,
                status,
                texture,
            });
        }
    }
    let mut preferences = editor.preferences.clone();
    if !matches!(
        preferences.assets_tab,
        AssetsTab::Media
            | AssetsTab::Sounds
            | AssetsTab::Text
            | AssetsTab::Effects
            | AssetsTab::Transitions
            | AssetsTab::Captions
            | AssetsTab::Stickers
    ) {
        preferences.assets_tab = AssetsTab::Media;
    }
    let (transition_refusal, transition_drag_enabled) =
        if preferences.assets_tab == AssetsTab::Transitions {
            let cuts = super::workflow::cuts(editor);
            let reason = super::workflow::selected_cut(editor, &cuts)
                .map(|cut| super::workflow::refusal(editor, cut))
                .unwrap_or(Some(Text::TransitionSelectCut));
            (reason, !cuts.is_empty())
        } else {
            (None, false)
        };
    let graphic_previews = if preferences.assets_tab == AssetsTab::Stickers {
        [
            project::GraphicParams::rectangle(),
            project::GraphicParams::ellipse(),
        ]
        .map(
            |graphic| match graphic_preview(graphic, ctx.pixels_per_point()) {
                Ok(image) => {
                    Some(ctx.load_texture("graphic-shelf", image, egui::TextureOptions::LINEAR))
                }
                Err(_) => {
                    tracing::warn!(category = "graphic_preview", "shape preview unavailable");
                    None
                }
            },
        )
    } else {
        [None, None]
    };
    let (music_credits, music_credits_failed) = match editor.session.project().music_credits() {
        Ok(credits) => (credits, false),
        Err(_) => (Vec::new(), true),
    };
    editor.assets.model = Model {
        preferences,
        rows,
        thumbnails,
        effects,
        transition_refusal,
        transition_drag_enabled,
        graphic_insertion_ready: editor.graphic_insertion_available(),
        graphic_previews,
        importing: editor.assets.importing(),
        selection: editor.assets.media_selection.clone(),
        reveal: editor.assets.media_reveal.map(|reveal| reveal.asset),
        marquee: editor.assets.media_marquee.clone(),
        failed: !editor.assets.import_failures.is_empty() || editor.assets.import_notice.is_some(),
        sounds: editor.sounds.as_ref().map(|sounds| sounds.view()),
        music: editor.music.as_ref().map(|music| music.view()),
        music_credits,
        music_credits_failed,
    };
}

/// Tiny run-owned shelf images use the production compositor, not a second shape renderer.
fn graphic_preview(graphic: project::GraphicParams, scale: f32) -> crate::Result<egui::ColorImage> {
    let mut data = project::Project::empty(project::ProjectId(0), "".into())?.into_data();
    data.canvas.width = 512;
    data.canvas.height = 512;
    data.canvas.background = project::Color([0; 4]);
    data.scene_mut().timeline.main.items.push(project::Item {
        hidden: false,
        blend_mode: Default::default(),
        masks: vec![],
        effects: vec![],
        animations: Default::default(),
        id: project::ItemId(0),
        source_asset: None,
        source_range: time::SourceRange::new(time::Tick::ZERO, time::Tick(1))?,
        start: time::Tick::ZERO,
        retime: Default::default(),
        content: project::ItemParams::Graphic(graphic),
    });
    let project = project::Project::new(data)?;
    let size = (48. * scale).round().clamp(48., 192.) as u32;
    let mut settings = render::RenderSettings::from_project(&project);
    settings.width = size;
    settings.height = size;
    let budget = engine::ByteBudget::new(4 << 20)?;
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let image = render::Renderer::new(&project, settings)?
        .render_frame(
            time::Tick::ZERO,
            &mut render::FileSources::default(),
            &hooks,
        )?
        .rgba8(13, &hooks)?;
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [size as usize, size as usize],
        &image.bytes,
    ))
}
