//! Prepares ordered-segment short projects.
use super::{
    Arc, CaptionParams, ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, ClipSubtitlePath,
    CueTrack, EngineError, Error, Item, ItemId, ItemParams, MAX_BATCH_CLIPS, MAX_BATCH_CUE_BYTES,
    MAX_CLIP_SEGMENTS, Mutex, NameSet, Options, PreparedClip, Project, ProjectData, Published,
    Tick, TimeRange, Timeline, Track, TrackId, TrackKind, VideoParams, extension, job_key,
    name_base,
};
impl ClipBatch {
    /// Validate the batch and build one derived project per short without touching
    /// media, submitting jobs or writing files. Request errors are `Invalid`;
    /// short-specific refusals name the zero-based short index.
    pub fn prepare(source: &Project, request: ClipBatchRequest) -> Result<Self, Error> {
        validate_request(&request)?;
        let asset = source
            .data()
            .assets
            .iter()
            .find(|asset| asset.id() == request.asset)
            .ok_or(EngineError::Edit(engine::EditError::Invalid(
                "clip source asset",
            )))?;
        if !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == media::StreamKind::Video)
        {
            return Err(EngineError::Edit(engine::EditError::Invalid("clip source media")).into());
        }
        let options = Options {
            asset,
            cues: &request.cues,
            shots: &request.shots,
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
            .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
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

fn validate_request(request: &ClipBatchRequest) -> Result<(), EngineError> {
    if request.shorts.is_empty() || request.shorts.len() > MAX_BATCH_CLIPS {
        return Err(EngineError::Edit(engine::EditError::Invalid(
            "clip batch size",
        )));
    }
    if !request.directory.is_dir() {
        return Err(EngineError::Edit(engine::EditError::Invalid(
            "clip output directory",
        )));
    }
    // The analysis is a partition of the source in time order; the engine caps it
    // at 4096 shots, so a longer list did not come from an analysis.
    if request.shots.len() > MAX_SHOTS
        || request.shots.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_none_or(|(a, b)| a.range.end() > b.range.start())
        })
    {
        return Err(EngineError::Edit(engine::EditError::Invalid(
            "clip shot analysis",
        )));
    }
    if request.max_output_bytes == 0 || request.max_output_bytes > (1 << 40) {
        return Err(EngineError::Edit(engine::EditError::Invalid(
            "clip output byte ceiling",
        )));
    }
    if request.languages.len() > 2
        || (request.languages.len() == 2 && request.languages.first() == request.languages.get(1))
    {
        return Err(EngineError::Edit(engine::EditError::Invalid(
            "clip caption languages",
        )));
    }
    if !request.subtitle_formats.is_empty() && request.languages.is_empty() {
        return Err(EngineError::Edit(engine::EditError::Invalid(
            "clip subtitle languages",
        )));
    }
    for (index, format) in request.subtitle_formats.iter().enumerate() {
        if request
            .subtitle_formats
            .get(..index)
            .is_some_and(|earlier| earlier.contains(format))
        {
            return Err(EngineError::Edit(engine::EditError::Invalid(
                "clip subtitle formats",
            )));
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
) -> Result<PreparedClip, Error> {
    let refuse = |reason| Error::ClipExport {
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
        .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
    let mut starts = Vec::new();
    starts
        .try_reserve_exact(short.segments.len())
        .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
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
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
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
    if short
        .layout
        .segments
        .keys()
        .any(|segment| *segment >= short.segments.len())
    {
        return Err(refuse("layout segment outside short"));
    }
    let base = names.unique(&name_base(short.title.as_str(), index));
    // A short is a portrait project: layouts place items in canvas pixels, so the
    // canvas is the output frame (`RenderSettings::portrait`).
    let canvas = project::Canvas {
        width: PORTRAIT[0],
        height: PORTRAIT[1],
        preset: Some(project::AspectPreset::Portrait),
        ..source.data().canvas.clone()
    };
    let frame = source_frame(options.asset).ok_or(refuse("source frame size"))?;
    let min_piece = canvas
        .fps
        .frame_duration()
        .map_err(|_| refuse("frame rate"))?;
    let layouts = crate::layout::plan(
        &ranges,
        &short.layout,
        options.shots,
        crate::layout::Geometry {
            canvas: [f64::from(PORTRAIT[0]), f64::from(PORTRAIT[1])],
            source: frame,
        },
        min_piece,
    );
    let mut next_item = 1u64;
    let mut video_items = Vec::new();
    let mut slot_items: Vec<Vec<Item>> = Vec::new();
    // Fill items whose crop the short's manual crop sets instead of the detected speaker.
    let mut manual_crop = Vec::new();
    for planned in &layouts {
        let (Some(segment_start), Some(segment)) =
            (starts.get(planned.segment), ranges.get(planned.segment))
        else {
            return Err(refuse("layout segment outside short"));
        };
        let start = planned
            .range
            .start()
            .checked_sub(segment.start())
            .and_then(|offset| segment_start.checked_add(offset))
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
        for (slot_number, slot) in planned.slots.iter().enumerate() {
            let mut slot = *slot;
            let manual = short.crop.is_some() && planned.layout == crate::Layout::Fill;
            if manual {
                slot.crop = None;
            }
            let id = ItemId(next_item);
            if manual {
                manual_crop.push(id);
            }
            let item = slot_item(id, options.asset.id(), planned.range, start, &slot)
                .map_err(Error::engine)?;
            match slot_number.checked_sub(1) {
                None => video_items.push(item),
                Some(extra) => {
                    while slot_items.len() <= extra {
                        slot_items.push(Vec::new());
                    }
                    if let Some(track) = slot_items.get_mut(extra) {
                        track.push(item);
                    }
                }
            }
            next_item = next_item
                .checked_add(1)
                .ok_or(EngineError::Session(engine::SessionError::IdExhausted))?;
        }
    }
    let mut caption_items = Vec::new();
    let mut sidecar_bytes = 0u64;
    let caption_track = if options.languages.is_empty() {
        None
    } else {
        for (segment, start) in short.segments.iter().zip(&starts) {
            let mapping = captions::EditMapping::new(vec![(segment.range, segment.range.start())])
                .map_err(Error::engine)?;
            let clipped = mapping.map_cues(options.cues).map_err(Error::engine)?;
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
                .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
            sidecar_bytes = sidecar_bytes
                .checked_add(bytes)
                .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
            if clipped.cues.is_empty() {
                continue;
            }
            caption_items.push(Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: ItemId(next_item),
                source_asset: Some(options.asset.id()),
                source_range: segment.range,
                start: *start,
                content: ItemParams::Caption(CaptionParams {
                    emphasis: Default::default(),
                    word_highlight: false,
                    preset: None,
                    overrides: captions::CaptionOverrides::default(),
                    cues: clipped,
                }),
            });
            next_item = next_item
                .checked_add(1)
                .ok_or(EngineError::Session(engine::SessionError::IdExhausted))?;
        }
        Some(TrackId(3))
    };
    if !options.formats.is_empty() {
        let formats = u64::try_from(options.formats.len())
            .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
        sidecar_bytes = sidecar_bytes
            .checked_mul(16)
            .and_then(|bytes| bytes.checked_add(4096u64.checked_mul(formats)?))
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
    } else {
        sidecar_bytes = 0;
    }
    let mut timeline = Timeline::default();
    timeline
        .overlays
        .try_reserve_exact(usize::from(caption_track.is_some()))
        .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
    timeline.main = Track {
        id: TrackId(2),
        name: "Main Track".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: video_items,
    };
    // Slots above the first stack on their own tracks, under the captions.
    for (items, id) in slot_items.into_iter().zip(SLOT_TRACK_FIRST..) {
        timeline.insert_track(Track {
            id: TrackId(id),
            name: format!("Layout slot {}", id - SLOT_TRACK_FIRST + 2).into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items,
        });
    }
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
        canvas,
        original_canvas_size: source.data().original_canvas_size,
        assets: vec![options.asset.clone()],
        luts: source.data().luts.clone(),
        scenes: vec![project::Scene {
            id: project::SceneId(0),
            name: "Main scene".into(),
            is_main: true,
            timeline,
            bookmarks: Vec::new(),
        }]
        .try_into()
        .map_err(Error::engine)?,
        current_scene_id: project::SceneId(0),
        settings: source.data().settings.clone(),
        caption_presets: source.data().caption_presets.clone(),
        text_presets: source.data().text_presets.clone(),
    })
    .map_err(Error::engine)?;
    let mut settings = render::ExportSettings::from_project(&project);
    settings.render = render::RenderSettings::portrait(&project);
    settings.render.caption_languages = options.languages.to_vec();
    if let Some(crop) = short.crop {
        let placement = render::Placement {
            crop,
            ..Default::default()
        };
        for id in manual_crop {
            settings.render.placements.insert(id, placement);
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
    render::Renderer::new(&project, settings.render.clone()).map_err(Error::engine)?;
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
        layouts,
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

/// Output frame of a short, as `render::RenderSettings::portrait` sets it.
const PORTRAIT: [u32; 2] = [1080, 1920];
/// Track id of the second layout slot; the first slot is the main track (2) and
/// the captions are track 3.
const SLOT_TRACK_FIRST: u64 = 4;
/// The engine's cap on the shots of one analysis.
const MAX_SHOTS: usize = 4096;

/// Pixel size of the source's first video stream.
fn source_frame(asset: &media::SourceAsset) -> Option<[f64; 2]> {
    let shape = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())?;
    Some([f64::from(shape.width), f64::from(shape.height)])
}

/// One layout slot as a video item: crop and fit in its params, scale and
/// position as static transform channels. Only slot 0 plays the source audio.
fn slot_item(
    id: ItemId,
    asset: media::AssetId,
    range: TimeRange,
    start: Tick,
    slot: &crate::layout::Slot,
) -> Result<Item, project::Error> {
    let mut animations = project::Animations::default();
    for (field, value) in [
        (project::TransformField::ScaleX, slot.scale[0]),
        (project::TransformField::ScaleY, slot.scale[1]),
        (project::TransformField::PositionX, slot.offset[0]),
        (project::TransformField::PositionY, slot.offset[1]),
    ] {
        if value != field.default_value() {
            animations.set(
                project::AnimationPath::from(field.builtin()),
                project::Param::new(project::ParamValue::Scalar(field.clamp(value)?))?,
            )?;
        }
    }
    Ok(Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations,
        id,
        source_asset: Some(asset),
        source_range: range,
        start,
        content: ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: slot.audio,
            volume: 0.0,
            muted: false,
            crop: slot.crop,
            fit: slot.fit,
            cleanup: Default::default(),
        }),
    })
}

fn cue_bytes(cues: &CueTrack) -> Result<u64, EngineError> {
    let mut total = 0u64;
    for cue in &cues.cues {
        let mut bytes = u64::try_from(cue.id.as_str().len() + cue.source_lang.as_str().len())
            .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?
            .checked_add(1024)
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
        for (language, text) in &cue.texts {
            let item = u64::try_from(
                text.text.as_str().len()
                    + language.as_str().len()
                    + text
                        .translation_ref
                        .as_ref()
                        .map_or(0, |reference| reference.as_str().len()),
            )
            .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?
            .checked_add(256)
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
            bytes = bytes
                .checked_add(item)
                .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
        }
        total = total
            .checked_add(bytes)
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
    }
    Ok(total)
}
