# Analysis

Interface: `scene_detection`, `speech_activity`, `waveform`, `loudness`, `chapters`, `fillers`, `emphasis`,
`AnalysisRequest`, `AnalysisJob`.

Every workflow here measures source media, never mutates a project, and returns a typed
`AnalysisError` (re-exported at the engine door). Its `Edit`, `Runtime`, `Providers`,
`Media`, `Render` and `Ai` causes retain their typed source and existing failure policy;
higher mixed workflows compose `engine::control::Error::Analysis`. Kernel-only validation and
reviewed edits return `EditError`, while cache/key operations and cache-only display
failures retain `RuntimeError`. Failed-job finishers retain the original shared cause in
opaque `RuntimeError::Failed` without inheriting cancellation, retry or advisory policy.
A request binds its source content and settings into a cache key; `submit` queues
one local or network job; `finish` validates and caches complete output, so cancellation before
runner promotion leaves no cache entry and cache pressure preserves the runner output for a retry.
Artifact corruption uses [RuntimeError::CacheCorrupt](../runtime/README.md).
Envelope corruption permits regeneration; scene, speech and shot workflows refuse
a valid envelope whose payload fails request validation. Waveforms also regenerate
invalid payloads, while cache-only display reports Failed without starting work.
Job progress, cancellation and byte admission belong to the [job runner](../runtime/README.md#background-jobs-and-byte-policy).

## Reframe

`ReframeRequest` turns a source range into a cached subject path in source ticks and
`emit_reframe` into the crop keys of one item; see [reframe](reframe/README.md).

## Loudness normalization

`audio.set_loudness {scope: clip|master, target_lufs, true_peak_db}` stores an EBU R128 target
(-50..-5 LUFS, -20..0 dBTP; default -14/-1) and drops stale gains; `audio.clear_loudness` removes
it. Measured gains arrive through `audio.apply_loudness {gains}`, which validates range and
ordering only ([`LoudnessTarget`](../../../project/src/model/settings.rs)).

`audio.measure_loudness {}` (`loudness::submit_loudness(runner, project, sources)`) queues one CPU
`MeasureLoudness` job that runs render's two-pass planner over the production mix graph for the
configured targets and refuses `Invalid` when none is set. It reports `Mix` progress, honours
cancellation and retains the raw typed failure in the job snapshot. Workflow finishers
that propagate a failed job compose `RuntimeError::Failed` upward. It takes no cache and never edits the
project: `job_status` returns `output.kind = "loudness"` (the gains plus the measured numbers per
clip and master, via `loudness::decode`) and the caller sends `audio.apply_loudness` with those
gains. The gains' key is `loudness::content_key`: a hash of the active
scene's audible items (timing, source content, volume/mute and their automation), track mutes,
sample rate and both targets; the master fader is excluded because measurement ignores it.

`submit_export*` calls `admit_loudness` (owned by `runtime/`, exported as `loudness::admit`),
which returns `runtime::Result<()>` and refuses `RuntimeError::LoudnessStale` when a target is
set and the stored gains are missing or measured for different audio, so preview and export
cannot disagree. Undo restores the gains with the
content. Limits: `LoudnessGains::MAX_CLIPS` 4096 clips and a 1 MiB result.

## Scene detection

`scene_detection::DetectionRequest::new(asset, source_range, DetectionSettings)` produces
source-time `SceneCandidate`s with exact decoded PTS, `HardCut`/`Fade` and a normalized
measured-change confidence (not a calibrated probability). `media.detect_scenes {asset, settings}`
runs one local CPU `DetectScenes` job over an admitted project asset's whole video through the lent
`MediaAccess`; omitted `settings` fields take `DetectionSettings::default()`. `job_status` returns
`output.kind = "scenes"` (`scene_detection::decode`), and `DetectionJob::finish`, run where control
first sees the job completed, validates it against its request and caches complete output as
`SceneDetection` without consuming the retained output. Keys bind source content, source range, all thresholds and the versioned
recipe; a warm hit opens no source. Shot analysis invokes this same detector in its RGB-only
hard-cut mode.

Candidates are conservative and require review; no general accuracy is claimed.
`apply_reviewed(session, item, result, selected_indexes)` validates explicit unique indexes,
matching source identity and strict source-time item bounds, converts source to output ticks
through project's retime mapping, and applies descending `SplitAt` commands grouped into one
transaction and one undo step within the command limit. An empty selection does nothing and any
refusal publishes nothing.

Limits: one day, 16 Mi-pixel frames, 4095 candidates, 1 MiB serialized result. Decode streams
through media's `decode_shot_frames` in ≤300-second, ≤100,000-frame slices including preroll.

## Speech activity

`speech_activity::SpeechActivityRequest::new(asset, range, SpeechActivitySettings)` produces
contiguous source-time `SpeechActivitySpan`s classified as speech or pause, each with a
heuristic confidence (not a calibrated probability). `submit` runs one local CPU
`AnalyzeSpeechActivity` job; `finish` validates and caches complete output as `SpeechActivity`.
Keys bind source content, source range, the thresholds and the versioned recipe. Adjacent
same-activity spans merge and spans always cover the requested range contiguously.

Limits: one day, 288,000 spans, 32 MiB serialized result and a 96 MiB workspace. A source
without an audio stream is `Unsupported`; a deleted or unreadable source fails the job with the
media error; overflow is `Limit`.

## Waveform peaks

`waveform::WaveformRequest::new(asset)` admits a mono or stereo audio source whose stream rate is
8-192 kHz. Its key binds the source content hash, sample rate, channel count and the versioned
recipe. `submit` runs one local CPU `GenerateWaveform` job; a warm cache hit opens no source.
`finish` validates and caches complete output as `ArtifactKind::Waveform`; an undecodable cached
payload is regenerated.

`waveform::display(asset, item, visible, columns, cache)` is the cache-only display query. It
clamps the visible output range to the clip, maps each of `columns` output slices through the
project item's trim and retime mapping, reduces the covered 128-sample buckets to one min/max bar
per slice, and returns `Ready(bars)`, `Pending` when no summary is cached, `Failed` when the
cached summary cannot be decoded, or `Unsupported` when the source has no mono or stereo audio.
It never decodes media and never blocks on a job. Bars combine all channels; `over` marks a bar
whose any sample leaves -1..=1.

Limits: columns 1..=4096 and one 64 MiB summary. A mismatched asset or an out-of-clip visible
range is `Invalid`. `media.generate_waveform {asset}` runs the same request as one control job
and publishes the completed summary through the attached media caches; a caller without those
caches refuses `Unsupported`.

## Pause candidates

`pause_candidates::PauseCandidatesRequest::new(project, PauseSettings { min_pause_ms, pad_ms,
crossfade_ms })` plans one CPU `AnalyzePauses` job over the active scene's audible items. Each
contiguous audible source interval is decoded once, every pause is inset by `pad` on its speech
sides (a pause clipped by an item edge keeps no pad there), mapped to output time, merged across
tracks, and every range covered by a native caption word span is subtracted, so a candidate
never cuts inside a word. The result is `PauseCandidates { spans, analyzed_sources }` in output
time, and `timeline.remove_pauses` consumes exactly those spans.

The job holds no artifact cache: an engine Session owns none, so a repeated query decodes again.
Defaults:

| Setting | Default | Source |
|---|---|---|
| `min_pause_ms` | 500 | FFmpeg `whisper` filter `vad_min_silence_duration`, the run length the speech detector uses |
| `pad_ms` | 20 | one detector frame (320 samples at 16 kHz), the boundary resolution of the measured pause |
| `crossfade_ms` | 20 | equal to the pad so the equal-power fade pair consumes only retained silence |

`pad_ms` is kept silence on each side of a cut and `crossfade_ms` must not exceed it. Settings
admit 0..=10 s; the result is bounded at 4096 spans and 1 MiB. An inaudible timeline completes
with zero spans rather than refusing. Limits: one day and 96 MiB workspace per source window,
40 MiB decode.

## Chapter generation

`chapters::ChapterRequest::new(project, language, max_chapters)` reads every caption cue of the
active scene that carries text in `language`, maps its range to output time and builds the
validated `ai::text::chapters::ChapterInput` (segment id `<item>:<cue>`, cue text, output range) with
the requested language, the 1..=50 chapter bound and the scene duration. Cues missing that
language are skipped, so a project with only another language refuses `EmptyTranscript`. An
unsupported language, an empty transcript, a zero or oversized bound, overlapping or blank cues
and an oversized payload refuse through `ai::text::chapters::ChapterRefusal` before any job or
provider work.

`submit` asks the lent [`TextAccess`](../providers/README.md#transcript-text-generation) for the
exact consent token and only then queues one network `GenerateChapters` job. The job bounds its
result at 1 MiB and decodes it through the versioned `ChapterDocument::from_json` validator.
`chapters.generate {language, max_chapters}` accepts exactly this request; `job_status` publishes
the completed document in `JobView.output` (`chapters`). A caller without a lent access refuses
`ConsentRequired`. The job has no artifact cache and never mutates the project.

## Filler classification

`fillers::FillerRequest::new(project, language, include_markers)` reads every visible measured
word of the active scene whose cue language has the requested primary subtag, builds each stable
candidate ID (`filler:<item>:<cue>:<source word start>`) and sends the exact bounded
`ai::text::fillers::FillerInput`. A transcript with no timed word in that language refuses
`EmptyTranscript`, an unsupported language refuses `UnsupportedLanguage`, and lossy or duplicate
word IDs refuse `InvalidSegments` before any job or provider work. The local
`timeline.filler_candidates` command stays synchronous and lexicon-only; this job is the opt-in
network path and never falls back to it.

`submit` asks the lent `TextAccess::prepare_fillers` for the exact consent token and queues one
network `ClassifyFillers` job. Each returned ID is mapped back to this job's visible word table,
sorted by output start, bounded at 1 MiB, and an ID that does not exist in the sent transcript
fails the job with the typed `InvalidSegments` reason. `job_status` publishes `JobView.output` (`fillers`);
`timeline.remove_fillers` consumes the same IDs. A caller without a lent access refuses
`ConsentRequired`. No artifact cache is used and the project is never mutated.

## Caption emphasis

`emphasis::EmphasisRequest::new(project, item, cues)` reads one caption item's cues that carry
verified word timing (`captions::validate_selection`), builds the `<item>:<cue>` segments with one
`{position, text}` per verified word and sends the exact bounded `ai::text::emphasis::EmphasisInput`.
`cues` of `None` covers every eligible cue; a named cue that is missing or lacks verified timing
refuses with `captions::Error::NoWordTiming` (or `Invalid`), and an item with no eligible cue
refuses `EmptyTranscript`, all before any job or provider work. The local lexicon is
`captions.select_emphasis`; this job is the opt-in network path and never falls back to it.

`submit` asks the lent `TextAccess::prepare_emphasis` for the exact consent token and queues one
network `SelectEmphasis` job. A caller without a lent emphasis adapter, or without consent, refuses
`ConsentRequired`. Returned positions that are not words of the cue they came with fail the job with
`InvalidSegments`; a provider failure fails it with its typed reason, with no result. The result is
a list of `EmphasisSuggestion {item, cue, word_ids}` (`job_status` output `emphasis`), bounded at
1 MiB, that `captions.set_emphasis` applies. The project is never mutated and no artifact cache is
used.
