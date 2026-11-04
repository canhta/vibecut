# render

CPU reference compositor, shared sample-window mixer and bounded MP4 export.
Consumes immutable project snapshots; media owns codecs/conversion/mux, text owns
shaping/rasterization, project owns atomic publication, engine owns scheduling and
aggregate byte policy. No UI, effects graph, keyframes or persisted render state.

## Working pixels and scene

`Renderer::new(project, RenderSettings)` validates output geometry and per-item
normalized crop/destination rectangles. `render_frame(t, sources, hooks)` returns
one `Image`; `preview_frame` is the explicitly separate placeholder path. Track
order is bottom-to-top; hidden visual tracks and half-open inactive items produce
no pixels. Position maps at unit speed to the item's source range. Cover uses a
center crop, Contain letterboxes, and Stretch maps the selected crop. Background
blur renders behind every timeline layer. Selected caption languages admit up to
two languages; default selects the source language or the first available text.

Every working pixel is **RGBA, premultiplied alpha, linear-light f32**, finite
0..1 with RGB <= alpha. Compositing is Porter–Duff source-over. Bilinear sampling
has transparent borders; repeated box half-filters precede large downscales,
including odd-sized edges. The separable sampler reserves a bounded horizontal
coordinate table per draw and resolves vertical neighbors once per output row;
its arithmetic and transparent-border behavior match the scalar reference.
BT.709, sRGB and linear transfers convert at ingress;
text's premultiplied RGBA8 is unpremultiplied before transfer conversion, then
premultiplied once in linear light. Strided decoder RGBA rows convert directly
into working pixels without an intermediate packed image; padding is ignored,
row bounds are validated before allocation, and conversion checks cancellation
per row. Opaque video packing and alpha-aware packing use separate loops with
the same transfer tables and rounding. No gamma-space blending. `rgba8` produces
straight-alpha boundary pixels. `video_rgba8` composites residual transparency
over black in linear light before producing opaque BT.709 pixels for MP4.

File-backed new pixels admit 8-bit BT.709 primaries (unknown SDR tags use media's
BT.709 default), the supported transfers above, square pixels and identity display
matrices. Other primaries, HDR, high depth, non-square pixels and rotation return
`Unsupported`; no implicit gamut change, tone mapping or display transform.
Unchanged passthrough retains those source parameters instead. Output geometry
must be positive and at most 2^26 pixels; encoded H.264 dimensions must be even.

Text uses bundled Inter Regular and text's bundled fallback identity, centered
inside its destination. Pixel size scales with output/project width. Immutable
raster reuse is local to a Renderer, keyed by NFC text, font/fallback identity,
size, color and pixel geometry. Default caps are 8 MiB/four entries; settings cap
at 256 MiB/64 entries. Zero bytes disables reuse. LRU entries retain byte leases;
pressure clears idle entries before one retry. `text_cache_stats` reports hits,
misses, bytes and entries; `clear_text_cache` releases them. Observed repeat-frame
benefit and cached/uncached pixel equality are recorded in the ticket report.

## Subtitle styling and burn-in

`RenderSettings::caption_style` controls bundled font family/style, RGBA fill,
outer outline, rounded background box, top/middle/bottom position, safe area and
maximum total lines. Size, outline and box dimensions are pixels at 1080 output
lines and scale with output height in every aspect ratio. The safe area is a
0–40 percent inset on each canvas edge; the default is 5 percent. An explicit
item placement overrides that default destination. Caption geometry uses a
bounded line-height band inside the safe rectangle rather than a full-frame
raster. Settings are transient and do not change the project persistence schema.

Selected language payloads join in display order with a mandatory line break.
Unicode UAX #14 wrapping and bundled CJK fallback handle both Han scripts without
inserted spaces; the text owner normalizes NFC and keeps graphemes intact. Empty
payloads do not paint a box. Missing requested languages, a line-count overflow,
or text that cannot fit its destination return typed refusals; output never
silently drops the second language. Cue activity uses half-open source ranges,
so gaps have no caption pixels. Both `preview_frame` and `render_frame` call the
same overlay path. Outline, box, font, size, position, line limit, text and pixel
geometry participate in the existing bounded raster cache; there is no second
caption cache or colour conversion.

`tests/captions.rs` pins visually inspected Vietnamese/English and Hans/Hant
portrait goldens, NFC/NFD equality, safe-area clipping, gap boundaries, cached
versus uncached equality and zero preview/export pixel difference. Its encoded
MP4 test measures decoded pixel error and writes `caption-burn-decoded.png` under
`target/fixtures/render/`. The ignored `caption_frame_benchmark` measures the same
public renderer with and without raster reuse at 1080×1920; measurements and host
limits live in the ticket's local evidence report.

## Audio clock and headroom

`mix_audio(project, absolute_start_sample, frames, channels, sources, hooks)` is
the preview/export owner. Project rate defaults to 48 kHz and admits 8–192 kHz;
output channels are mono or stereo, with stereo default. Every source/item tick
boundary rounds once to the absolute output sample grid using Nearest. Silence
fills gaps and half-open inactive intervals. A block admits at most 65,536 samples
per channel and 4,096 audible spans. Stereo stays L/R; mono duplicates at unity;
stereo to mono averages L/R. Other source layouts are explicitly unsupported.

Media resamples each continuous source from its original clock, retaining filter
phase across arbitrary block partitions, splits and trims. Seeking backwards
restarts that source cursor. Source-rate decode and AAC priming are media-owned;
render never reapplies priming. Accumulation is f32 with constant item dB gain,
no normalization, clipping or limiter. Nonfinite input/overflow refuses the mix.
Preview can inspect `AudioBlock::peak`; export returns `AudioOverload` above
unity, preserving the old destination. Limiter/fade policy belongs to /42.

## Sources, bounds and cancellation

`FileSources` holds only active item cursors and bounded signature metadata.
Full content hashes and live probe checks admit required sources; frame-time
checks use file identity/length/mtime and final publication rehashes sources.
This detects observed changes; it is not an OS filesystem snapshot or protection
against a concurrent adversary restoring metadata. Missing/changed sources return
asset/item/source-range diagnostics. Strict final output refuses by default;
`allow_degraded` explicitly permits initially missing/changed assets as
black/silence with a receipt. New source failure during execution still refuses.
`SourceProvider` is the seam for an already validated source owner or test fixture;
it must uphold requested timing, color and audio contracts.

`ExecutionHooks` checks cancellation and reserves lifetime-bound bytes before
allocation under Decoded, Compositor or Export ownership. Engine's implementation
uses its one aggregate budget and cache reclamation; adapters must not charge the
same bytes twice. Decoded cursor ceilings include aligned native frames and a
conservative 96 MiB packet/probe allowance, plus bounded PCM/resampler expansion.
Plan expansion caps active visual layers and audible spans at 4,096. Image and
packing buffers are separately leased. A frame exceeding the budget returns a
typed error with requested bytes before allocation. Native/OS codec allocations,
fixed transfer tables, metadata and allocator overhead are not an RSS sandbox;
measure peak RSS separately. Sources and renderer release at completion/failure.

## Export and publication

`export` uses `FileSources`; `export_with_sources` accepts an explicit source
owner. Settings select exact range, canvas/crop, fps, bitrate/quality,
hardware-preferred or platform software encoder, audio and overwrite policy.
Frames are sampled at range.start + n/fps; output timestamps start at zero. Exact
non-keyframe cuts seek to a preceding keyframe and decode forward. Audio uses the
same absolute sample grid as preview. AAC delay/padding remain FFmpeg-owned.

Auto copies unchanged video when a single visible source covers the output with
no pixel changes. It copies compatible unchanged audio, or reencodes the exact
mixed/resampled audio window while retaining video packet bytes. Only the first
video/audio stream selected by decode is published. An explicitly accepted
keyframe cut additionally requires exact start/end keys (or source EOF), regular
requested fps and PTS==DTS; reordered/open-GOP candidates refuse forced copy and
Auto encodes instead. No requested boundary is silently moved. `HdrPolicy`
requires passthrough for HDR, otherwise refuses. Resume always returns a typed
unsupported error; a partial movie is never a reusable checkpoint.

Project's `StreamingStage` owns same-directory private staging, byte/seek bounds,
overwrite authorization and finalize/publish. Export validates aliases against
source paths (including Unix hard links), flushes encoders, finishes mux/trailer,
finalizes/syncs, releases working buffers, revalidates sources, then calls the
publication fence. `ExecutionHooks::begin_publish` must atomically arbitrate
cancellation for a concurrent owner. Engine reports Publishing and ignores a
cancel arriving after its fence, so a committed file is reported Completed.
Before the fence cancellation/failure preserves the prior file. After the atomic
publish there are no fallible progress callbacks or result allocations.
Crash-orphan, replacement and filesystem guarantees belong to
[project](../project/README.md#streaming-output).

## Evidence

`tests/contracts.rs`, `goldens.rs` and `media_pipeline.rs` own deterministic local
scenes and final encoded/decode comparisons, exact PCM partitions, cuts, crop,
HDR/display metadata, offline admission, byte pressure and failure boundaries.
Engine tests submit real jobs and exercise cancellation fences and worker unwind.
Required-oracle mode fails on absent ffmpeg/ffprobe; a found but broken tool always
fails. External FFmpeg shares codec algorithms, so it is a pipeline oracle rather
than an independent codec implementation. Fixtures are generated under target;
none are distributed. Windows runtime and QuickTime playback remain unverified.

Observed on the task /09 M1 Pro: explicit software H.264 cannot open a
VideoToolbox software encoder. This returns `media::MediaError::EncoderUnavailable`
with the native source retained and leaves existing output intact. Software
quality is unverified on this machine. Software H.264 fallback requires separate
codec/licence admission; selecting software never silently selects hardware.

Observed for task /28 on Apple M1 Pro, 16 GiB, macOS 27.2, release profile,
two alternating warm fresh-process runs of the production engine benchmark:
the exact 60 s/1,800-frame 1080p fixture exported to 1080x1920 in 55.69/52.60 s
wall (55.576/52.489 s internally, 32.39/34.29 fps), with peak RSS
257,048,576/258,834,432 bytes. The matching external crop/Lanczos hardware-encode
command took 9.31/9.37 s: a remaining 5.98/5.61x gap. All 1,800 decoded frames
match the pre-optimization compositor export exactly (SSIM 1, PSNR infinity);
against the differently cropped gamma-domain FFmpeg output, SSIM is 0.994169
and PSNR 38.998035 dB in both runs. Native profiling still observes full-frame
RGBA conversion, linear sampling and transfer packing; the external reference
avoids the linear float round trip. These measurements cover this fixture and
host, not other canvases, hardware or media.
