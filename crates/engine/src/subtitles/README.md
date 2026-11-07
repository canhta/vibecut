# Subtitles

Interface: `run`, `prepare_ass_import`; also reachable at their old `subtitles::` paths: `Providers`, `transcribe`, `translate_selected`.

## Subtitle workflow

`subtitles` owns the shared end-to-end composition used by the CLI and later UI:
speech-window extraction, explicit transcription/translation provider choice,
checkpointed ASR chunks, optional local alignment, cue segmentation, translation
application and the existing SRT/burn export jobs. It never reimplements captions,
render, provider consent/cost or cache/checkpoint primitives.

`provider_options()` is offline capability data (operation, model ids, languages,
local/remote and native timing) for pickers. `choose_transcription`/`choose_translation`
accept only a listed model and never fall back. `Runtime` selects local threads,
the explicit GPU request and the shipped Gipformer runner path. `local_asr_runner`
shares configured/bundled helper discovery between the desktop and CLI roots. `Credentials::new`
accepts composition-root secrets and validates them; errors name the environment
entry, never the value.

`Providers::open` loads only the selected adapters. Local Whisper/Gipformer models
come from verified `ArtifactCache` bundles and keep their `ModelLease` alive for the
adapter lifetime; text-only ASR additionally loads the pinned `whisper-small-dtw`
Candle aligner. Missing weights return `ModelMissing`, a missing runner
`RunnerMissing`, and missing credentials `CredentialMissing`. Remote adapters use
the official endpoints, shared `ureq` transport and their own disclosures.
`with_translation` attaches the explicitly selected translator only after an
auto-detected source language is known, so a same-language local run never requires
unrelated cloud credentials.

`run(request, providers, runner, cache, consent, progress, cancellation)` imports
the input, extracts mono 16 kHz windows at speech-safe boundaries, reuses complete
raw `AsrChunk` cache entries keyed by source content, exact sample range, language
and provider/model/settings, executes missing windows as Transcribe jobs, then
applies the selected local aligner as a separate CPU job when the recognition is
text-only. Raw recognition is committed to the checkpoint cache before alignment,
so an alignment failure preserves completed ASR work; alignment is recomputed on
the next run and its request is bound to the exact recognized text by
`ai::align`. The workflow then assembles with `ai::assemble`, converts and
segments through captions, applies translation windows through the existing
`TranslationRun`, and dispatches `export_subtitles`/`burn_subtitles` through the
real job runner. Silent windows are skipped; no speech returns `NoSpeech`, mean
word confidence below the fixed floor returns `LowConfidence`, and unknown
duration or burn without video return typed refusals. No project file is written.

`Cancellation` is the operation-wide cooperative signal. `run` passes its media
flag to extraction, checks it at every stage/window boundary, and cancels the
active submitted job; `wait_for_job` joins that job before returning. Cancellation
after a completed ASR window keeps the checkpoint. Export keeps render's existing
publication fence: once publication is sealed a late cancel cannot turn the
completed file into `Cancelled`, and cancellation before the fence preserves the
existing destination. The CLI currently exposes no signal-driven cancel gesture,
so process interruption remains crash semantics; UI/CLI callers that do have a
gesture pass the shared token.

Remote upload scopes are prompted through `ConsentPrompt` before every request;
`DenyUploads` is the default test refusal and `AllowAll` is the explicit test
grant. `UploadRequest` carries provider/model, languages, audio seconds, payload
bytes, cost ceiling and the adapter's official policy links. The
workflow never uploads without a prompt approval and never downloads weights.
The CLI downloads a catalog model through `models::ModelTransfers::open_native`, the same transfer
interface the desktop uses, granting `DownloadConsent` after its `--yes-download` action;
`default_cache_root` is the single shared model/checkpoint location. Provider choice, credentials,
consent and model availability live in `providers/`; speech windows, ASR checkpoints and cue
translation live in `transcript/`, which subtitles and the [shorts crate](../../../shorts/README.md) both call.

Tests in `tests/it/subtitles/workflow.rs` cover all six vi/en/zh translation directions and
every bilingual pair, SRT plus burned MP4 output, ASR checkpoint reuse on a second
run (and reuse of a checkpoint stored under the pre-move key), remote consent refusal with zero calls, credential refusal, catalog filtering
and a real generated fixture. They establish composition and failure contracts,
not model accuracy, live provider quality or Windows behavior.

## ASS caption import

`captions.import { asset, language, start, track, name, cues }` inserts parsed
`captions::AssCue` values as one `CaptionParams` item on an existing text track
or a newly allocated track named `name` (default `Captions`). The asset is the
already-read script identity; no path or file IO runs inside the command. The
item starts at `start`; source ranges stay in script coordinates and cue styles
remain per cue. The asset, optional track and item form one command and one undo
step, so undo removes all three. Empty cue lists, more than 100,000 cues,
negative starts and invalid cue styles refuse atomically. Every cue stores its
full style, so a result the project encoder would refuse (about 3,000 cues
under the 200,000-separator bound) is refused with `Limit` instead of leaving
a session that cannot be saved; a shared per-item style table would lift that.

`prepare_ass_import` reads at most 16 MiB, parses through the
captions owner and returns the registered command plus cue/style/skipped counts
and line-numbered warnings for headless adapters. It returns `Limit` for an
oversized file and `Unsupported` when no cue survives. `captions.import` works
inside a batch. `docs/product/agent-control/commands.md`, `catalog.digest` and
`cargo xtask catalog` remain the generated source of
truth.

## Subtitle export commands

The existing `registry()` also declares `export_subtitles` and `burn_subtitles`.
They publish files without editing the Session or undo history. Dispatch their
strict JSON parameters with `Session::execute_export(runner, input, id, params)`
or their typed `Command` with `submit_export_command`; ordinary edit execution,
batches and active transactions refuse file publication. Both execute against a
captured immutable project snapshot in the existing JobRunner.

`export_subtitles` takes `track` (a Text TrackId containing captions), `format` (`srt`, `web_vtt` or `txt`),
`languages` (one tag or two distinct tags in display order), native `path`, and
`overwrite` (bool). Ordinary title items on the shared Text track are skipped by
byte admission and cue serialization; caption items retain their source windows
and output gaps. `subtitle_file` exposes the same serialization plus captions'
typed repair warnings for preflight presentation. Each caption item's source
window maps to its output start through captions' EditMapping, preserving cuts,
repeated occurrences and timeline gaps. TXT is one cue per line with no clock.
Captions owns the only SRT/VTT/TXT serializer.
This path opens no media and works with an offline source. Jobs reserve bounded
mapping/serialization scratch, write through project's StreamingStage, finalize,
then fence cancellation before atomic publication. Existing files survive refusal
or cancellation; source path aliases are refused through render's shared output
admission. Job output contains no transcript text.

`burn_subtitles` takes `range` (integer ticks), `languages`, `style` (render's
CaptionStyle), `portrait` (bool), native `path` and `overwrite` (bool). Portrait
selects 1080×1920; otherwise the project canvas is retained. It requires a visible
caption item intersecting the range and forces encoding through the production
render export path. Render owns pixel styling, media admission, encoding and
atomic finalization; missing/changed required video or audio returns its typed
asset/range diagnostics and preserves the previous destination. Advanced codec,
canvas and placement choices remain available through `submit_export` with
render's ExportSettings. UI/CLI adapters consume these engine APIs; they do not
implement their own overlay or serializer.
