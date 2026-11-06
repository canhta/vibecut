//! Prepares ordered-segment short projects.
use super::{
    Arc, CaptionParams, ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, ClipSubtitlePath,
    CueTrack, EditError, Item, ItemId, ItemParams, MAX_BATCH_CLIPS, MAX_BATCH_CUE_BYTES,
    MAX_CLIP_SEGMENTS, Mutex, NameSet, Options, PreparedClip, Project, ProjectData, Published,
    Tick, Timeline, Track, TrackId, TrackKind, VideoParams, extension, job_key, name_base,
};
impl ClipBatch {
    /// Validate the batch and build one derived project per short without touching
    /// media, submitting jobs or writing files. Request errors are `Invalid`;
    /// short-specific refusals name the zero-based short index.
    pub fn prepare(source: &Project, request: ClipBatchRequest) -> Result<Self, EditError> {
        validate_request(&request)?;
        let asset = source
            .data()
            .assets
            .iter()
            .find(|asset| asset.id() == request.asset)
            .ok_or(EditError::Invalid("clip source asset"))?;
        if !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == media::StreamKind::Video)
        {
            return Err(EditError::Invalid("clip source media"));
        }
        let options = Options {
            asset,
            cues: &request.cues,
            languages: &request.languages,
            formats: &request.subtitle_formats,
            directory: &request.directory,
            overwrite: request.overwrite,
            allow_degraded: request.allow_degraded,
            max_output_bytes: request.max_output_bytes,
        };
        let sources = Arc::new(source.clone());
        let mut names = NameSet::default();
        let mut cue_bytes = 0u64;
        let mut clips = Vec::new();
        clips
            .try_reserve_exact(request.shorts.len())
            .map_err(|_| EditError::Limit)?;
        for (index, short) in request.shorts.iter().enumerate() {
            clips.push(prepare_clip(
                source,
                &sources,
                &options,
                short,
                index,
                &mut names,
                &mut cue_bytes,
            )?);
        }
        let jobs = std::iter::repeat_with(|| None).take(clips.len()).collect();
        Ok(Self { clips, jobs })
    }
}

fn validate_request(request: &ClipBatchRequest) -> Result<(), EditError> {
    if request.shorts.is_empty() || request.shorts.len() > MAX_BATCH_CLIPS {
        return Err(EditError::Invalid("clip batch size"));
    }
    if !request.directory.is_dir() {
        return Err(EditError::Invalid("clip output directory"));
    }
    if request.max_output_bytes == 0 || request.max_output_bytes > (1 << 40) {
        return Err(EditError::Invalid("clip output byte ceiling"));
    }
    if request.languages.len() > 2
        || (request.languages.len() == 2 && request.languages.first() == request.languages.get(1))
    {
        return Err(EditError::Invalid("clip caption languages"));
    }
    if !request.subtitle_formats.is_empty() && request.languages.is_empty() {
        return Err(EditError::Invalid("clip subtitle languages"));
    }
    for (index, format) in request.subtitle_formats.iter().enumerate() {
        if request
            .subtitle_formats
            .get(..index)
            .is_some_and(|earlier| earlier.contains(format))
        {
            return Err(EditError::Invalid("clip subtitle formats"));
        }
    }
    Ok(())
}

fn prepare_clip(
    source: &Project,
    sources: &Arc<Project>,
    options: &Options<'_>,
    short: &ClipShort,
    index: usize,
    names: &mut NameSet,
    cue_total: &mut u64,
) -> Result<PreparedClip, EditError> {
    let refuse = |reason| EditError::ClipExport {
        short: index,
        reason,
    };
    if short.segments.is_empty() {
        return Err(refuse("no segments"));
    }
    if short.segments.len() > MAX_CLIP_SEGMENTS {
        return Err(refuse("too many segments"));
    }
    let mut ranges = Vec::new();
    ranges
        .try_reserve_exact(short.segments.len())
        .map_err(|_| EditError::Limit)?;
    let mut starts = Vec::new();
    starts
        .try_reserve_exact(short.segments.len())
        .map_err(|_| EditError::Limit)?;
    let mut cursor = Tick::ZERO;
    for segment in &short.segments {
        let range = segment.range;
        if range.start() < Tick::ZERO {
            return Err(refuse("negative source range"));
        }
        if options
            .asset
            .probe()
            .duration
            .is_some_and(|duration| range.end() > duration)
        {
            return Err(refuse("segment outside source"));
        }
        starts.push(cursor);
        cursor = cursor
            .checked_add(range.duration())
            .ok_or(EditError::Limit)?;
        ranges.push(range);
    }
    let mut ordered = ranges.clone();
    ordered.sort_by_key(|range| range.start());
    for pair in ordered.windows(2) {
        let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        if left.end() > right.start() {
            return Err(refuse("segments overlap"));
        }
    }
    let base = names.unique(&name_base(short.title.as_str(), index));
    let mut next_item = 1u64;
    let mut video_items = Vec::new();
    video_items
        .try_reserve_exact(ranges.len())
        .map_err(|_| EditError::Limit)?;
    for (range, start) in ranges.iter().zip(&starts) {
        video_items.push(Item {
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(next_item),
            source_asset: Some(options.asset.id()),
            source_range: *range,
            start: *start,
            content: ItemParams::Video(VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
            }),
        });
        next_item = next_item.checked_add(1).ok_or(EditError::IdExhausted)?;
    }
    let mut caption_items = Vec::new();
    let mut sidecar_bytes = 0u64;
    let caption_track = if options.languages.is_empty() {
        None
    } else {
        for (segment, start) in short.segments.iter().zip(&starts) {
            let mapping = captions::EditMapping::new(vec![(segment.range, segment.range.start())])?;
            let clipped = mapping.map_cues(options.cues)?;
            for cue in &clipped.cues {
                for language in options.languages {
                    if !cue.texts.contains_key(language) {
                        return Err(refuse("caption language missing"));
                    }
                }
            }
            let bytes = cue_bytes(&clipped)?;
            *cue_total = cue_total
                .checked_add(bytes)
                .filter(|total| *total <= MAX_BATCH_CUE_BYTES)
                .ok_or(EditError::Limit)?;
            sidecar_bytes = sidecar_bytes.checked_add(bytes).ok_or(EditError::Limit)?;
            if clipped.cues.is_empty() {
                continue;
            }
            caption_items.push(Item {
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: ItemId(next_item),
                source_asset: Some(options.asset.id()),
                source_range: segment.range,
                start: *start,
                content: ItemParams::Caption(CaptionParams { cues: clipped }),
            });
            next_item = next_item.checked_add(1).ok_or(EditError::IdExhausted)?;
        }
        Some(TrackId(3))
    };
    if !options.formats.is_empty() {
        let formats = u64::try_from(options.formats.len()).map_err(|_| EditError::Limit)?;
        sidecar_bytes = sidecar_bytes
            .checked_mul(16)
            .and_then(|bytes| bytes.checked_add(4096u64.checked_mul(formats)?))
            .ok_or(EditError::Limit)?;
    } else {
        sidecar_bytes = 0;
    }
    let mut timeline = Timeline::default();
    timeline
        .overlays
        .try_reserve_exact(usize::from(caption_track.is_some()))
        .map_err(|_| EditError::Limit)?;
    timeline.main = Track {
        id: TrackId(2),
        name: "Main Track".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: video_items,
    };
    if caption_track.is_some() {
        timeline.insert_track(Track {
            id: TrackId(3),
            name: "Captions".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: caption_items,
        });
    }
    let project = Project::new(ProjectData {
        id: source.data().id,
        name: short.title.clone(),
        canvas: source.data().canvas.clone(),
        original_canvas_size: source.data().original_canvas_size.clone(),
        assets: vec![options.asset.clone()],
        scenes: vec![project::Scene {
            id: project::SceneId(0),
            name: "Main scene".into(),
            is_main: true,
            timeline,
            bookmarks: Vec::new(),
        }]
        .try_into()?,
        current_scene_id: project::SceneId(0),
        settings: source.data().settings.clone(),
    })?;
    let mut settings = render::ExportSettings::from_project(&project);
    settings.render = render::RenderSettings::portrait(&project);
    settings.render.caption_languages = options.languages.to_vec();
    if let Some(crop) = short.crop {
        let placement = render::Placement {
            crop,
            ..Default::default()
        };
        for item in project
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.items)
        {
            settings.render.placements.insert(item.id, placement);
        }
    }
    settings.mode = render::ExportMode::Encode;
    settings.audio_cut_fade_ms = 10;
    settings.overwrite = if options.overwrite {
        project::atomic::Overwrite::Replace
    } else {
        project::atomic::Overwrite::Refuse
    };
    settings.allow_degraded = options.allow_degraded;
    settings.max_bytes = options.max_output_bytes;
    // Validate geometry, crop rectangles and caption style before submitting.
    render::Renderer::new(&project, settings.render.clone())?;
    let video = options.directory.join(format!("{base}.mp4"));
    let subtitles = options
        .formats
        .iter()
        .map(|format| ClipSubtitlePath {
            format: *format,
            path: options
                .directory
                .join(format!("{base}.{}", extension(*format))),
        })
        .collect();
    let input = job_key(&project, cursor, &settings, short.crop, options)?;
    let published = Arc::new(Mutex::new(Published {
        video: false,
        subtitles: vec![false; options.formats.len()],
    }));
    Ok(PreparedClip {
        project: Arc::new(project),
        settings,
        paths: ClipPaths { video, subtitles },
        caption_track,
        languages: options.languages.to_vec(),
        input,
        duration: cursor,
        sidecar_bytes,
        overwrite: options.overwrite,
        max_output_bytes: options.max_output_bytes,
        published,
        sources: sources.clone(),
    })
}

fn cue_bytes(cues: &CueTrack) -> Result<u64, EditError> {
    let mut total = 0u64;
    for cue in &cues.cues {
        let mut bytes = u64::try_from(cue.id.as_str().len() + cue.source_lang.as_str().len())
            .map_err(|_| EditError::Limit)?
            .checked_add(1024)
            .ok_or(EditError::Limit)?;
        for (language, text) in &cue.texts {
            let item = u64::try_from(
                text.text.as_str().len()
                    + language.as_str().len()
                    + text
                        .translation_ref
                        .as_ref()
                        .map_or(0, |reference| reference.as_str().len()),
            )
            .map_err(|_| EditError::Limit)?
            .checked_add(256)
            .ok_or(EditError::Limit)?;
            bytes = bytes.checked_add(item).ok_or(EditError::Limit)?;
        }
        total = total.checked_add(bytes).ok_or(EditError::Limit)?;
    }
    Ok(total)
}
