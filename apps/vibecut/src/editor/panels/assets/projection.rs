//! Prepares the area's read-only workflow projection before rendering.
use super::effects::preview_instance;
use super::stickers;
use super::{
    AssetRow, AssetState, AssetsTab, BTreeMap, Editor, EffectRow, Icon, MediaKind, Model, Row,
    Text, asset_row, egui, file_name,
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
    if let Some(previews) = &mut editor.assets.effect_previews {
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
            let params = preview_instance(definition);
            if let (Some(previews), Some(params)) =
                (&mut editor.assets.effect_previews, params.as_ref())
            {
                previews.ensure(definition.id, params, &project::Animations::default());
            }
            let status = editor
                .assets
                .effect_previews
                .as_ref()
                .and_then(|previews| previews.status(definition.id));
            let texture = editor
                .assets
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
            | AssetsTab::Adjustment
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
        insertion_ready: editor.insertion_available(),
        stickers: (preferences.assets_tab == AssetsTab::Stickers).then(|| {
            let query = editor
                .assets
                .searches
                .get(&AssetsTab::Stickers)
                .map_or("", String::as_str);
            stickers::rows(
                query,
                editor.assets.sticker_tab,
                editor.assets.sticker_expanded,
                &editor.preferences.sticker_recents,
                editor.preferences.locale,
            )
        }),
        importing: editor.assets.importing(),
        selection: editor.assets.media_selection.clone(),
        clip_export: editor.assets.clip_export_status(),
        reveal: editor.assets.media_reveal.map(|reveal| reveal.asset),
        marquee: editor.assets.media_marquee.clone(),
        failed: !editor.assets.import_failures.is_empty() || editor.assets.import_notice.is_some(),
        sounds: editor.assets.sounds.as_ref().map(|sounds| sounds.view()),
        music: editor.music.as_ref().map(|music| music.view()),
        music_credits,
        music_credits_failed,
    };
}
