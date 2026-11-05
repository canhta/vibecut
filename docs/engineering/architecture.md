# Architecture

## Crate map

The core runs headless: editing, tests and the CLI need no window or UI toolkit; only `apps/*` may depend on UI, native window/dialog, audio-device or accessibility crates, including the app-owned wgpu adapter. Apps compose concrete adapters; the desktop app uses egui and wgpu. Admit each crate with its first working ticket. `cargo xtask layers` enforces this table: an edge not listed is forbidden (all dependency kinds count), and outside `apps/*` it denies the app-only UI/GPU, native window/dialog, audio-device and accessibility crates, admitting one to that list only with a named consumer.

| Package | Path | Owns | May depend on |
|---|---|---|---|
| time | crates/time | Ticks, rational rates, source ranges | - |
| media | crates/media | Safe FFmpeg I/O: probe, demux/mux, decode/encode, hardware acceleration, conversion, speech and vision media preparation | time |
| project | crates/project | Scene/timeline/animation model, validation/evaluation, persistence, atomic file writes | time, media, captions |
| captions | crates/captions | Transcript and cue model, segmentation, mapping, language profiles, SRT/VTT | time |
| text | crates/text | Unicode shaping and rasterization | - |
| render | crates/render | CPU compositor, caption overlay, audio mix, export execution | time, media, project, captions, text |
| ai | crates/ai | Provider interfaces, capabilities, consent and cost, AI adapters | time, media, captions |
| engine | crates/engine | Session, commands, history, recovery scheduling, jobs, cache, adapter selection, clip batch export and per-shot analysis | time, media, project, captions, text, render, ai |
| clip-eval | crates/clip-eval | Fixed labelled clip-pipeline evaluation harness, metrics and local reports | time, media, captions, ai, engine |
| vibecut-cli | apps/vibecut-cli | Headless adapter: CLI arguments, typed exit codes, dispatch, control-session lifetime, project lease composition, export admission, bounded batch job files, MCP tools and authenticated local transport | engine, media, project, render, time |
| vibecut | apps/vibecut | Desktop app: UI, playback, model-manager presentation, wgpu compositor, preferences, app controls and inspection; composes the CLI-owned MCP adapter | time, engine, media, project, captions, text, render, ai, vibecut-cli |

Only `media` links FFmpeg. Render's CPU compositor is the reference shared by preview and export; the app owns the optional wgpu adapter behind render's interface. Update this table in the same change that adds a responsibility or edge.

## Contracts

The [glossary](../../GLOSSARY.md) owns domain definitions; crate READMEs own contract details. This index records the seams:

| Area | Contract owner |
|---|---|
| SourceAsset, codecs, probe, packet/frame timing | [media](../../crates/media/README.md#entry-points-and-timing) |
| Project, scenes, validation, persistence, writer lease | [project](../../crates/project/README.md#writer-lease-and-save-acknowledgement) |
| Param and keyframes | [project animation](../../crates/project/README.md#animated-parameters) |
| SourceRange | [time](../../crates/time/README.md) |
| Transcript, Cue, EditMapping, LanguageProfile | [captions](../../crates/captions/README.md) |
| Managed catalog, copied-media publication and catalog history | [engine library](../../crates/engine/README.md#managed-project-library); project retains metadata/persistence |
| Session, registry, transactions, gestures, undo/redo | [engine](../../crates/engine/README.md#transactions-gestures-and-changes) |
| Control schemas, revisions, receipts, resync events | [engine control](../../crates/engine/README.md#control-admission-revisions-and-receipts) |
| Job runner, scheduling, retries, checkpoints, aggregate bytes | [engine jobs](../../crates/engine/README.md#background-jobs-and-byte-policy) |
| Font catalog, system/bundled/Google fonts, download cache, `fonts.*` commands | [engine fonts](../../crates/engine/README.md#font-catalog); resolution and raster in [text](../../crates/text/README.md#fonts) |
| Row tasks and native conversion worker allowance | [engine kernel execution](../../crates/engine/README.md#render-kernel-execution) |
| Export execution | [render](../../crates/render/README.md#export-and-publication) |
| Clip naming, cue mapping, per-short cancel/retry | [engine batch export](../../crates/engine/README.md#clip-batch-export) |
| Local model catalog, consent, integrity, transport / transfers, cache bundles / manager presentation | [ai catalog](../../crates/ai/README.md#model-catalog-and-language-neutral-weights) / [engine transfers](../../crates/engine/README.md#local-model-transfers) / [app manager](../../apps/vibecut/README.md#local-model-manager) |
| Clip boundary snapping / speech groups / energy | [engine](../../crates/engine/README.md#clip-boundary-adjustment) / [captions](../../crates/captions/README.md#source-cut-units) / [media](../../crates/media/README.md#speech-energy-windows) |
| Shot analysis and complete artifact caching | [engine analysis](../../crates/engine/README.md#per-shot-analysis) |
| Vision preparation / inference, face embeddings and shape-plan cache | [media](../../crates/media/README.md#vision-preparation) / [ai detectors](../../crates/ai/README.md#local-vision-detectors) |
| Provider capabilities, consent, cost / text timing / isolated Gipformer | [ai](../../crates/ai/README.md#consent-and-cost) / [recognition](../../crates/ai/README.md#recognition-and-time) / [local runner](../../crates/ai/README.md#local-gipformer) |
| Sound search, saved local sounds, preview cache and attribution | [engine sounds](../../crates/engine/README.md#sound-library) |
| Translation windows and response validation / selection, retry, edit protection | [ai translation](../../crates/ai/README.md#contextual-text-translation) / [engine application](../../crates/engine/README.md#translation-selection-and-application) |
| SpeechSynthesizer capabilities, requests, consent, PCM validation | [ai speech](../../crates/ai/README.md#speech-synthesis) |

Engine wraps media/render/AI execution, supplying progress and cancellation hooks. Persistent artifacts use content keys; frame/glyph caches stay with data owners and share accounting leases. Engine owns speech provider selection, retry budgets, jobs and placement.

Every project mutation goes through the engine registry; UI/CLI/MCP/plugins translate their transport into it. CLI owns parsing/typed exit codes, MCP tools/transports, app widgets/preferences/transient state. A shared control library needs working CLI and MCP consumers; until then its first adapter owns the code.

Per-project view state stays outside editable content and undo: engine Session exposes its command, project owns the bounded [metadata sidecar](../../crates/project/README.md#optional-project-metadata) under WriterLease. Engine owns [recovery scheduling, session liveness and save acknowledgement](../../crates/engine/README.md#session-recovery-and-save-acknowledgement); project owns snapshot format/writes/rotation, app presents recovery choices.

## Languages

Languages are BCP 47 data (`vi`, `en`, `zh-Hans`, `zh-Hant`), each with one [captions profile](../../crates/captions/README.md#entry-points-and-persistence). Provider adapters declare language capabilities and keep per-language decode settings as profile data. Adding a language adds a profile, fonts and an optional UI catalog; unknown tags use the generic profile.

## Media and time

Runtime codec admission, software fallback, GPU frame lifetime, keyframe seeking, stream copy, probe fields and presentation timestamps follow the [media contract](../../crates/media/README.md#entry-points-and-timing).

Time units follow the [glossary](../../GLOSSARY.md); conversion rules follow [standards](standards.md#time-and-text).
