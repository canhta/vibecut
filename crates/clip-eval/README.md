# clip-eval

Fixed labelled evaluation of the clip pipeline: transcript segments in, ranked
shorts out, metrics and a local JSON/Markdown report. This is our own AI logic
(selection plus boundary snapping), so it is measured; third-party model accuracy
is not a gate here.
## Purpose and exclusions

Owns the labelled-set format, transcript cache lookup, provider/baseline runs,
metric definitions and report rendering. It does not own ASR, translation,
transcript segmentation, suggestion generation, boundary snapping, export or
provider adapters; those stay with their existing crates.

Excluded: network calls unless a live LLM run is explicitly requested, model
accuracy claims, product quality gates, and any committed private media,
transcripts, labels or reports. Reports are local artifacts under
`target/clip-eval/`.

| Module | Responsibility | Contract |
|---|---|---|
| `harness/` | Runs the fixed label set into a scored report. | [Entry points](src/harness/README.md) |
| `harness/suggestion` | Calls one provider and snaps the clips it proposes to source boundaries. | [source](src/harness/suggestion.rs) |
| `harness/report` | Owns the persisted evaluation report and its Markdown rendering. | [source](src/harness/report.rs) |
| `asr` | Prepares an opt-in transcript through the speech provider. | [source](src/asr.rs) |
| `baseline` | Holds the pinned main-revision baseline inputs and metrics. | [Baseline](src/baseline/README.md) |
| `budget` | Holds one cumulative run budget shared by ASR and suggestion invocations. | [source](src/budget.rs) |
| `label` | Loads one JSON label document per source video. | [Label format](src/label/README.md) |
| `metrics` | Defines the labelled clip metrics. | [Metrics](src/metrics/README.md) |
| `transcript` | Caches the pipeline's fixed transcript input by source content. | [Transcript cache](src/transcript/README.md) |

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
