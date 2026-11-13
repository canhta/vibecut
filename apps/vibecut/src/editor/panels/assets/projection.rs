//! Prepares the area's read-only workflow projection before rendering.
use super::effects::{EffectPreviews, preview_instance};
use super::stickers;
use super::{
    AssetRow, AssetState, AssetsTab, BTreeMap, EffectRow, FilterRow, Icon, MediaKind,
    MediaSection, Model, Row, SceneRow, State, Text, asset_row, egui, file_name,
};
use crate::editor::Context;

/// Build the read-only projection the view draws. `now` is the editor clock and
/// `insertion_ready` the frame's answer for whether an insertion is allowed now.
pub(in crate::editor) fn prepare(
    state: &mut State,
    cx: &Context<'_>,
    ctx: &egui::Context,
    now: std::time::Duration,
    insertion_ready: bool,
) {
    let rows = if !matches!(
        cx.preferences.assets_tab,
        AssetsTab::Text
            | AssetsTab::Effects
            | AssetsTab::Filters
            | AssetsTab::Transitions
            | AssetsTab::Captions
    ) {
        state.refresh_media_sizes(cx.session);
        let sizes = state.media_size_map();
        let order = state.media_order(cx.session, cx.preferences);
        let by_id: BTreeMap<_, _> = cx
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
                let status = state
                    .asset_states
                    .get(&id)
                    .copied()
                    .unwrap_or(AssetState::Probing);
                Some(Row {
                    display: asset_row(asset, cx.locale, sizes.get(&id).copied().flatten(), status),
                    kind: MediaKind::of(asset),
                })
            })
            .collect();
        rows.extend(state.pending_imports.iter().map(|(id, path)| Row {
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
            state
                .thumbnail_texture(ctx, row.display.id)
                .map(|texture| (row.display.id, texture))
        })
        .collect();
    let mut effects = Vec::new();
    if let Some(previews) = &mut state.effect_previews {
        previews.update(cx.preferences.assets_tab == AssetsTab::Effects, now);
    }
    if cx.preferences.assets_tab == AssetsTab::Effects {
        ctx.request_repaint_after(std::time::Duration::from_nanos(1_000_000_000 / 30));
        for definition in project::effect_definitions()
            .iter()
            .filter(|definition| definition.shelf.in_library())
        {
            let params = preview_instance(definition);
            if let (Some(previews), Some(params)) = (&mut state.effect_previews, params.as_ref()) {
                previews.ensure(definition.id, params, &project::Animations::default());
            }
            let status = state
                .effect_previews
                .as_ref()
                .and_then(|previews| previews.status(definition.id));
            let texture = state
                .effect_previews
                .as_mut()
                .and_then(|previews| previews.texture(ctx, definition.id));
            effects.push(EffectRow {
                kind: definition.id,
                name: crate::i18n::effect_label(definition.id, None)
                    .map_or(definition.name, |key| cx.tr(key)),
                shelf: definition.shelf,
                status,
                texture,
            });
        }
    }
    let filters = filter_rows(state, cx, ctx);
    let mut preferences = cx.preferences.clone();
    if !matches!(
        preferences.assets_tab,
        AssetsTab::Media
            | AssetsTab::Sounds
            | AssetsTab::Text
            | AssetsTab::Effects
            | AssetsTab::Filters
            | AssetsTab::Transitions
            | AssetsTab::Captions
            | AssetsTab::Stickers
            | AssetsTab::Adjustment
    ) {
        preferences.assets_tab = AssetsTab::Media;
    }
    let (transition_refusal, transition_drag_enabled) =
        if preferences.assets_tab == AssetsTab::Transitions {
            let cuts = super::workflow::cuts(cx.session);
            let reason = super::workflow::selected_cut(cx.session, Some(cx.playhead), &cuts)
                .map(|cut| super::workflow::refusal(cx.session, cut))
                .unwrap_or(Some(Text::TransitionSelectCut));
            (reason, !cuts.is_empty())
        } else {
            (None, false)
        };
    let (music_credits, music_credits_failed) = match cx.session.project().music_credits() {
        Ok(credits) => (credits, false),
        Err(_) => (Vec::new(), true),
    };
    let stickers = (preferences.assets_tab == AssetsTab::Stickers).then(|| {
        let query = state
            .searches
            .get(&AssetsTab::Stickers)
            .map_or("", String::as_str);
        stickers::rows(
            query,
            state.sticker_tab,
            state.sticker_expanded,
            &cx.preferences.sticker_recents,
            cx.preferences.catalog_favorites.list("stickers"),
            cx.preferences.locale,
        )
    });
    let scenes = if preferences.assets_tab == AssetsTab::Media
        && state.media_section == MediaSection::Timeline
    {
        let data = cx.session.project().data();
        data.scenes
            .iter()
            .map(|scene| SceneRow {
                id: scene.id,
                name: scene.name.as_str().to_owned(),
                main: scene.is_main,
                active: scene.id == data.current_scene_id,
            })
            .collect()
    } else {
        Vec::new()
    };
    state.model = Model {
        preferences,
        rows,
        thumbnails,
        effects,
        filters,
        transition_refusal,
        transition_drag_enabled,
        insertion_ready,
        stickers,
        scenes,
        importing: state.importing(),
        selection: state.media_selection.clone(),
        clip_export: state.clip_export_status(),
        reveal: state.media_reveal.map(|reveal| reveal.asset),
        marquee: state.media_marquee.clone(),
        failed: !state.import_failures.is_empty() || state.import_notice.is_some(),
        sounds: state.sounds.as_ref().map(|sounds| sounds.view()),
        music: state.music.as_ref().map(|music| music.view()),
        music_credits,
        music_credits_failed,
    };
}

/// The Filters tiles. Looks are static, so their previews render once at time zero on the
/// procedural picture and stay until the tab is left; the previews exist only while it shows.
fn filter_rows(state: &mut State, cx: &Context<'_>, ctx: &egui::Context) -> Vec<FilterRow> {
    let shown = cx.preferences.assets_tab == AssetsTab::Filters;
    if !shown {
        if let Some(previews) = &mut state.filter_previews {
            previews.update(false, std::time::Duration::ZERO);
        }
        return Vec::new();
    }
    if state.filter_previews.is_none() {
        state.filter_previews = EffectPreviews::filters().ok();
    }
    let Some(previews) = &mut state.filter_previews else {
        return Vec::new();
    };
    previews.update(true, std::time::Duration::ZERO);
    if let Err(error) = previews.tick() {
        tracing::warn!(category = "filter_preview", error = %error, "preview tick failed");
    }
    let mut pending = false;
    let mut rows = Vec::new();
    for definition in project::filter_definitions() {
        let Ok(instance) = definition.instance("filter") else {
            continue;
        };
        previews.ensure(definition.id, &instance, &project::Animations::default());
        let status = previews.status(definition.id);
        pending |= status == Some(super::effects::EffectPreviewStatus::Pending);
        rows.push(FilterRow {
            id: definition.id,
            name: definition.name,
            shelf: definition.shelf,
            status,
            texture: previews.texture(ctx, definition.id),
        });
    }
    if pending {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
    rows
}
