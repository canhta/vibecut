# Architecture

## Boundaries

The Rust core runs headless. Core crates must not depend on UI toolkits or OS APIs;
platform and presentation adapters depend inward on core contracts. Review actual
dependency edges when admitting crates. The desktop app crate uses egui and wgpu.

AI providers sit behind adapters. OpenAI comes first; local models follow later.
Remote processing obeys the [privacy contract](open-source.md#privacy).

## Media and time

Implement decode, demux and mux in our own pure Rust crates. Hardware encode/decode
uses OS codecs behind a trait: VideoToolbox on macOS and Media Foundation on Windows.
Keep FFmpeg out of the product; ffmpeg/ffprobe may run only as external test oracles
or fixture generators, never linked or shipped. The single OS-codec FFI crate follows
the [unsafe exception](standards.md#safety).

Use integer tick time for all edit/timing math, exact for common video frame rates
including NTSC and audio sample rates. Choose the type name when its crate lands.

The workspace contains only `xtask` automation and an empty `crates/` reservation.
Product crate layout remains undecided. See [ADR 0002](../adr/0002-desktop-media-and-time.md)
for the rationale behind these boundaries.
