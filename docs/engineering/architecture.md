# Architecture

## Crate map

The core runs headless: editing, tests and the CLI need no window or UI toolkit;
only `apps/*` may depend on UI, native window/dialog, audio-device or accessibility
crates, including the app-owned wgpu adapter. Apps compose concrete adapters; the desktop app uses egui and wgpu. Admit each crate with
its first working ticket. `cargo xtask layers` enforces this table: an edge not listed
is forbidden (all dependency kinds count), and outside `apps/*` it denies the app-only
UI/GPU, native window/dialog, audio-device and accessibility crates, admitting one to that list only
with a named consumer.

| Package | Path | Owns | May depend on |
|---|---|---|---|
| time | crates/time | Ticks, rational rates, source ranges | - |
| media | crates/media | Safe FFmpeg I/O: probe, demux/mux, decode/encode, hardware acceleration, conversion, speech-audio preparation | time |
| project | crates/project | Timeline/animation model, validation/evaluation, persistence, atomic file writes | time, media, captions |
| captions | crates/captions | Transcript and cue model, segmentation, mapping, language profiles, SRT/VTT | time |
| text | crates/text | Unicode shaping and rasterization | - |
| render | crates/render | CPU compositor, caption overlay, audio mix, export execution | time, media, project, captions, text |
| ai | crates/ai | Provider interfaces, capabilities, consent and cost, AI adapters | time, media, captions |
| engine | crates/engine | Session, commands, history, recovery scheduling, jobs, cache, adapter selection | time, media, project, captions, text, render, ai |
| vibecut-cli | apps/vibecut-cli | Headless adapter: CLI arguments, typed exit codes, dispatch, project lease composition and export admission | engine, media, project, render, time |
| vibecut | apps/vibecut | Desktop app: UI, playback, wgpu compositor, preferences, app controls and inspection | engine, media, project, captions, text, render, ai |

Only `media` links FFmpeg. Render's CPU compositor is the reference shared by preview
and export; the app owns the optional wgpu adapter behind render's interface. Update
this table in the same change that adds a responsibility or edge.

## Contracts

Names here and in the [glossary](../../GLOSSARY.md) match the code; the owner crate's
README holds the details.

| Contract | Owner | Meaning |
|---|---|---|
| SourceAsset | media | Validated source identity, path, content hash, persistable probe |
| Project | project | Immutable validated timeline snapshot, versioned persistence and writer lease; see [project contract](../../crates/project/README.md#writer-lease-and-save-acknowledgement) |
| Param, keyframes | project | Item-relative parameter paths, validated channels, interpolation and time propagation; see [animation contract](../../crates/project/README.md#animated-parameters) |
| SourceRange | time | Nonempty half-open `[start, end)` in source ticks |
| Transcript | captions | Stable segments, optional word and token timing with raw/NFC/display correspondence, provider fingerprint |
| Cue | captions | Source range and per-language text with edit flags; user edits survive unless a command overrides them |
| EditMapping | captions | Pure source-to-output segments; one source range may appear several times |
| LanguageProfile | captions | Per-language segmentation, punctuation and font fallback data |
| Job | engine | Identity, input fingerprint, progress, cancellation, typed failure, checkpoint |
| Session, commands | engine | Owns immutable Arc<Project> snapshots and bounded history; typed Command and `execute(command_id, params) -> Result<Outcome>`; see [engine contract](../../crates/engine/README.md) |
| Codec interfaces | media | Reference-counted packets and frames, rational time bases, decode/encode/flush, seek |
| Export execution | render | Project snapshot, range and settings in; progress and cancellation hooks out |
| Local models | ai + engine | ai owns catalog/download consent/integrity and shared transport; engine owns transfers and native ArtifactCache bundles; see [model transfer contract](../../crates/engine/README.md#local-model-transfers) |
| Provider interfaces | ai | Transcriber, Aligner, Translator, ClipSuggester; capabilities per model and language; request-scoped consent |

**Jobs.** Engine wraps all media, render and AI execution in its job runner and
owns scheduling, retries, checkpoints and the aggregate byte policy; see the
[engine job contract](../../crates/engine/README.md#background-jobs-and-byte-policy).
Persistent job artifacts use content keys; frame/glyph caches stay with their data
owners and share accounting leases. Processing crates receive
progress and cancellation hooks and never build their own.

**Commands.** Every project edit is an engine command with a stable ID, label,
parameter schema and enabled predicate. UI, CLI, MCP and plugins all dispatch
through the same registry. Commands validate, then commit all-or-nothing; a batch
or closure group is one undo step; continuous drags merge by key or explicit
gesture begin/commit/cancel. Engine restores project, history and deferred changes
on refusal or unwind; see the [transaction contract](../../crates/engine/README.md#transactions-gestures-and-changes).
Presentation state and preferences stay in the app.

**Recovery.** Engine owns unsaved-session scheduling, OS session liveness and revision-bound save acknowledgement; project owns its one snapshot format, atomic writes and rotation. See the [session recovery contract](../../crates/engine/README.md#session-recovery-and-save-acknowledgement). The app presents recovery choices.

**Control.** Engine owns command IDs, schemas, events, revision and idempotency
rules, and every project mutation; dispatch is transport-neutral. Adapters only
translate: the CLI parses arguments and maps typed exit codes, MCP maps tools and
transports, the app maps widgets and preferences. All dispatch through the one
registry, never a second command catalog, and none mutates the project directly.
A shared control library is admitted only with working CLI and MCP consumers;
until then the first adapter owns the shared code.

**Translation.** AI owns one contextual TranslationWindow contract and structured response validation; engine selects cues, budgets/retries work and applies the registered `retranslate` command with edit protection. See [AI translation](../../crates/ai/README.md#contextual-text-translation) and [engine application](../../crates/engine/README.md#translation-selection-and-application).

**AI.** `ai` owns consent, cost estimates and capability checks once for every
operation; consent binds provider, operation, content and languages, and retries
stay inside it. Captions owns text correspondence and unit identity; translations
never reuse source word timing. Wholly untimed raw tokens may precede segment alignment;
measured token ranges require segment timing and an advertised capability.
Local Gipformer runs in ai's isolated native runner; see the
[local contract](../../crates/ai/README.md#local-gipformer). Details: [ai README](../../crates/ai/README.md).

## Languages

Languages are data. Tags are BCP 47 (`vi`, `en`, `zh-Hans`, `zh-Hant`); each has one
LanguageProfile in `captions` (segmentation limits, reading speed, line breaking,
punctuation, font fallback). Provider adapters declare language capabilities and
keep per-language decode settings as profile data. Adding a language adds a
profile, fonts and an optional UI catalog; unknown tags use the generic profile.

## Media and time

FFmpeg does probe, demux, decode, encode, mux, scaling and resampling. Codec
availability is queried at runtime; a missing encoder returns `Unsupported` or a
same-quality software path. Hardware frames stay on the GPU until CPU pixels are
needed. Frame-accurate cuts seek to the preceding keyframe and decode forward;
stream copy is used whenever the operation allows it.

Probe reports format, duration when known, rational time bases, codec parameters
including priming, and frame-count or VFR evidence; unknown stays unknown. Packet
and frame PTS are presentation coordinates.

Time is 254_016_000_000 integer ticks per second, exact for admitted frame and
sample rates; conversions follow [standards](standards.md#time-and-text).
