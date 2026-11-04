# Architecture

## Boundaries

The core runs headless: no window, display server, UI toolkit or direct platform
API is needed for editing, tests or CLI execution. Apps assemble concrete adapters;
`oscodec` alone owns codec FFI. AI adapters obey [privacy](open-source.md#privacy).
The desktop composition root uses egui and wgpu. See [decisions](../adr/README.md).

## Crate map

Planned layout; admit each crate only with its first working ticket. Cargo metadata
records admitted members; table names are Cargo package names. Allowed
dependencies are direct workspace edges, including optional, target-specific,
build and dev dependencies;
absence means forbidden.
External dependencies still require boundary and [license](open-source.md#licensing)
review. `cargo xtask layers` reads this table, rejects unknown members and edges,
and passes with only `xtask`. `xtask` is tooling, with no product dependencies.

| Package | Path | Responsibility | Allowed workspace dependencies |
|---|---|---|---|
| time | crates/time | Tick time, rational rates, source ranges | - |
| media | crates/media | Source assets, probe, demux/mux, codec interfaces, software decode and speech-audio preparation | time |
| oscodec | crates/oscodec | Safe hardware codec adapter over platform FFI | time, media |
| project | crates/project | Timeline model, validation and project persistence | time, media, captions |
| captions | crates/captions | Transcript/cue model, segmentation, mapping, SRT/VTT | time |
| text | crates/text | Unicode shaping and rasterization | - |
| render | crates/render | CPU compositor, caption overlay, audio mix and export execution; injected codecs | time, media, project, captions, text |
| ai | crates/ai | Provider interfaces, capabilities, consent/cost policy and transcription/alignment/translation adapters | time, media, captions |
| engine | crates/engine | Session, commands, history, jobs/cache and processing orchestration; adapter selection | time, media, oscodec, project, captions, text, render, ai |
| vibecut-cli | apps/vibecut-cli | Headless command adapter and composition | engine, oscodec, media, ai |
| vibecut | apps/vibecut | Desktop UI, device playback, GPU adapter, presentation/preferences and composition | engine, oscodec, media, project, captions, text, render, ai |

Dependencies point from apps through engine to domain/processing crates and time.
`media` defines codec traits; engine/apps inject `oscodec`. Render consumes those
traits, never the concrete platform adapter. The desktop app owns the optional
wgpu compositor adapter; render owns its device-free interface and CPU reference,
shared by preview and export. Core crates exclude egui/wgpu and
platform bindings; `oscodec` admits platform codec bindings only. Update this table
in the same change as an admitted responsibility or edge. App tickets add actual
app members to the root manifest; do not create empty app crates in advance.

## Contracts and commands

Planned public contracts; implementation tickets reconcile final names here and
in the [glossary](../../GLOSSARY.md), without creating a second model owner.

| Contract | Owner | Required meaning |
|---|---|---|
| SourceAsset | media | Identity, path, content hash and probe: duration, codec/size/rational rate/VFR and audio rate/channels/start offset |
| SourceRange | time | Nonempty half-open `[start, end)` in source ticks |
| Transcript | captions | Stable segments and optional word ranges/confidence; provider/model/settings fingerprint |
| Cue | captions | Source range, source-language text and per-language text with per-language edit flags and translation reference; preserve user edits unless explicitly commanded |
| EditMapping | captions | Pure source-to-output segments; a range may yield several intersections; preserve source identity/edit flags with deterministic fragment identity; rebuild from source |
| Job | engine | Identity, kind, input fingerprint, bounded progress/stage, cancellation, typed failure and checkpoint |
| Session / commands | engine | Own Project/history; typed interface plus `execute(command_id, params) -> Result<Outcome>` |
| Codec interfaces | media | Packet/frame metadata, capabilities, decode/encode/flush and typed unsupported input; software and OS-codec adapters use the same interface |
| ExportSettings / execution | render | Immutable project snapshot, output range/settings and injected codecs/compositor; execution reports progress/cancellation without owning an engine Job |
| Provider interfaces | ai | Transcriber, Aligner and Translator; capabilities per model/mode/language, validated results, request-scoped consent and cost estimate |
| LanguageProfile | captions | Language data lookup with generic fallback; callers pass resolved rules/fonts to processing modules |

`engine` wraps render/AI/media execution in its Job runner; processing crates
never import engine or create competing Job, retry, checkpoint or cache protocols.
Pass cancellation/progress hooks into execution; engine retains scheduling, retry
budgets and promotion of completed outputs. Apps inject concrete adapters;
engine selects them by capability and applies software decode fallback once.
An absent encoder returns `Unsupported`; software decode is not software encode.
The project persistence module owns reusable atomic-file writes; app preferences
reuse that implementation while retaining their own schema. Desktop composition
may consume captions profiles and text font resolution directly; these dependencies
do not move processing or command policy into widgets.

AI's offline execution seam is documented in [its crate contract](../../crates/ai/README.md).
It accepts absolute source-sample audio windows, returns validated recognition with
explicit missing timing, and converts fully word/segment-timed results to captions'
Transcript. Character/token layers remain explicit transport data until captions
admits them. Configuration is versioned and location-tagged; consent binds the
provider, operation, content, languages, text and source window. Cost bounds are
per invocation; engine retains aggregate retry/chunk budget accounting.

AI owns upload consent, estimates and provider/model capability validation once;
transcription, translation and clip suggestion adapters reuse that policy. Consent
covers the provider, payload and operation; retries/chunks stay within that scope.
LanguageProfile lookup and caption-text normalization belong to captions; text consumes
plain layout/font inputs without depending on captions. Text still validates its
own public input; validation of a different trust seam is not duplicate logic.

Everything that edits a project is a command. UI, CLI and future MCP/plugins use
the same engine registry: stable ID, label, parameter schema and enabled predicate.
Validate before committing; commands are all-or-nothing. A batch is one undo step;
continuous drags merge by key. UI-local presentation state stays in the app.
The app action registry maps gestures
to engine command IDs or app-local actions; command schemas and enabled predicates
remain engine-owned. App preferences have one app-owned store, separate from Project
and project undo history. Headless callers need no widget, device or preference state.

## Languages

Languages are data, not code paths. Identify them with BCP 47 tags (`vi`, `en`,
`zh-Hans`, `zh-Hant`). Each supported language has one language profile owned by
`captions`: segmentation rules (grapheme or character limits, reading speed, line
breaking, optional word segmenter), punctuation, and default font fallback chain
consumed by `text`. Provider adapters declare supported languages as capabilities.
Adding a language adds a profile, fonts and optional UI catalog; core crates never
`match` on a fixed language list. Unknown tags fall back to a generic profile.

## Media and time

Pure Rust owns demux/mux/decode. Hardware codecs use VideoToolbox on macOS and
Media Foundation on Windows through `oscodec`. FFmpeg is never linked or shipped;
external ffmpeg/ffprobe are [test oracles](verification.md#fixtures).
Time uses 254_016_000_000 integer ticks per second, exact for admitted rational
frame and audio sample rates. [Standards](standards.md#time-and-text) own conversions.
