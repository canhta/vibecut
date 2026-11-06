# clip-eval

Fixed labelled evaluation of the clip pipeline: transcript segments in, ranked
shorts out, metrics and a local JSON/Markdown report. This is our own AI logic
(selection plus boundary snapping), so it is measured; third-party model accuracy
is not a gate here.

## Modules

| Module | Responsibility |
|---|---|
| `harness` | Runs the fixed label set and writes its scored report. |
| `harness/suggestion` | Calls one provider and snaps the clips it proposes to source boundaries. |
| `harness/report` | Owns the persisted evaluation report and its Markdown rendering. |
| `asr` | Prepares an opt-in transcript through the speech provider. |
| `baseline` | Holds the pinned main-revision baseline inputs and metrics. |
| `budget` | Holds one cumulative run budget shared by ASR and suggestion invocations. |
| `label` | Loads one JSON label document per source video. |
| `metrics` | Defines the labelled clip metrics. |
| `transcript` | Caches the pipeline's fixed transcript input by source content. |

## Purpose and exclusions

Owns the labelled-set format, transcript cache lookup, provider/baseline runs,
metric definitions and report rendering. It does not own ASR, translation,
transcript segmentation, suggestion generation, boundary snapping, export or
provider adapters; those stay with their existing crates.

Excluded: network calls unless a live LLM run is explicitly requested, model
accuracy claims, product quality gates, and any committed private media,
transcripts, labels or reports. Reports are local artifacts under
`target/clip-eval/`.

## Entry points

```sh
cargo xtask clip-eval synthetic
cargo xtask clip-eval <set> --labels DIR --provider heuristic --k 5
cargo xtask clip-eval <set> --transcribe --max-cost-usd 1
cargo xtask clip-eval <set> --provider gpt-6-luna --live --max-cost-usd 1
cargo xtask clip-eval <set> --write-baseline PATH --baseline-revision MAIN_SHA
cargo xtask clip-eval <set> --baseline PATH
```

`xtask` launches the `clip-eval` binary with the workspace FFmpeg environment.
`<set>` resolves to `crates/clip-eval/fixtures/synthetic` for `synthetic`, an
existing directory path, or `.privates/clip-eval/<set>` (falling back to the main
checkout's `.privates` from a linked worktree). Label files are `*.json` in that
directory; `*.transcript.json` and `baseline.json` are inputs, not labels.

Live LLM calls require `--live`, `OPENAI_API_KEY`, and stay within
`--max-cost-usd` (default USD 1). One cumulative budget covers ASR and suggestion
reservations across every video; each remote invocation is reserved before its
first upload and the reservation stays spent if it later fails. The report prints
the per-video estimate and remaining budget before the call. Pure LLM calls use
only `gpt-6-luna`. The default provider is the free local heuristic, so the
synthetic set runs offline.

`--transcribe` is the opt-in live ASR step for a label without a transcript. It
uploads bounded 15-minute mono PCM chunks to the admitted ElevenLabs Scribe v2
adapter, assembles absolute source coordinates, and caches the resulting
transcript. It requires `ELEVENLABS_API_KEY` and the same cumulative budget. ASR
is not an LLM call, so the Luna-only rule does not apply.

A main-revision baseline is an explicit pinned file, never candidate metrics
relabelled. `--write-baseline` records the heuristic metrics plus the complete
metric identity: source, transcript, resolved pipeline settings, ordered picks
and priorities, `k`, language and energy mode. Run it on the pinned main
revision. `--baseline` loads and validates that file, rejecting a mismatched
label set or any identity mismatch as a typed refusal. A `baseline.json` beside
the labels is loaded automatically; the synthetic fixtures ship one. Use
`--no-baseline` to ignore it. When no baseline is present the report says the
main baseline was not measured. The persisted baseline schema is 2.

## Label format

One file per video. Paths are relative to the label file.

```json
{
  "version": 1,
  "source": "interview.mp4",
  "sha256": "<lowercase sha256 of the source bytes>",
  "language": "vi",
  "transcript": "interview.transcript.json",
  "settings": {
    "count": 5,
    "min_duration_seconds": 20.0,
    "max_duration_seconds": 60.0,
    "min_segment_duration_seconds": 2.0,
    "max_segments": 4,
    "overlap_tolerance": 0.5,
    "frame_rate": 30.0,
    "snap_tolerance_seconds": 1.0,
    "silence_rms": 0.01,
    "keywords": []
  },
  "clips": [
    {
      "id": "human-pick-1",
      "priority": "must",
      "segments": [
        {"start": 2540160000000, "end": 5080320000000},
        {"start": 7620480000000, "end": 10160640000000}
      ]
    }
  ]
}
```

`segments` are ordered source-segment lists in play order; gaps and reordering
are allowed and a single range is the one-segment case. `priority` is `must` or
`nice`. Synthetic fixtures replace `source` with
`"synthetic": {"duration_seconds": 41.0}`; the harness writes a deterministic
silence WAV under `target/clip-eval/synthetic/` and still verifies `sha256`.

`transcript` is a serialized `captions::Transcript` with source word or span
timing. The harness caches it under
`target/clip-eval/cache/transcripts/<source-sha>-<transcript-sha>.json`; a
prepared cache named `<source-sha>.json` is used when a label omits `transcript`.
Without either, `--transcribe` runs Scribe and caches
`<source-sha>-elevenlabs-scribe_v2.json`. Missing/mismatched hashes, a missing
transcript without `--transcribe`, and a language mismatch are typed errors before
pipeline execution.

## Metrics

Each metric is reported per video and aggregated for the selected provider, the
`source: heuristic` baseline at the evaluated revision, and the pinned main
baseline when one is loaded:

- **Must hit rate**: share of `must` picks matched by any suggestion with summed
  source-segment IoU >= 0.5. Intersections and source durations are summed
  pairwise, gaps are excluded, and disjoint pieces are never matched by their
  enclosing envelope.
- **Precision@k**: share of the top `k` suggestions matching any labelled pick.
  The denominator is the number of available suggestions when fewer than `k`.
- **Boundary quality**: share of all suggestion segment edges that do not fall
  strictly inside a captions-owned speech unit or a sentence span.
- **Duration compliance**: share of suggestions within the configured total,
  piece and segment-count bounds.
- **Cost and wall time**: provider upper-bound estimate in USD micros and elapsed
  time per video. Billing is not measured.

Snapping is applied to every suggestion before metrics. A per-suggestion snapping
refusal is counted, keeps the unsnapped ranges for measurement, and is noted in
the report; the whole eval does not abort.

## Tests and limits

`cargo test -p clip-eval` covers exact, partial, none, reordered, disjoint with
overlapping envelopes, overlapping suggestions, boundary word/sentence cuts,
stable synthetic WAV generation, typed missing/mismatched label hashes, untimed
ASR punctuation attachment, the extracted-chunk sample grid, cumulative
ASR+LLM budget reservations, a second video refused by one run budget, pinned
baseline comparison where candidate and main metrics differ, and baseline identity
refusal for changed picks, changed `k`, changed energy mode and a changed source
hash. `cargo xtask ci` runs the workspace suite. The synthetic set has
two fixtures plus a pinned baseline and is the acceptance path before maintainer
labels exist.

Label sets are capped by the AI clip contract (10,000 transcript segments, 4 MiB
text, 1,024 windows, 64 KiB wire). The harness reads at most one label directory
level and never writes outside `target/clip-eval/`.
