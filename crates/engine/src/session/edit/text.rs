//! Applies text commands to a candidate project.
use super::{
    Arc, BTreeMap, BTreeSet, ClipboardRefs, Command, CreatedId, EditError, Item, ItemId,
    ItemParams, NewItem, Placement, ProjectData, Result, SourceRange, Tick, TrackId, TrackKind,
    TrackPlacement, TrimSources, apply, item, set_range,
};

/// Change only the opt-in. Timing, copied styles and authoritative trim sources stay intact.
pub(super) fn set_word_highlight(
    mut data: ProjectData,
    id: ItemId,
    enabled: bool,
) -> Result<ProjectData> {
    if item_in_other_scene(&data, id) {
        return Err(EditError::SceneBoundary);
    }
    let target = item(&mut data, id)?;
    let ItemParams::Caption(params) = &mut target.content else {
        return Err(EditError::Invalid("word highlight requires caption item"));
    };
    params.word_highlight = enabled;
    Ok(data)
}

/// Upper bound on cues one selection may name.
const MAX_EMPHASIS_CUES: usize = 4096;

fn caption_params(data: &mut ProjectData, id: ItemId) -> Result<&mut project::CaptionParams> {
    if item_in_other_scene(data, id) {
        return Err(EditError::SceneBoundary);
    }
    match &mut item(data, id)?.content {
        ItemParams::Caption(params) => Ok(params),
        _ => Err(EditError::Invalid("emphasis requires caption item")),
    }
}

fn find_cue<'a>(
    params: &'a project::CaptionParams,
    cue: &captions::NfcText,
) -> Result<&'a captions::Cue> {
    params
        .cues
        .cues
        .iter()
        .find(|value| &value.id == cue)
        .ok_or(EditError::Invalid("emphasis cue"))
}

/// Choose emphasis words for the named cues with the deterministic lexicon. Every cue must
/// carry verified word timing, otherwise the whole command refuses with the typed
/// `NoWordTiming` error and nothing changes. Selections replace earlier ones for those cues.
pub(super) fn select_emphasis(
    mut data: ProjectData,
    id: ItemId,
    cue_ids: &[captions::NfcText],
    mode: crate::EmphasisMode,
) -> Result<ProjectData> {
    let crate::EmphasisMode::Lexicon = mode;
    if cue_ids.is_empty() || cue_ids.len() > MAX_EMPHASIS_CUES {
        return Err(EditError::Invalid("emphasis cue ids"));
    }
    let params = caption_params(&mut data, id)?;
    let mut chosen = Vec::with_capacity(cue_ids.len());
    for cue_id in cue_ids {
        let cue = find_cue(params, cue_id)?;
        let lexicon = captions::EmphasisLexicon::builtin(&cue.source_lang).ok();
        chosen.push((
            cue_id.clone(),
            captions::select_lexicon(cue, lexicon.as_ref())?,
        ));
    }
    for (cue_id, ids) in chosen {
        if ids.is_empty() {
            params.emphasis.remove(&cue_id);
        } else {
            params.emphasis.insert(cue_id, ids);
        }
    }
    Ok(data)
}

/// Replace one cue's emphasis with explicit word ids; an empty list clears it. Ids must be
/// ascending, unique, at most three, and verified timed words of the cue's current source text.
pub(super) fn set_emphasis(
    mut data: ProjectData,
    id: ItemId,
    cue_id: &captions::NfcText,
    word_ids: Vec<u32>,
) -> Result<ProjectData> {
    let params = caption_params(&mut data, id)?;
    if word_ids.is_empty() {
        find_cue(params, cue_id)?;
        params.emphasis.remove(cue_id);
        return Ok(data);
    }
    captions::validate_selection(find_cue(params, cue_id)?, &word_ids)?;
    params.emphasis.insert(cue_id.clone(), word_ids);
    Ok(data)
}

pub(super) fn execute_apply_caption_style(
    mut data: ProjectData,
    id: ItemId,
    style: crate::CaptionStyleApply,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    if item_in_other_scene(&data, id) {
        return Err(EditError::SceneBoundary);
    }
    // The ordinary lock and kind guard precedes equality or preset lookup.
    {
        let item = item(&mut data, id)?;
        if !matches!(item.content, ItemParams::Caption(_)) {
            return Err(EditError::Invalid("style requires caption item"));
        }
    }
    if let crate::CaptionStyleApply::Saved { preset, overrides } = &style {
        let base = data
            .caption_presets
            .iter()
            .find(|value| value.id == *preset)
            .ok_or(EditError::Invalid("caption preset not found"))?
            .style
            .clone();
        let mut effective = base;
        overrides.apply(&mut effective);
        if !effective.is_valid() {
            return Err(EditError::Invalid("caption style"));
        }
        let item = item(&mut data, id)?;
        let ItemParams::Caption(params) = &mut item.content else {
            return Err(EditError::Invalid("style requires caption item"));
        };
        if sources
            .get(&id)
            .map_or(&params.cues, AsRef::as_ref)
            .cues
            .is_empty()
        {
            return Err(EditError::Invalid("empty caption item"));
        }
        if params.preset == Some(*preset) && params.overrides == *overrides {
            return Ok(data);
        }
        params.preset = Some(*preset);
        params.overrides = overrides.clone();
        return Ok(data);
    }
    let style = style
        .materialized()
        .ok_or(EditError::Invalid("caption style kind"))?;
    if !style.is_valid() {
        return Err(EditError::Invalid("caption style"));
    }
    let item = item(&mut data, id)?;
    let ItemParams::Caption(params) = &mut item.content else {
        return Err(EditError::Invalid("style requires caption item"));
    };
    let source = sources.get(&id).map_or(&params.cues, AsRef::as_ref);
    if source.cues.is_empty() {
        return Err(EditError::Invalid("empty caption item"));
    }
    if params.preset.is_none()
        && params.overrides.is_empty()
        && source
            .cues
            .iter()
            .all(|cue| source.style_of(cue) == Some(&style))
    {
        return Ok(data);
    }
    params.preset = None;
    params.overrides = captions::CaptionOverrides::default();
    let source = match sources.get_mut(&id) {
        Some(source) => Arc::make_mut(source),
        None => &mut params.cues,
    };
    source.styles = vec![style];
    for cue in &mut source.cues {
        cue.style = Some(captions::CueStyleId(0));
    }
    set_range(item, item.source_range, Tick::ZERO, sources)?;
    Ok(data)
}

pub(super) fn save_caption_preset(
    mut data: ProjectData,
    id: ItemId,
    name: captions::NfcText,
    sources: &TrimSources,
    next: &mut Option<u64>,
    created: &mut Vec<CreatedId>,
) -> Result<ProjectData> {
    if item_in_other_scene(&data, id) {
        return Err(EditError::SceneBoundary);
    }
    {
        let target = item(&mut data, id)?;
        if !matches!(target.content, ItemParams::Caption(_)) {
            return Err(EditError::Invalid("preset requires caption item"));
        }
    }
    let params = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find_map(|item| {
            if item.id == id
                && let ItemParams::Caption(params) = &item.content
            {
                Some(params)
            } else {
                None
            }
        })
        .ok_or(EditError::Invalid("preset requires caption item"))?;
    let style = if let Some(selected) = params.preset {
        let mut style = data
            .caption_presets
            .iter()
            .find(|preset| preset.id == selected)
            .ok_or(EditError::Invalid("caption preset not found"))?
            .style
            .clone();
        params.overrides.apply(&mut style);
        style
    } else {
        let source = sources.get(&id).map_or(&params.cues, AsRef::as_ref);
        let uniform = source
            .cues
            .first()
            .and_then(|cue| source.style_of(cue))
            .filter(|style| {
                source
                    .cues
                    .iter()
                    .all(|cue| source.style_of(cue) == Some(*style))
            })
            .ok_or(EditError::Invalid(
                "preset requires uniform saved caption style",
            ))?;
        render::style_from_cue(uniform)
    };
    check_preset_name(&data, &name, None)?;
    if data.caption_presets.len() >= project::MAX_CAPTION_PRESETS {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    let id = project::CaptionPresetId(super::alloc(next)?);
    data.caption_presets
        .push(project::SavedCaptionPreset { id, name, style });
    created.push(CreatedId::CaptionPreset(id));
    Ok(data)
}

fn check_preset_name(
    data: &ProjectData,
    name: &captions::NfcText,
    except: Option<project::CaptionPresetId>,
) -> Result<()> {
    if !project::SavedCaptionPreset::valid_name(name)
        || data
            .caption_presets
            .iter()
            .any(|p| Some(p.id) != except && p.name == *name)
    {
        return Err(EditError::Invalid("caption preset name"));
    }
    Ok(())
}

pub(super) fn save_text_preset(
    mut data: ProjectData,
    id: ItemId,
    name: captions::NfcText,
    next: &mut Option<u64>,
    created: &mut Vec<CreatedId>,
) -> Result<ProjectData> {
    if item_in_other_scene(&data, id) {
        return Err(EditError::SceneBoundary);
    }
    let target = item(&mut data, id)?;
    let ItemParams::Text(params) = &target.content else {
        return Err(EditError::Invalid("preset requires text item"));
    };
    let style = project::TextStyle::from_params(params);
    check_text_preset_name(&data.text_presets, &name)?;
    if data.text_presets.len() >= project::MAX_TEXT_PRESETS {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    let id = project::TextPresetId(super::alloc(next)?);
    data.text_presets
        .push(project::SavedTextPreset { id, name, style });
    created.push(CreatedId::TextPreset(id));
    Ok(data)
}

fn check_text_preset_name(
    presets: &[project::SavedTextPreset],
    name: &captions::NfcText,
) -> Result<()> {
    if !project::SavedTextPreset::valid_name(name)
        || presets.iter().any(|preset| preset.name == *name)
    {
        return Err(EditError::Invalid("text preset name"));
    }
    Ok(())
}

/// Copy one saved style into every named unlocked text item as one atomic edit.
/// A non-text, missing or locked item refuses the whole command.
pub(super) fn apply_text_preset(
    mut data: ProjectData,
    items: Vec<ItemId>,
    preset: crate::TextPresetApply,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty text preset selection"));
    }
    let mut remaining: BTreeSet<_> = items.iter().copied().collect();
    if remaining.len() != items.len() {
        return Err(EditError::Invalid("duplicate text preset selection"));
    }
    let style = match preset {
        crate::TextPresetApply::Saved { preset } => data
            .text_presets
            .iter()
            .find(|value| value.id == preset)
            .map(|value| value.style.clone())
            .ok_or(EditError::Invalid("text preset not found"))?,
        crate::TextPresetApply::Look { look } => look.style(),
    };
    for track in data.scene_mut().timeline.iter_mut() {
        let matched = track
            .items
            .iter()
            .any(|target| remaining.contains(&target.id));
        if !matched {
            continue;
        }
        if track.locked {
            return Err(EditError::Locked(track.id));
        }
        for target in &mut track.items {
            if !remaining.remove(&target.id) {
                continue;
            }
            let ItemParams::Text(params) = &mut target.content else {
                return Err(EditError::Invalid("text preset requires text items"));
            };
            style.apply_to(params);
        }
    }
    if let Some(id) = remaining.into_iter().next() {
        return Err(if item_in_other_scene(&data, id) {
            EditError::SceneBoundary
        } else {
            EditError::NoItem(id)
        });
    }
    Ok(data)
}

pub(super) fn rename_caption_preset(
    mut data: ProjectData,
    id: project::CaptionPresetId,
    name: captions::NfcText,
) -> Result<ProjectData> {
    check_preset_name(&data, &name, Some(id))?;
    let preset = data
        .caption_presets
        .iter_mut()
        .find(|preset| preset.id == id)
        .ok_or(EditError::Invalid("caption preset not found"))?;
    preset.name = name;
    Ok(data)
}

pub(super) fn delete_caption_preset(
    mut data: ProjectData,
    id: project::CaptionPresetId,
) -> Result<ProjectData> {
    let index = data
        .caption_presets
        .iter()
        .position(|preset| preset.id == id)
        .ok_or(EditError::Invalid("caption preset not found"))?;
    // A live selection never dangles: any caption item in any scene keeps its look.
    if data.scenes.iter().any(|scene| {
        scene
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .any(|item| matches!(&item.content, ItemParams::Caption(p) if p.preset == Some(id)))
    }) {
        return Err(EditError::Invalid("caption preset in use"));
    }
    data.caption_presets.remove(index);
    Ok(data)
}

/// Import parsed ASS cues as one caption item on a text track. The asset,
/// optional track allocation and item insertion form one command and therefore
/// one undo step. File parsing and asset probing stay with the adapter.
#[allow(clippy::too_many_arguments)]
pub(super) fn import_captions(
    data: ProjectData,
    asset: media::SourceAsset,
    language: captions::LanguageTag,
    start: Tick,
    track: Option<TrackId>,
    name: Option<captions::NfcText>,
    styles: Vec<captions::CueStyle>,
    cues: Vec<captions::AssCue>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    if cues.is_empty() || cues.len() > captions::ass::MAX_ASS_CUES {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    if styles.iter().any(|style| !style.is_valid()) {
        return Err(EditError::Invalid("imported cue style"));
    }
    if styles.len() > captions::MAX_CUE_STYLES {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    if start < Tick::ZERO {
        return Err(EditError::Invalid("negative output start"));
    }
    let mut end = Tick::ZERO;
    let mut cue_track = captions::CueTrack {
        styles,
        cues: Vec::with_capacity(cues.len()),
    };
    for (index, cue) in cues.into_iter().enumerate() {
        if usize::from(cue.style.0) >= cue_track.styles.len() || cue.range.start() < Tick::ZERO {
            return Err(EditError::Invalid("imported cue style or timing"));
        }
        end = end.max(cue.range.end());
        let id = format!("ass-{}", index.saturating_add(1)).into();
        cue_track.cues.push(captions::Cue {
            word_timing: captions::CueWordTiming::Unverified,
            id,
            range: cue.range,
            source_lang: language.clone(),
            texts: std::collections::BTreeMap::from([(
                language.clone(),
                captions::CueText {
                    text: cue.text,
                    edited: false,
                    translation_ref: None,
                },
            )]),
            style: Some(cue.style),
        });
    }
    let source_range = SourceRange::new(Tick::ZERO, end)?;
    add_caption_item(
        data,
        asset,
        start,
        track,
        name,
        source_range,
        cue_track,
        created,
        next,
        sources,
        copied_text_bytes,
    )
}

/// The ephemeral asset, the text track (existing or allocated) and the caption item over
/// `source_range`, as one candidate. Import and manual entry share it.
#[allow(clippy::too_many_arguments)]
fn add_caption_item(
    mut data: ProjectData,
    asset: media::SourceAsset,
    start: Tick,
    track: Option<TrackId>,
    name: Option<captions::NfcText>,
    source_range: SourceRange,
    cue_track: captions::CueTrack,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    let asset_id = asset.id();
    data = apply(
        data,
        Command::AddAsset { asset },
        created,
        &mut Vec::new(),
        next,
        sources,
        copied_text_bytes,
        ClipboardRefs::default(),
    )?
    .into_data();
    let target = match track {
        Some(track) => {
            let value = data
                .scene()
                .timeline
                .iter()
                .find(|value| value.id == track)
                .ok_or(EditError::Invalid("import track not found"))?;
            if value.kind != TrackKind::Text {
                return Err(EditError::Invalid("import requires a text track"));
            }
            track
        }
        None => {
            let mark = created.len();
            data = apply(
                data,
                Command::AddTrack {
                    name: name.unwrap_or_else(|| "Captions".into()),
                    kind: TrackKind::Text,
                    placement: TrackPlacement::Auto,
                },
                created,
                &mut Vec::new(),
                next,
                sources,
                copied_text_bytes,
                ClipboardRefs::default(),
            )?
            .into_data();
            created
                .get(mark..)
                .unwrap_or_default()
                .iter()
                .find_map(|id| match id {
                    CreatedId::Track(track) => Some(*track),
                    _ => None,
                })
                .ok_or(EditError::Invalid("import track was not created"))?
        }
    };
    data = apply(
        data,
        Command::InsertItem {
            placement: Placement::Track { track: target },
            item: NewItem {
                retime: Default::default(),
                animations: project::Animations::default(),
                source_asset: Some(asset_id),
                source_range,
                start,
                content: ItemParams::Caption(project::CaptionParams {
                    emphasis: Default::default(),
                    word_highlight: false,
                    preset: None,
                    overrides: captions::CaptionOverrides::default(),
                    cues: cue_track,
                }),
            },
        },
        created,
        &mut Vec::new(),
        next,
        sources,
        copied_text_bytes,
        ClipboardRefs::default(),
    )?
    .into_data();
    // Imported content must survive a save: a result that cannot be encoded
    // could not be saved or re-opened, so refuse it before the session accepts it.
    project::encode(&project::Project::new(data.clone())?, false)
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    Ok(data)
}
/// Manual entry: one caption item with no cues over `duration` from `start`, with its
/// internal asset and optional new track, as one command and one undo step. Cues are added
/// afterwards through the ordinary item-parameter edit.
#[allow(clippy::too_many_arguments)]
pub(super) fn add_empty_captions(
    data: ProjectData,
    asset: media::SourceAsset,
    start: Tick,
    duration: Tick,
    track: Option<TrackId>,
    name: Option<captions::NfcText>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    if start < Tick::ZERO {
        return Err(EditError::Invalid("negative output start"));
    }
    if duration <= Tick::ZERO {
        return Err(EditError::Invalid("caption duration must be positive"));
    }
    if !asset.ephemeral() {
        return Err(EditError::Invalid("manual captions need an internal asset"));
    }
    add_caption_item(
        data,
        asset,
        start,
        track,
        name,
        SourceRange::new(Tick::ZERO, duration)?,
        captions::CueTrack::default(),
        created,
        next,
        sources,
        copied_text_bytes,
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_import_captions(
    mut data: ProjectData,
    asset: media::SourceAsset,
    language: captions::LanguageTag,
    start: Tick,
    track: Option<TrackId>,
    name: Option<captions::NfcText>,
    styles: Vec<captions::CueStyle>,
    cues: Vec<captions::AssCue>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    data = import_captions(
        data,
        asset,
        language,
        start,
        track,
        name,
        styles,
        cues,
        created,
        next,
        sources,
        copied_text_bytes,
    )?;

    Ok(data)
}
/// Applies transcript word cuts as reviewed output spans. Selected words are
/// removed from every track through the shared reviewed-span cutter; an
/// unselected word inside a span, an edited cue or another scene refuses.
pub(super) fn execute_transcript_delete(
    data: ProjectData,
    word_ids: Vec<crate::WordId>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let all = super::super::transcript::scene_words(data.scene())?;
    let selected = super::super::transcript::selected_words(&data, &all, &word_ids)?;
    cut_selected_words(data, &selected, created, next, sources)
}

/// Removes reviewed filler candidates (lexicon or LLM-classified) as measured
/// words through the same cutter and ripple path as a word-range cut. Every id
/// must still name a visible candidate or measured word of the active scene; an
/// edited cue refuses without `confirm_edited`.
pub(super) fn execute_remove_fillers(
    data: ProjectData,
    ids: Vec<captions::NfcText>,
    confirm_edited: bool,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    if ids.is_empty() {
        return Err(EditError::Invalid("empty filler selection"));
    }
    let mut unique = BTreeSet::new();
    if ids.iter().any(|id| !unique.insert(id.clone())) {
        return Err(EditError::Invalid("duplicate filler"));
    }
    let candidates: BTreeMap<_, _> = super::super::filler::scene_candidates(&data, None, true)?
        .into_iter()
        .map(|candidate| (candidate.id.clone(), candidate))
        .collect();
    let visible = if ids.iter().any(|id| !candidates.contains_key(id)) {
        Some(super::super::filler::visible_words(&data)?)
    } else {
        None
    };
    let mut selected = BTreeSet::new();
    for id in &ids {
        if let Some(candidate) = candidates.get(id) {
            if candidate.edited_cue && !confirm_edited {
                return Err(EditError::Protected);
            }
            selected.extend(candidate.words().iter().cloned());
            continue;
        }
        // LLM classification may name lexicon-independent words. The reviewed ID
        // must still address one visible measured word of this scene.
        let (word, edited) = visible
            .as_ref()
            .and_then(|visible| visible.get(id))
            .ok_or(EditError::Invalid("stale filler candidate"))?;
        if *edited && !confirm_edited {
            return Err(EditError::Protected);
        }
        selected.insert(word.clone());
    }
    cut_selected_words(data, &selected, created, next, sources)
}

/// Remove exactly `selected` measured words as reviewed output spans. Consecutive
/// selected words join one span; an unselected word inside a run refuses the edit.
/// The cut touches every item any span overlaps, on every track, so a locked track
/// among them refuses before the cutter changes anything.
fn cut_selected_words(
    data: ProjectData,
    selected: &BTreeSet<crate::WordId>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let all = super::super::transcript::scene_words(data.scene())?;
    let mut spans: Vec<SourceRange> = Vec::new();
    let mut run: Option<(Tick, Tick)> = None;
    for word in &all {
        let id = crate::WordId {
            item: word.item,
            cue: word.cue.clone(),
            word: word.index,
        };
        if selected.contains(&id) {
            run = Some(match run {
                Some((start, end)) if word.output.start() <= end => {
                    (start, end.max(word.output.end()))
                }
                Some((start, end)) => {
                    spans.push(SourceRange::new(start, end)?);
                    (word.output.start(), word.output.end())
                }
                None => (word.output.start(), word.output.end()),
            });
        } else if let Some((start, end)) = run.take() {
            if word.output.start() < end {
                return Err(EditError::Invalid("transcript delete crosses a word"));
            }
            spans.push(SourceRange::new(start, end)?);
        }
    }
    if let Some((start, end)) = run {
        spans.push(SourceRange::new(start, end)?);
    }
    refuse_locked(&data, |item| {
        let end = item.end()?;
        Ok(spans
            .iter()
            .any(|span| item.start < span.end() && span.start() < end))
    })?;
    super::clips::remove_output_ranges(
        data,
        &spans,
        0,
        super::clips::WordCut::Words(selected),
        created,
        next,
        sources,
    )
}

/// Merges adjacent cues of one caption item into the earliest cue. Each language
/// keeps its joined text, measured word timings concatenate in cue order, and the
/// source cue track is rebuilt through the mapping owner so trimmed and repeated
/// spans stay consistent. Any refused selection changes nothing.
pub(super) fn execute_transcript_merge(
    mut data: ProjectData,
    cue_ids: Vec<crate::CueId>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    if cue_ids.len() < 2 {
        return Err(EditError::Invalid("merge cue count"));
    }
    let mut unique = std::collections::BTreeSet::new();
    if cue_ids.iter().any(|id| !unique.insert(id.clone())) {
        return Err(EditError::Invalid("duplicate merge cue"));
    }
    let item_id = cue_ids
        .first()
        .ok_or(EditError::Invalid("merge cue count"))?
        .item;
    let track_id = item_track(&data, item_id).ok_or_else(|| {
        if item_in_other_scene(&data, item_id) {
            EditError::SceneBoundary
        } else {
            EditError::NoItem(item_id)
        }
    })?;
    for id in cue_ids.iter().skip(1) {
        let other = item_track(&data, id.item).ok_or_else(|| {
            if item_in_other_scene(&data, id.item) {
                EditError::SceneBoundary
            } else {
                EditError::NoItem(id.item)
            }
        })?;
        if other != track_id {
            return Err(EditError::CuesAcrossTracks);
        }
        if id.item != item_id {
            return Err(EditError::CuesAcrossItems);
        }
    }
    refuse_locked(&data, |item| Ok(item.id == item_id))?;

    let retained = sources.contains_key(&item_id);
    let item = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find(|item| item.id == item_id)
        .ok_or(EditError::NoItem(item_id))?;
    let cues = if retained {
        match sources.get_mut(&item_id) {
            Some(source) => Arc::make_mut(source),
            None => return Err(EditError::NoItem(item_id)),
        }
    } else {
        let ItemParams::Caption(params) = &mut item.content else {
            return Err(EditError::Invalid("merge requires caption item"));
        };
        &mut params.cues
    };
    merge_adjacent(cues, &cue_ids)?;
    // Rebuild the visible view from the authoritative source track: clipping,
    // repeated occurrences and fragment identities stay with the mapping owner.
    set_range(item, item.source_range, Tick::ZERO, sources)?;
    Ok(data)
}

/// Merge the selected cue identities inside one source cue track.
fn merge_adjacent(cues: &mut captions::CueTrack, cue_ids: &[crate::CueId]) -> Result<()> {
    // Resolve identities to time-ordered references; input order is irrelevant.
    let mut selected = Vec::with_capacity(cue_ids.len());
    for id in cue_ids {
        let index = cues
            .cues
            .iter()
            .position(|cue| cue.id == id.cue)
            .ok_or(EditError::Invalid("merge cue"))?;
        let start = cues
            .cues
            .get(index)
            .ok_or(EditError::Invalid("merge cue"))?
            .range
            .start();
        selected.push((start, index));
    }
    selected.sort_by_key(|(start, _)| *start);
    let selected: Vec<&captions::Cue> = selected
        .iter()
        .map(|(_, index)| cues.cues.get(*index).ok_or(EditError::Invalid("merge cue")))
        .collect::<Result<_>>()?;
    for pair in selected.windows(2) {
        let [left, right] = pair else {
            continue;
        };
        if left.range.end() != right.range.start() {
            return Err(EditError::CuesNotAdjacent);
        }
    }
    if selected
        .iter()
        .any(|cue| cue.source_text().is_some_and(|text| text.edited))
    {
        return Err(EditError::Protected);
    }
    let first = selected
        .first()
        .ok_or(EditError::Invalid("merge cue count"))?;
    let source_lang = first.source_lang.clone();
    if selected.iter().any(|cue| cue.source_lang != source_lang) {
        return Err(EditError::Invalid("merge cue source language"));
    }
    let timed = selected
        .iter()
        .filter(|cue| !cue.word_ranges().is_empty())
        .count();
    if timed != 0 && timed != selected.len() {
        return Err(EditError::Invalid("merge cue word timings"));
    }
    let mut texts = std::collections::BTreeMap::new();
    let languages: std::collections::BTreeSet<_> = selected
        .iter()
        .flat_map(|cue| cue.texts.keys().cloned())
        .collect();
    for language in languages {
        let mut text = String::new();
        let mut edited = false;
        let mut reference: Option<Option<captions::NfcText>> = None;
        for cue in &selected {
            let Some(value) = cue.texts.get(&language) else {
                continue;
            };
            if !text.is_empty() && !value.text.as_str().is_empty() && language.joins_with_space() {
                text.push(' ');
            }
            text.push_str(value.text.as_str());
            edited |= value.edited;
            match &reference {
                None => reference = Some(value.translation_ref.clone()),
                Some(current) if current == &value.translation_ref => {}
                Some(_) => reference = Some(None),
            }
        }
        texts.insert(
            language,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: reference.flatten(),
            },
        );
    }
    let last = selected
        .last()
        .ok_or(EditError::Invalid("merge cue count"))?;
    let range = SourceRange::new(first.range.start(), last.range.end())?;
    let word_timing = captions::CueWordTiming::joined(
        &selected,
        if source_lang.joins_with_space() {
            " "
        } else {
            ""
        },
    )?;
    let merged = captions::Cue {
        word_timing,
        id: first.id.clone(),
        range,
        source_lang,
        texts,
        style: first.style,
    };
    drop(selected);
    let selected: std::collections::BTreeSet<_> = cue_ids.iter().map(|id| id.cue.clone()).collect();
    cues.cues.retain(|cue| !selected.contains(&cue.id));
    let at = cues
        .cues
        .iter()
        .position(|cue| cue.range.start() > range.start())
        .unwrap_or(cues.cues.len());
    cues.cues.insert(at, merged);
    Ok(())
}

/// The one lock check every transcript edit runs before it changes anything: refuses
/// with `Locked` when any item `affected` selects sits on a locked track.
fn refuse_locked(
    data: &ProjectData,
    mut affected: impl FnMut(&Item) -> Result<bool>,
) -> Result<()> {
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            if track.locked && affected(item)? {
                return Err(EditError::Locked(track.id));
            }
        }
    }
    Ok(())
}

fn item_track(data: &ProjectData, id: ItemId) -> Option<TrackId> {
    data.scene().timeline.iter().find_map(|track| {
        track
            .items
            .iter()
            .any(|item| item.id == id)
            .then_some(track.id)
    })
}

fn item_in_other_scene(data: &ProjectData, id: ItemId) -> bool {
    let scene = data.scene().id;
    data.scenes
        .iter()
        .filter(|other| other.id != scene)
        .flat_map(|other| other.timeline.iter())
        .flat_map(|track| &track.items)
        .any(|item| item.id == id)
}

pub(super) fn execute_retranslate(
    mut data: ProjectData,
    id: ItemId,
    target_language: captions::LanguageTag,
    texts: std::collections::BTreeMap<captions::NfcText, captions::NfcText>,
    translation_ref: captions::NfcText,
    force: bool,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    if texts.is_empty()
        || texts.len() > crate::session::wire::MAX_TRANSLATION_CUES
        || translation_ref.as_str().trim().is_empty()
        || translation_ref.as_str().len() > 256
    {
        return Err(EditError::Invalid("translation application"));
    }
    let item = item(&mut data, id)?;
    let ItemParams::Caption(params) = &mut item.content else {
        return Err(EditError::Invalid("translation requires caption item"));
    };
    if texts
        .keys()
        .any(|id| !params.cues.cues.iter().any(|c| &c.id == id))
        || texts.values().any(|t| t.as_str().trim().is_empty())
    {
        return Err(EditError::Invalid("translation cue ids or text"));
    }
    // A selected text edit does not replace the full caption content.
    // Update the retained track, then let the mapping owner rebuild the
    // clipped view so reversal can restore hidden cues and original ranges.
    let retained = sources.contains_key(&id);
    let cues = match sources.get_mut(&id) {
        Some(source) => Arc::make_mut(source),
        None => &mut params.cues,
    };
    if texts
        .keys()
        .any(|id| !cues.cues.iter().any(|cue| &cue.id == id))
    {
        return Err(EditError::Invalid("missing retained translation cue"));
    }
    // A retranslated cue drops its emphasis: ids chosen against the old bilingual text are
    // never reused. Text and word timing stay as they were.
    for id in texts.keys() {
        params.emphasis.remove(id);
    }
    for cue in &mut cues.cues {
        if let Some(text) = texts.get(&cue.id) {
            if cue.source_lang == target_language {
                return Err(EditError::Invalid("cannot replace source with translation"));
            }
            cue.set_text(
                target_language.clone(),
                captions::CueText {
                    text: text.clone(),
                    edited: false,
                    translation_ref: Some(translation_ref.clone()),
                },
                force,
            );
        }
    }
    if retained {
        set_range(item, item.source_range, Tick::ZERO, sources)?;
    }

    Ok(data)
}
