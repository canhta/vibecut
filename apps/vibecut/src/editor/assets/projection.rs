//! Prepares the area's read-only workflow projection before rendering.
use super::{
    AssetRow, AssetState, AssetsTab, BTreeMap, Editor, EffectRow, Icon, MediaKind, Model, Row,
    Text, asset_row, effect_preview_params, egui, file_name,
};

pub(in crate::editor) fn prepare(editor: &mut Editor, ctx: &egui::Context) {
    let rows = if !matches!(
        editor.preferences.assets_tab,
        AssetsTab::Text | AssetsTab::Effects | AssetsTab::Transitions | AssetsTab::Captions
    ) {
        editor.refresh_media_sizes();
        let sizes = editor.media_size_map();
        let order = editor.media_order();
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
        rows.extend(editor.pending_imports.iter().map(|(id, path)| Row {
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
                .thumbnail_texture(ctx, row.display.id)
                .map(|texture| (row.display.id, texture))
        })
        .collect();
    let mut effects = Vec::new();
    if editor.preferences.assets_tab == AssetsTab::Effects {
        for definition in project::effect_definitions() {
            let params = effect_preview_params(definition);
            if params.is_none() {
                continue;
            }
            if let (Some(previews), Some(params)) = (&mut editor.effect_previews, params.as_ref()) {
                previews.ensure(definition.kind, params);
            }
            let status = editor
                .effect_previews
                .as_ref()
                .and_then(|previews| previews.status(definition.kind));
            if matches!(
                status,
                Some(crate::effect_previews::EffectPreviewStatus::Ready)
            ) {
                if !editor.effect_textures.contains_key(definition.kind)
                    && let Some(image) = editor
                        .effect_previews
                        .as_ref()
                        .and_then(|previews| previews.image(definition.kind))
                {
                    editor.effect_textures.insert(
                        definition.kind.to_owned(),
                        crate::effect_previews::preview_texture(ctx, definition.kind, image),
                    );
                }
            } else {
                editor.effect_textures.remove(definition.kind);
            }
            effects.push(EffectRow {
                kind: definition.kind,
                name: if definition.kind == "blur" {
                    editor.tr(Text::EffectBlur)
                } else {
                    definition.name
                },
                status,
                texture: editor.effect_textures.get(definition.kind).cloned(),
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
    editor.assets.model = Model {
        preferences,
        rows,
        thumbnails,
        effects,
        transition_refusal,
        transition_drag_enabled,
        importing: editor.importing(),
        selection: editor.media_selection.clone(),
        reveal: editor.media_reveal.map(|reveal| reveal.asset),
        marquee: editor.media_marquee.clone(),
        failed: !editor.import_failures.is_empty() || editor.import_notice.is_some(),
        sounds: editor.sounds.as_ref().map(|sounds| sounds.view()),
    };
}
