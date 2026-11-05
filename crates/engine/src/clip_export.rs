//! Batch export of accepted shorts, each an ordered list of source pieces.
//!
//! Engine owns preparation, scheduling and output naming. Render owns pixels and
//! publication; captions owns cue mapping; the job runner owns concurrency,
//! progress, cancellation and retry boundaries.
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSnapshot, JobSpec, JobStage, JobState,
    ResourceClass,
};
use crate::{EditError, subtitle_file};
use captions::{CueTrack, Format, LanguageTag, NfcText};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectData, Timeline, Track, TrackId,
    TrackKind, VideoParams,
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use time::{Tick, TimeRange};
use unicode_segmentation::UnicodeSegmentation;

/// Matches the AI clip contract's `max_segments` ceiling.
pub const MAX_CLIP_SEGMENTS: usize = 32;
/// Bounds eager preparation of derived projects and the submitted output set.
pub const MAX_BATCH_CLIPS: usize = 1000;
/// Bounds cue content cloned into all derived projects in one batch.
pub const MAX_BATCH_CUE_BYTES: u64 = 256 << 20;
/// Longest UTF-8 base name before suffixes and extensions.
const MAX_BASE_BYTES: usize = 200;

/// One accepted short: a title and its source pieces in play order.
#[derive(Clone, Debug)]
pub struct ClipShort {
    pub title: NfcText,
    /// Canonical AI clip pieces in play order. Export reads each piece's range.
    pub segments: Vec<ai::clips::ClipSourceSegment>,
    /// Optional normalized source crop applied to every piece.
    pub crop: Option<render::Rect>,
}
impl ClipShort {
    pub fn new(title: impl Into<NfcText>, segments: Vec<ai::clips::ClipSourceSegment>) -> Self {
        Self {
            title: title.into(),
            segments,
            crop: None,
        }
    }
    /// Preserve the accepted suggestion's play order and ranges; crop stays centered.
    pub fn from_suggestion(suggestion: &ai::clips::ClipSuggestion) -> Self {
        Self {
            title: suggestion.proposal.title.clone(),
            segments: suggestion.segments.clone(),
            crop: None,
        }
    }
    /// Apply one normalized crop rectangle to every piece in this short.
    pub fn with_crop(mut self, crop: Option<render::Rect>) -> Self {
        self.crop = crop;
        self
    }
}

/// Inputs shared by every short in one batch.
#[derive(Clone, Debug)]
pub struct ClipBatchRequest {
    /// Source media holding every short's pieces.
    pub asset: media::AssetId,
    /// Source-timed cues; mapped through each short's ordered pieces.
    pub cues: CueTrack,
    pub shorts: Vec<ClipShort>,
    /// Existing output directory.
    pub directory: PathBuf,
    /// None for no captions, or one/two distinct languages to burn and write.
    pub languages: Vec<LanguageTag>,
    /// Sidecar formats; empty writes no subtitle files.
    pub subtitle_formats: Vec<Format>,
    pub overwrite: bool,
    /// Explicit consent for initially missing/changed visual sources.
    pub allow_degraded: bool,
    /// Per-artifact output and staging ceiling; 1 byte..=1 TiB.
    pub max_output_bytes: u64,
}
impl ClipBatchRequest {
    pub fn new(
        asset: media::AssetId,
        shorts: Vec<ClipShort>,
        directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            asset,
            cues: CueTrack::default(),
            shorts,
            directory: directory.into(),
            languages: Vec::new(),
            subtitle_formats: Vec::new(),
            overwrite: false,
            allow_degraded: false,
            max_output_bytes: 64 << 30,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipSubtitlePath {
    pub format: Format,
    pub path: PathBuf,
}

/// Artifacts written for one short. Every path is a complete file once published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipPaths {
    pub video: PathBuf,
    pub subtitles: Vec<ClipSubtitlePath>,
}

/// Artifacts this prepared short already published, retained across retries.
#[derive(Default)]
struct Published {
    video: bool,
    subtitles: Vec<bool>,
}
struct PreparedClip {
    project: Arc<Project>,
    settings: render::ExportSettings,
    paths: ClipPaths,
    caption_track: Option<TrackId>,
    languages: Vec<LanguageTag>,
    input: CacheKey,
    duration: Tick,
    sidecar_bytes: u64,
    overwrite: bool,
    max_output_bytes: u64,
    published: Arc<Mutex<Published>>,
    /// Immutable original snapshot; only its asset identities are read, to keep
    /// output admission protecting every original project source.
    sources: Arc<Project>,
}
struct ClipJob {
    id: JobId,
    cancel: CancelToken,
}
/// Prepared shorts and their submitted jobs. Preparation performs no IO and
/// submits nothing; `submit` adds one export job per short to the caller's runner.
pub struct ClipBatch {
    clips: Vec<PreparedClip>,
    jobs: Vec<Option<ClipJob>>,
}
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
    pub fn len(&self) -> usize {
        self.clips.len()
    }
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
    /// Derived snapshot for preview or inspection; the batch owns it.
    pub fn project(&self, index: usize) -> Option<&Project> {
        self.clips.get(index).map(|clip| clip.project.as_ref())
    }
    /// Export settings for the prepared short, including portrait geometry,
    /// caption languages and per-piece crop placements.
    pub fn settings(&self, index: usize) -> Option<&render::ExportSettings> {
        self.clips.get(index).map(|clip| &clip.settings)
    }
    pub fn paths(&self, index: usize) -> Option<&ClipPaths> {
        self.clips.get(index).map(|clip| &clip.paths)
    }
    pub fn duration(&self, index: usize) -> Option<Tick> {
        self.clips.get(index).map(|clip| clip.duration)
    }
    pub fn job_id(&self, index: usize) -> Option<JobId> {
        self.jobs
            .get(index)
            .and_then(Option::as_ref)
            .map(|job| job.id)
    }
    /// Latest runner snapshot, or None when never submitted or evicted.
    pub fn snapshot(&self, runner: &JobRunner, index: usize) -> Option<JobSnapshot> {
        runner.snapshot(self.job_id(index)?)
    }
    /// Submit every short that has no job yet. A repeated call submits nothing and
    /// returns an empty list. If submission fails midway, earlier jobs stay tracked
    /// and cancellable; nothing is published until a job runs.
    pub fn submit(&mut self, runner: &mut JobRunner) -> Result<Vec<JobId>, EditError> {
        let mut submitted = Vec::new();
        submitted
            .try_reserve_exact(self.clips.len())
            .map_err(|_| EditError::Limit)?;
        for index in 0..self.clips.len() {
            if self.jobs.get(index).is_some_and(Option::is_some) {
                continue;
            }
            let clip = self
                .clips
                .get(index)
                .ok_or(EditError::Invalid("clip index"))?;
            let (id, cancel) = submit_clip(runner, clip)?;
            if let Some(slot) = self.jobs.get_mut(index) {
                *slot = Some(ClipJob { id, cancel });
            }
            submitted.push(id);
        }
        Ok(submitted)
    }
    /// Resubmit a failed or cancelled short with its prepared inputs. Completed,
    /// running and unknown jobs are refused.
    pub fn retry(&mut self, runner: &mut JobRunner, index: usize) -> Result<JobId, EditError> {
        let clip = self
            .clips
            .get(index)
            .ok_or(EditError::Invalid("clip index"))?;
        let current = self
            .jobs
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(EditError::Invalid("clip not submitted"))?;
        match runner.snapshot(current.id).map(|snapshot| snapshot.state) {
            Some(JobState::Failed | JobState::Cancelled) => {}
            _ => return Err(EditError::Invalid("clip is not retryable")),
        }
        let (id, cancel) = submit_clip(runner, clip)?;
        if let Some(slot) = self.jobs.get_mut(index) {
            *slot = Some(ClipJob { id, cancel });
        }
        Ok(id)
    }
    /// Cancel one short. Cancelling an already finished job has no effect.
    pub fn cancel(&self, index: usize) -> Result<(), EditError> {
        let job = self
            .jobs
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(EditError::Invalid("clip not submitted"))?;
        job.cancel.cancel();
        Ok(())
    }
    pub fn cancel_all(&self) {
        for job in self.jobs.iter().flatten() {
            job.cancel.cancel();
        }
    }
}

struct Options<'a> {
    asset: &'a media::SourceAsset,
    cues: &'a CueTrack,
    languages: &'a [LanguageTag],
    formats: &'a [Format],
    directory: &'a Path,
    overwrite: bool,
    allow_degraded: bool,
    max_output_bytes: u64,
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
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(next_item),
            source_asset: Some(options.asset.id()),
            source_range: *range,
            start: *start,
            content: ItemParams::Video(VideoParams {
                source_audio_enabled: true,
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
        assets: vec![options.asset.clone()],
        timeline,
        bookmarks: Vec::new(),
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
            .timeline
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.items)
        {
            settings.render.placements.insert(item.id, placement);
        }
    }
    settings.mode = render::ExportMode::Encode;
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

fn job_key(
    project: &Project,
    duration: Tick,
    settings: &render::ExportSettings,
    crop: Option<render::Rect>,
    options: &Options<'_>,
) -> Result<CacheKey, EditError> {
    let encoded = project::encode(project, false)?;
    let source = media::ContentHash::from_bytes(&encoded).0;
    let range = media::ContentHash::from_bytes(&duration.0.to_le_bytes()).0;
    let languages = options
        .languages
        .iter()
        .map(captions::LanguageTag::as_str)
        .collect::<Vec<_>>()
        .join(",");
    let formats = options
        .formats
        .iter()
        .map(|format| extension(*format))
        .collect::<Vec<_>>()
        .join(",");
    let crop = crop.map_or_else(String::new, |crop| {
        format!("{}:{}:{}:{}", crop.x, crop.y, crop.width, crop.height)
    });
    let settings_key = format!(
        "{}x{}@{}:{}:{}:{}:{}:{}:{}:{}",
        settings.render.width,
        settings.render.height,
        settings.fps.num(),
        settings.fps.den(),
        settings.bit_rate,
        settings.quality,
        settings.include_audio,
        languages,
        formats,
        crop,
    );
    // Export files are user artifacts, not cache entries; this key identifies the job.
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source,
        provider: "vibecut-engine",
        model: "clip-batch-export",
        version: "1",
        settings: settings_key.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: range,
        translation: [0; 32],
    })
}

fn submit_clip(
    runner: &mut JobRunner,
    clip: &PreparedClip,
) -> Result<(JobId, CancelToken), EditError> {
    let project = clip.project.clone();
    let settings = clip.settings.clone();
    let paths = clip.paths.clone();
    let caption_track = clip.caption_track;
    let languages = clip.languages.clone();
    let overwrite = clip.overwrite;
    let max_output_bytes = clip.max_output_bytes;
    let published = clip.published.clone();
    let sources = clip.sources.clone();
    let range = TimeRange::new(Tick::ZERO, clip.duration)?;
    runner.submit(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::GpuEncoder,
            input: clip.input,
            bytes: JobBytes {
                export: clip.sidecar_bytes,
                ..Default::default()
            },
        },
        move |context| {
            context.progress(JobStage::Admit, 0.0)?;
            let policy = if overwrite {
                project::atomic::Overwrite::Replace
            } else {
                project::atomic::Overwrite::Refuse
            };
            // Resume only missing artifacts; an artifact this short already
            // published is never overwritten again.
            let (video_done, subtitles_done) = reconcile_published(&published, &paths)?;
            // Refuse aliases and occupied destinations before staging or encoding.
            // Validate against the original snapshot, not the reduced derived
            // timeline, so every project source stays protected.
            for (index, subtitle) in paths.subtitles.iter().enumerate() {
                if subtitles_done.get(index).copied().unwrap_or(false) {
                    continue;
                }
                render::validate_output_path(&sources, &subtitle.path, context)?;
            }
            if !video_done {
                render::validate_output_path(&sources, &paths.video, context)?;
            }
            // Stage unpublished sidecars first: a later MP4 refusal leaves no
            // subtitle artifact.
            let mut staged = Vec::new();
            staged
                .try_reserve_exact(paths.subtitles.len())
                .map_err(|_| EditError::Limit)?;
            for (index, subtitle) in paths.subtitles.iter().enumerate() {
                if subtitles_done.get(index).copied().unwrap_or(false) {
                    continue;
                }
                context.check_cancelled()?;
                let track = caption_track.ok_or(EditError::Invalid("clip subtitle track"))?;
                let export = subtitle_file(&project, track, subtitle.format, &languages)?;
                let stage =
                    project::atomic::StreamingStage::new(&subtitle.path, policy, max_output_bytes)?;
                {
                    let mut writer = stage.writer()?;
                    for chunk in export.text.as_bytes().chunks(65536) {
                        context.check_cancelled()?;
                        writer.write_all(chunk)?;
                    }
                }
                staged.push((index, stage.finalize()?));
            }
            // The MP4 is the primary artifact and publishes through render's fence.
            if !video_done {
                render::export(&project, range, &settings, &paths.video, context)?;
                mark_video(&published)?;
            }
            for (index, stage) in staged {
                // A retry may skip render and its publication fence entirely.
                context.commit_publication(|| {
                    stage.publish()?;
                    mark_subtitle(&published, index)
                })?;
            }
            Ok(Vec::new())
        },
    )
}

fn extension(format: Format) -> &'static str {
    match format {
        Format::Srt => "srt",
        Format::WebVtt => "vtt",
    }
}

#[derive(Default)]
struct NameSet {
    used: BTreeSet<String>,
}
impl NameSet {
    fn unique(&mut self, base: &str) -> String {
        for attempt in 1..=10_000u32 {
            let suffix = if attempt == 1 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let allowance = MAX_BASE_BYTES.saturating_sub(suffix.len());
            let mut candidate = fit(base, allowance);
            if candidate.is_empty() {
                candidate.push_str("clip");
            }
            candidate.push_str(&suffix);
            if self.used.insert(candidate.to_lowercase()) {
                return candidate;
            }
        }
        "clip".to_owned()
    }
}

fn fit(value: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for grapheme in value.graphemes(true) {
        if out.len() + grapheme.len() > max_bytes {
            break;
        }
        out.push_str(grapheme);
    }
    out
}

fn name_base(title: &str, index: usize) -> String {
    let mut output = String::new();
    for grapheme in title.graphemes(true) {
        let mut piece = String::new();
        for character in grapheme.chars() {
            if unsafe_filename_character(character) {
                piece.push('-');
            } else {
                piece.push(character);
            }
        }
        if output.len() + piece.len() > MAX_BASE_BYTES {
            break;
        }
        output.push_str(&piece);
    }
    let base = output
        .trim_matches(|character: char| character.is_whitespace() || character == '.')
        .trim()
        .to_owned();
    if base.is_empty() {
        return format!("clip-{:02}", index + 1);
    }
    if reserved_name(&base) {
        return format!("clip-{base}");
    }
    base
}

fn unsafe_filename_character(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
    ) || character.is_control()
        || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn reserved_name(base: &str) -> bool {
    let stem = base.split('.').next().unwrap_or(base).to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && bytes
            .get(3)
            .is_some_and(|digit| matches!(*digit, b'1'..=b'9'))
}

fn artifact_present(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Drop publication marks whose artifact disappeared; keep every artifact that
/// still exists so a retry does not overwrite its own completed output. Reads the
/// marks before touching the filesystem and holds no lock across that IO.
fn reconcile_published(
    published: &Mutex<Published>,
    paths: &ClipPaths,
) -> Result<(bool, Vec<bool>), EditError> {
    let (video, mut subtitles) = {
        let state = published.lock().map_err(|_| EditError::JobPoisoned)?;
        (state.video, state.subtitles.clone())
    };
    let video = video && artifact_present(&paths.video);
    for (index, subtitle) in paths.subtitles.iter().enumerate() {
        if subtitles.get(index).copied().unwrap_or(false)
            && !artifact_present(&subtitle.path)
            && let Some(done) = subtitles.get_mut(index)
        {
            *done = false;
        }
    }
    {
        let mut state = published.lock().map_err(|_| EditError::JobPoisoned)?;
        state.video = video;
        state.subtitles.clone_from(&subtitles);
    }
    Ok((video, subtitles))
}

fn mark_video(published: &Mutex<Published>) -> Result<(), EditError> {
    published.lock().map_err(|_| EditError::JobPoisoned)?.video = true;
    Ok(())
}

fn mark_subtitle(published: &Mutex<Published>, index: usize) -> Result<(), EditError> {
    let mut state = published.lock().map_err(|_| EditError::JobPoisoned)?;
    let done = state
        .subtitles
        .get_mut(index)
        .ok_or(EditError::Invalid("clip artifact index"))?;
    *done = true;
    Ok(())
}
