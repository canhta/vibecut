# Desktop media and time

Use egui with wgpu in the desktop app while preserving a UI-free headless core.
Use dynamically linked, LGPL-only FFmpeg for media I/O, demux, decode, encode,
mux, pixel conversion and audio resampling. Maintaining codecs and containers is
outside the editor's responsibility. Hardware acceleration is selected through
FFmpeg; Rust owns timeline semantics, the compositor, text/caption rendering and
job orchestration. Preview and export consume the same compositor pixels.

Use the maintained `ffmpeg-next` safe wrapper and its matching system bindings.
The permissive WTFPL dependency license is admitted for these wrappers. Keep any
necessary direct FFI inside narrowly allowed modules of `media`, behind safe
`Result` APIs; do not own a second bindings layer. Provision current stable shared
LGPL builds; exclude GPL/nonfree components. Backport verified upstream security
fixes as patch files citing their upstream commits, applied by `cargo xtask ffmpeg`
on every platform. Fail provisioning when a patch no longer applies; review and
delete it once a stable release contains the fix. Build Windows from the same
patched source, never an unpatched prebuilt. Cache identity includes the patch
contents so an older SDK cannot bypass a new security fix. Distribution rules live in
[open-source policy](../engineering/open-source.md#ffmpeg-distribution).

Integer ticks at 254,016,000,000 per second preserve admitted video and audio
rates exactly. FFmpeg owns demux presentation timestamps, codec priming and
reordering; convert rational timestamps at the Rust boundary without applying
container edits a second time. Current contracts live in
[architecture](../engineering/architecture.md) and
[source standards](../engineering/standards.md).
