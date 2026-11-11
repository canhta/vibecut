# Subtitles

Interface: `run`, `prepare_ass_import`, `SubtitleRequest`, `SubtitleOutcome`, `AssImportRequest`,
`AssImportReport`, `CaptionAccess`, `decode_captions`, `decode_translations`; `LanguageTag` is re-exported. Selected providers come from
[`providers`](../providers/README.md).

## Subtitle workflow

`run` composes the file-to-subtitle workflow: speech-window extraction, explicit
transcription and translation provider choice, checkpointed ASR chunks, optional local
alignment, cue segmentation, translation application and the SRT/burn export jobs. It never
reimplements captions, render, provider consent or cost, or cache and checkpoint primitives;
speech windows, ASR checkpoints and cue translation live in [transcript](../transcript/README.md).

Provider setup, credentials and capability refusals belong to
[providers](../providers/README.md#failures).

`run(request, providers, runner, cache, consent, progress, cancellation)` imports the input,
extracts mono 16 kHz windows at speech-safe boundaries, reuses complete raw `AsrChunk` cache
entries keyed by source content, exact sample range, language and provider/model/settings, and
executes missing windows synchronously through execution hooks. Text-only recognition applies
the selected local aligner on that same thread; raw recognition is committed to the checkpoint cache
before alignment, so an alignment failure preserves completed ASR work. The workflow assembles
with `ai::assemble`, segments through captions, applies translation windows through
`TranslationRun`, and dispatches `export_subtitles`/`burn_subtitles` through the job runner.
Silent windows are skipped; no speech returns `NoSpeech`, mean word confidence below the fixed
floor returns `LowConfidence`, and unknown duration or burn without video return typed refusals.
No project file is written.

The retained blocking composition admits speech work through the runner's shared budget
before detection/extraction, using the [transcript memory ceilings](../transcript/README.md).
Pressure refuses before provider work and releases partial reservations on every exit.

`Cancellation` is the operation-wide cooperative signal; `run` checks it at every stage and
window boundary and cancels the active submitted job. Cancellation after a completed ASR window
keeps the checkpoint. Export keeps render's publication fence: once publication is sealed a late
cancel cannot turn the completed file into `Cancelled`, and cancellation before the fence
preserves the existing destination. The CLI has no signal-driven cancel, so process interruption
has crash semantics.

Remote uploads are prompted through `ConsentPrompt` before every request; `UploadRequest` carries
provider and model, languages, audio seconds, payload bytes, cost estimate and the adapter's
policy links. The workflow never uploads without prompt approval and never downloads weights;
model download belongs to `models::ModelTransfers` under `DownloadConsent`.

## Caption workflow commands

`captions.generate {asset, range?, source_language?}` starts one `Transcribe` job over an existing
project asset. The default range is the whole known duration. Omitted language is detected as the
first job stage through the selected provider. Remote transcription without a local aligner uses
the network class; all other transcription uses CPU. Speech extraction, checkpoint lookup,
recognition, alignment, assembly and segmentation run on that worker without nested jobs.

`CaptionAccess` lends resolved providers, a `ConsentPrompt` and the one shared checkpoint-cache
handle. Composition roots open the models cache once; every window locks it only for get/put.
Completed raw windows keep the existing source/sample/language/provider/model/settings identity,
including after failure or cancellation. Uncached remote windows ask the prompt before upload.
Generation and translation use the [shared approval boundary](../providers/README.md#adapters-scopes-and-consent),
including its separate exact-model unknown-price acknowledgement. The lent providers resolve
the selected model; control requests cannot replace that identity or grant consent.
Absent access refuses `Permission`; optional resource failures retain their setup category while
unrelated commands remain usable. The CLI control and headless MCP roots lend local speech
resources with `DenyUploads`, with no implicit download or client consent parameter.

The complete generated result contains source-timed cues and counts, language, asset/range,
provider/model and cost. `captions.apply {job, start?}` inserts that retained result as one edit,
placing the range at `start` (default zero). It requires the original Session and revision,
retained completed transcription, unchanged source identity and a still-valid asset range.
A second application under a fresh key is stale; replay of the original key stays idempotent.

`captions.translate {item, cues?, target_language, force?}` snapshots up to 10,000 distinct cues
(all cues when omitted), preserving manually edited target texts unless forced. Source context
and provider request windows are built at submission, at most 64 selected cues per window. The
single `Translate` job asks consent for each window and runs `TranslationRun::run` directly.
Retry invocations share a three-attempt ceiling and accumulated cost. Completed output contains
exact `retranslate` fields, provider/model and cost; applying it remains a separate ordinary edit.
Before publishing success, the job validates one complete `retranslate` request through
the existing parameter budget and wire frame parser, including dictionary keys and the
request envelope with maximum-length correlation/idempotency keys. A result that cannot
be applied atomically under those limits fails with `Limit` and publishes no output.
Every workflow refuses inside a transaction and never mutates the live Session in the worker.

Both result reservations and serialized outputs are capped at 8 MiB, generated cues at 10,000.
Oversize, malformed, failed or cancelled work yields no partial result. Outer control and preview
bounds remain owned by the [wire contract](../session/wire/README.md).

## ASS caption import

`captions.import { asset, language, start, track, name, styles, cues }` inserts the parser's
shared `captions::CueStyle` table and parsed `captions::AssCue` values as one `CaptionParams`
item on an existing text track or a newly allocated track named `name` (default `Captions`). Each
cue references a table entry by `CueStyleId`, so style data is stored once rather than per cue.
The asset is the already-read script identity; no path or file IO runs inside the command. The
asset, optional track and item form one command and one undo step, and `captions.import` works
inside a batch. Empty cue lists, more than `captions::ass::MAX_ASS_CUES` cues, more than
`captions::MAX_CUE_STYLES` table entries, an out-of-range cue style id, negative starts and
invalid cue styles refuse atomically. The resulting project is encoded before the session accepts
it, so any import the save path would refuse returns `Limit` without changing project or history.

`prepare_ass_import` reads at most `captions::ass::MAX_ASS_BYTES`, parses through the captions
owner and returns the registered command plus cue, style and skipped counts and line-numbered
warnings for headless adapters. It returns `Limit` for an oversized file and `Unsupported` when
no cue survives.

## Subtitle export commands

`export_subtitles` and `burn_subtitles` publish files without editing the Session or undo
history. Dispatch their strict JSON parameters with `Session::execute_export(runner, input, id,
params)` or their typed `Command` with `submit_export_command`; ordinary edit execution, batches
and active transactions refuse file publication. Both execute against a captured immutable
project snapshot in the JobRunner.

`export_subtitles` skips ordinary title items on the shared Text track and keeps caption items'
source windows and output gaps; each caption item's source window maps to its output start
through captions' `EditMapping`, preserving cuts, repeated occurrences and timeline gaps. Captions
owns the only SRT, VTT and TXT serializer, and `subtitle_file` exposes the same serialization
plus its typed repair warnings for preflight. The path opens no media and works with an offline
source. Jobs reserve bounded mapping and serialization scratch, write through project's
`StreamingStage`, finalize, then fence cancellation before atomic publication. Existing files
survive refusal or cancellation, source path aliases are refused through render's shared output
admission, and job output contains no transcript text.

`burn_subtitles` requires a visible caption item intersecting its range and forces encoding
through the production render export path; portrait selects 1080×1920 and otherwise the project
canvas is kept. Render owns pixel styling, media admission, encoding and atomic finalization;
missing or changed required video or audio returns its typed asset and range diagnostics and
preserves the previous destination.
