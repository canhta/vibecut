# Architecture

## Boundaries

The core runs headless: no window, display server, UI toolkit or direct platform
API is needed for editing, tests or CLI execution. Apps assemble concrete adapters;
`oscodec` alone owns codec FFI. AI adapters obey [privacy](open-source.md#privacy).
The desktop composition root uses egui and wgpu. See [decisions](../adr/README.md).

## Crate map

Planned layout; admit each crate only with its first working ticket. Table names
are Cargo package names. Allowed dependencies are direct workspace edges, including
optional, target-specific, build and dev dependencies; absence means forbidden.
External dependencies still require boundary and [license](open-source.md#licensing)
review. `cargo xtask layers` reads this table, rejects unknown members and edges,
and passes with only `xtask`. `xtask` is tooling, with no product dependencies.

| Package | Path | Responsibility | Allowed workspace dependencies |
|---|---|---|---|
| time | crates/time | Tick time, rational rates, source ranges | - |
| media | crates/media | Probe, source assets, pure Rust demux/mux/decode and codec traits | time |
| oscodec | crates/oscodec | Safe hardware codec adapter over platform FFI | time, media |
| project | crates/project | Timeline model, validation and project persistence | time, media, captions |
| captions | crates/captions | Transcript/cue model, segmentation, mapping, SRT/VTT | time |
| text | crates/text | Unicode shaping and rasterization | - |
| render | crates/render | Reference compositor, audio mix and export; injected codec traits | time, media, project, captions, text |
| ai | crates/ai | Provider contracts, consent-aware transcription/translation adapters | time, media, captions |
| engine | crates/engine | Session, commands, history, jobs and cache; adapter wiring | time, media, oscodec, project, captions, text, render, ai |
| vibecut-cli | apps/vibecut-cli | Headless command adapter and composition | engine, oscodec |
| vibecut | apps/vibecut | Desktop UI and composition | engine, oscodec |

Dependencies point from apps through engine to domain/processing crates and time.
`media` defines codec traits; engine/apps inject `oscodec`. Render consumes those
traits, never the concrete platform adapter. Core crates exclude egui/wgpu and
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
| Cue | captions | Source range, EN/VI text, edit flags and translation reference; preserve user edits unless explicitly commanded |
| EditMapping | captions | Pure source-to-output segments; drop outside cues, clamp intersections, preserve identity/edit flags; rebuild from source |
| Job | engine | Identity, kind, input fingerprint, bounded progress/stage, cancellation, typed failure and checkpoint |
| Session / commands | engine | Own Project/history; typed API plus `execute(command_id, params) -> Result<Outcome>` |

Everything that edits a project is a command. UI, CLI and future MCP/plugins use
the same engine registry: stable ID, label, parameter schema and enabled predicate.
Validate before committing; commands are all-or-nothing. A batch is one undo step;
continuous drags merge by key. UI-local presentation state may stay in the app.

## Media and time

Pure Rust owns demux/mux/decode. Hardware codecs use VideoToolbox on macOS and
Media Foundation on Windows through `oscodec`. FFmpeg is never linked or shipped;
external ffmpeg/ffprobe are [test oracles](verification.md#fixtures).
Time uses 254_016_000_000 integer ticks per second, exact for admitted rational
frame and audio sample rates. [Standards](standards.md#time-and-text) own conversions.
