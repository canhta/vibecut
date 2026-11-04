# Desktop media and time

Use egui with wgpu in the desktop app while preserving a UI-free headless core.
Own pure Rust decode/demux/mux and put hardware codecs behind a trait using
VideoToolbox on macOS and Media Foundation on Windows, keeping FFmpeg outside the
shipped product to preserve the Rust-first boundary. The single OS-codec FFI crate
may use narrowly allowed, documented unsafe modules behind a safe `Result` API;
integer tick time preserves common video rates including NTSC and audio sample
rates exactly, with its type name deferred until implementation. Current contracts
live in [architecture](../engineering/architecture.md) and
[source standards](../engineering/standards.md); product crate layout remains open.
