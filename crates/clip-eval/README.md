# clip-eval

Fixed labelled evaluation of the clip pipeline: transcript segments in, ranked shorts out, metrics and a local JSON/Markdown report. It measures our own selection and boundary snapping logic. Third-party model accuracy is not a gate.

Owns the labelled-set format, transcript cache lookup, provider and baseline runs, metric definitions and report rendering. ASR, translation, transcript segmentation, suggestion generation, boundary snapping, export and provider adapters stay with their crates.

Network calls happen only when a live run is requested. No private media, transcripts, labels or reports are committed; reports are local artifacts under `target/clip-eval/`. The synthetic set (two fixtures and a pinned baseline) is the acceptance path before maintainer labels exist.

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

Label sets are capped by the AI clip contract (10,000 transcript segments, 4 MiB text, 1,024 windows, 64 KiB wire). The harness reads at most one label directory level and never writes outside `target/clip-eval/`.
