//! Applies text commands to a candidate project.
use super::{
    Arc, BTreeMap, BTreeSet, Command, CreatedId, EditError, ItemId, ItemParams, NewItem, Placement,
    ProjectData, Result, SourceRange, Tick, TrackId, TrackKind, TrackPlacement, TrimSources, apply,
    item, set_range,
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

pub(super) fn execute_apply_caption_style(
    mut data: ProjectData,
    id: ItemId,
    style: crate::CaptionStyleApply,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let resolved = style.resolve(&data);
    if item_in_other_scene(&data, id) {
        return Err(EditError::SceneBoundary);
    }
    let item = item(&mut data, id)?;
    let ItemParams::Caption(params) = &mut item.content else {
        return Err(EditError::Invalid("style requires caption item"));
    };
    let style = resolved?;
    if !style.is_valid() {
        return Err(EditError::Invalid("caption style"));
    }
    let source = sources.get(&id).map_or(&params.cues, AsRef::as_ref);
    if source.cues.is_empty() {
        return Err(EditError::Invalid("empty caption item"));
    }
    if source
        .cues
        .iter()
        .all(|cue| source.style_of(cue) == Some(&style))
    {
        return Ok(data);
    }
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
    let target = item(&mut data, id)?;
    let ItemParams::Caption(params) = &target.content else {
        return Err(EditError::Invalid("preset requires caption item"));
    };
    let source = sources.get(&id).map_or(&params.cues, AsRef::as_ref);
    let style = source
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
        ))?
        .clone();
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
    data.caption_presets.remove(index);
    Ok(data)
}

/// Import parsed ASS cues as one caption item on a text track. The asset,
/// optional track allocation and item insertion form one command and therefore
/// one undo step. File parsing and asset probing stay with the adapter.
#[allow(clippy::too_many_arguments)]
pub(super) fn import_captions(
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
    let asset_id = asset.id();
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
            word_ranges: Vec::new(),
            style: Some(cue.style),
        });
    }
    let source_range = SourceRange::new(Tick::ZERO, end)?;

    data = apply(
        data,
        Command::AddAsset { asset },
        created,
        next,
        sources,
        copied_text_bytes,
        None,
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
                next,
                sources,
                copied_text_bytes,
                None,
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
                    word_highlight: false,
                    cues: cue_track,
                }),
            },
        },
        created,
        next,
        sources,
        copied_text_bytes,
        None,
    )?
    .into_data();
    // Imported content must survive a save: a result that cannot be encoded
    // could not be saved or re-opened, so refuse it before the session accepts it.
    project::encode(&project::Project::new(data.clone())?, false)
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    Ok(data)
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
    let (track_id, locked) = item_track(&data, item_id).ok_or_else(|| {
        if item_in_other_scene(&data, item_id) {
            EditError::SceneBoundary
        } else {
            EditError::NoItem(item_id)
        }
    })?;
    for id in cue_ids.iter().skip(1) {
        let (other, _) = item_track(&data, id.item).ok_or_else(|| {
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
    if locked {
        return Err(EditError::Locked(track_id));
    }

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
    let mut word_ranges = Vec::new();
    if selected.iter().any(|cue| !cue.word_ranges.is_empty()) {
        for cue in &selected {
            if cue.word_ranges.is_empty() || cue.word_ranges.len() != cue.words().len() {
                return Err(EditError::Invalid("merge cue word timings"));
            }
            word_ranges.extend_from_slice(&cue.word_ranges);
        }
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
        word_ranges,
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

fn item_track(data: &ProjectData, id: ItemId) -> Option<(TrackId, bool)> {
    data.scene().timeline.iter().find_map(|track| {
        track
            .items
            .iter()
            .any(|item| item.id == id)
            .then_some((track.id, track.locked))
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
