//! Applies text commands to a candidate project.
use super::{
    Arc, Command, CreatedId, EditError, ItemId, ItemParams, NewItem, Placement, ProjectData,
    Result, SourceRange, Tick, TrackId, TrackKind, TrackPlacement, TrimSources, apply, item,
    set_range,
};

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
    cues: Vec<captions::AssCue>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    if cues.is_empty() || cues.len() > captions::ass::MAX_ASS_CUES {
        return Err(EditError::Limit);
    }
    if start < Tick::ZERO {
        return Err(EditError::Invalid("negative output start"));
    }
    let asset_id = asset.id();
    let mut end = Tick::ZERO;
    let mut converted = Vec::with_capacity(cues.len());
    for (index, cue) in cues.into_iter().enumerate() {
        if !cue.style.is_valid() || cue.range.start() < Tick::ZERO {
            return Err(EditError::Invalid("imported cue style or timing"));
        }
        end = end.max(cue.range.end());
        let id = format!("ass-{}", index.saturating_add(1)).into();
        converted.push(captions::Cue {
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
                    cues: captions::CueTrack { cues: converted },
                }),
            },
        },
        created,
        next,
        sources,
        copied_text_bytes,
    )?
    .into_data();
    // Per-cue styles are large: a result that cannot be encoded could not be
    // saved or re-opened, so refuse it before the session accepts it.
    project::encode(&project::Project::new(data.clone())?, false).map_err(|_| EditError::Limit)?;
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
        super::clips::WordCut::Words(&selected),
        created,
        next,
        sources,
    )
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
        || texts.len() > ai::MAX_SEGMENTS
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
