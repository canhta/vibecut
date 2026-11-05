# render

CPU reference compositor, shared sample-window mixer and bounded MP4/MOV/WebM export.
Consumes immutable project snapshots and renders/mixes only their active scene; media owns codecs/conversion/mux, text owns
shaping/rasterization, project owns atomic publication, engine owns scheduling and
aggregate byte policy. No UI, device or persisted render state; it evaluates
project-owned keyframed opacity.

`ExportSettings::from_project` uses media's platform encoder default: software
encoding on Windows pending p0-core/78 hardware picture-integrity verification.
Hardware decoding remains enabled; requested quality and bitrate are retained.

## Working pixels and scene

`Renderer::new(project, RenderSettings)` validates output geometry and per-item
normalized crop/destination rectangles. `render_frame(t, sources, hooks)` returns
one `Image`; `preview_frame` is the explicitly separate placeholder path. Track
order is bottom-to-top; hidden visual tracks and half-open inactive items produce
no pixels. Position maps at unit speed to the item's source range. Cover uses a
center crop, Contain letterboxes, and Stretch maps the selected crop. Background
blur renders behind every timeline layer. Selected caption languages admit up to
two languages; default selects the source language or the first available text.

An item's `opacity` animation overrides its static `Placement.opacity`. Evaluation
calls project's `Animations::value_at` with item-local ticks, the same evaluator the
engine uses; render adds no interpolation path. Values are clamped to the
compositor's 0..=1 range. Missing opacity data keeps the static placement, and
other animation paths stay unconsumed until their owning feature lands.

Every working pixel is **RGBA, premultiplied alpha, linear-light f32**, finite
0..1 with RGB <= alpha. Compositing uses the item's blend function followed by
Porter–Duff source-over. Bilinear sampling
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

Text uses the item's bundled font family, centered inside its project-owned
destination. Positions are canvas percentages of the box centre, with clipping at
canvas edges rather than clamping the box; the default 50/84.5 reproduces the
original lower-third box. An optional project-owned background box paints before
the glyphs. Pixel size and box padding scale with output/project width. Immutable
raster reuse is local to a Renderer, keyed by NFC text, font/fallback identity,
size, color, background and pixel geometry. Default caps are 8 MiB/four entries; settings cap
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
payloads do not paint a box. Captions wrap at the requested size first. When they
exceed the line count or destination, a bounded ten-step search reduces the font
size to fit all text, down to one output pixel (or the requested size if smaller).
Already-fitting captions retain their exact pixels; outline and background padding
stay fixed. Missing requested languages, mandatory breaks exceeding the line count,
or text that still cannot fit at the minimum size return typed refusals; output
never silently drops the second language. Cue activity uses half-open source ranges,
so gaps have no caption pixels. Both `preview_frame` and `render_frame` call the
same overlay path. Outline, box, font, size, position, line limit, text and pixel
geometry participate in the existing bounded raster cache; there is no second
caption cache or colour conversion.

`tests/it/captions.rs` pins visually inspected Vietnamese/English and Hans/Hant
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
render never reapplies priming. Accumulation is f32 with per-item dB gain and
held mute evaluation through project's item-local animation evaluator. Each
sample maps from the absolute output sample index to ticks before evaluation;
block size and other tracks ending cannot change its gain. Video source audio
uses the same volume/muted controls; `source_audio_enabled=false` excludes it
regardless of picture visibility. Nonfinite input/overflow refuses the mix.

`AudioMixer::new(snapshot, channels, fade_ms, hooks)` retains bounded DSP state for
sequential realtime or offline blocks. Its linked master chain uses a -1 dBFS
threshold, 20:1 hard knee, one-pole 1 ms attack / 120 ms release, 5 ms lookahead
plus eight samples of FIR detector alignment, and a final hard sample clamp.
A Kaiser-windowed polyphase detector estimates intersample peaks; this is not a
certified output true-peak ceiling. Post-limiter gain is 0.98. Below-threshold
signals are unchanged by dynamics and still receive that post gain. All paths
apply the chain, including quiet offline exports; no block-dependent bypass.
`latency_samples` reports the full delay, removed by priming and feeding ahead,
so returned PCM stays on the requested output sample grid without leading delay
or lost final samples. Gaps/empty content stay bit-exact zero.

Retain one mixer for an immutable snapshot. Discontinuous seeks deterministically
replay from timeline zero in bounded, cancellable blocks to reconstruct the full
release envelope; cost grows with seek position. `mix_audio` is a convenience
for independent windows and uses this same graph. Playback owns one mixer per
request epoch; export owns one per operation. Sources and DSP leases remain
caller-owned; a failed read invalidates the graph cursor before retry.

`ExportSettings::audio_cut_fade_ms` optionally applies linear ramps at audible
item edges on the absolute sample grid (0 disables, maximum 1000 ms, capped at
half item duration). Clip batch exports use 10 ms. All included export audio
runs through the graph and encodes AAC at 192 kbit/s for MP4/MOV or Opus for
WebM, even when unchanged video packets copy. Media owns codec priming/FIFO.

Horizontal interpolation retains only two destination-width rows per active
pixel task and reuses them when adjacent output rows share source neighbors.
Vertical interpolation and source-over keep their original floating-point order;
fully opaque samples replace the destination after the same clamp. Transfer
packing uses exact float-bit transition buckets derived from the original
16-bit transfer table. Monotonic endpoint and threshold checks cover all finite
working values for the three admitted transfers, preserving its quantization.

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
`video_window` may return transparency outside an explicitly requested horizontal
sampling range while retaining full image geometry. Its default calls ordinary
`video`. File-backed cover/crop renders request the shared transform's footprint;
media converts an aligned region with a reconstruction halo, and ingress writes
it into its original source coordinates. Fractional sampling does not move.
Half reduction, background blur and runtime dimension changes use full frames.
Preview and export use this same path; no second compositor or cache is involved.


`ExecutionHooks::parallelism` defaults to one and bounds row kernels and native
conversion to at most eight workers. `run_tasks` defaults to sequential execution;
engine supplies joined borrowed tasks. Render reserves disjoint destination
buffers and small task metadata before dispatch; kernels check cancellation per
row. An engine worker's unwind is typed, and every borrowed task finishes before
the operation releases buffers or returns an error. Native conversion reserves
an additional conservative 8 MiB per extra worker for each active converter or
encoder; engine separately reserves each extra row worker's 512 KiB stack.
Native admission lowers the worker count before allocating when additional
workers do not fit, retaining the original one-worker minimum and typed byte
refusal. Engine uses sequential row tasks when extra stacks do not fit. These
are bounded concurrency/memory tradeoffs, not an OS RSS sandbox.

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
The initial encoder preference follows [media's platform default](../media/README.md#input-and-quality-boundaries).
`ExportSettings::container` defaults to MP4. MP4/MOV encode H.264 + AAC;
WebM encodes VP9 + Opus using media's existing encoders. Output is selected by the
settings, never guessed from the filename. Auto copies only an admitted container
video codec (H.264/HEVC/MPEG-4 for MP4/MOV; VP8/VP9/AV1 for WebM); otherwise it
encodes, and forced copy refuses. Included audio always mixes and encodes for the target so mastering cannot be bypassed.
Optional `degraded_consent` binds an interactive caller's `allow_degraded` to the
accepted typed asset/item/range/reason diagnostics. Any additional unavailable
range or changed reason refuses, so a retry cannot silently expand that consent.
CLI callers can retain their explicit whole-export `allow_degraded` switch.

Frames are sampled at range.start + n/fps; output timestamps start at zero. Exact
non-keyframe cuts seek to a preceding keyframe and decode forward. Audio uses the
same absolute sample grid as preview. AAC delay/padding remain FFmpeg-owned.

Auto copies unchanged video when a single visible source covers the output with
no pixel changes. An opacity override (including a channel with no keys) disables
video copy; Auto encodes it and forced stream copy refuses it. It encodes the exact mastered audio window while retaining video packet bytes. Only the first
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

`tests/it/contracts.rs`, `goldens.rs` and `media_pipeline.rs` own deterministic local
scenes and final encoded/decode comparisons, exact PCM partitions, cuts, crop,
HDR/display metadata, offline admission, byte pressure and failure boundaries.
Engine tests submit real jobs and exercise cancellation fences and worker unwind.
Required-oracle mode fails on absent ffmpeg/ffprobe; a found but broken tool always
fails. External FFmpeg shares codec algorithms, so it is a pipeline oracle rather
than an independent codec implementation. Fixtures are generated under target;
none are distributed. Windows runtime and QuickTime playback remain unverified.

Task /31 corrects the /09 software H.264 failure: the platform software encoder
exists and the rejected QSCALE setting caused the earlier open failure. Encoder
selection, rate-control fallback and typed errors follow the
[media contract](../media/README.md#input-and-quality-boundaries).
Observed on Apple M1 Pro/macOS 27.2: the real software-export regression produces
30 H.264 frames plus AAC with maximum decoded color-patch channel error <=6.
Intel/Rosetta, VM-only fallback and Windows runtime remain unverified.

Observed for task /30 on Apple M1 Pro, 16 GiB, macOS 27.2, release profile:
nine alternating warm fresh-process pairs through the production engine benchmark
exported the exact 60 s/1,800-frame 1080p fixture to 1080x1920 in 15.99–20.66 s
process wall, versus the matching external crop/Lanczos hardware-encode command's
9.29–9.55 s. The median paired wall ratio is 1.863734x. The lowest-prepair-load
run (one-minute host load 10.19) takes 16.09 s / 15.987541 s internally, versus
FFmpeg 9.55 s (1.684817x); three individual pairs exceed 2x under shared load.
The contemporaneous exact-base renderer takes 53.98 s. Current peak RSS ranges
266,452,992–275,103,744 bytes; sampled aggregate charged peak is 560,156,672.
This trades bounded worker stacks/native conversion allowances and fixed packing
tables for throughput; the original minimum-budget export still works by reducing
concurrency. Every one of the nine outputs matches all 1,800 decoded /28 frames
exactly (SSIM 1, PSNR infinity); external-reference SSIM is 0.994169 and PSNR
38.998035 dB. Preview/export pixels, fractional crops and linear premultiplied
math are shared. These observations cover this fixture and host, not other
canvases, hardware, formats or guaranteed latency under concurrent workloads.

## Effects and desktop adapter

`effects::EffectCompositor` receives a working item image, evaluated ordered pass
list, optional item/mask coverage and the already-composited track-below image.
`CpuEffects` is the reference and CLI/export default; `RenderSettings.effects`
optionally injects the desktop adapter. Unsupported GPU capability falls back to
CPU; cancellation, budget and invalid-data failures propagate. The renderer paints
an effected item into a transparent output-sized layer, filters it, then composites
it over the unchanged below image. Effects disable video stream copy.

Blur intensity uses the static project value unless its scalar `Param` override
exists. Evaluation uses project item-local time; animated overshoot clamps to
0–100. Sigma is intensity/5 times min(output width/1920, output height/1080).
The normalized separable Gaussian clamps edge taps and processes all RGBA channels
in premultiplied linear f32, without colour/alpha conversion. Sigma <=0.01 output
pixels is an identity. Above sigma 8, n=ceil((sigma/8)^2) iterations use sigma/sqrt(n)
per axis, capped at 256 iterations and 49 taps per pass. Independent finite kernels
approximate the untruncated Gaussian; they do not claim source bit identity.

The desktop WGSL adapter consumes the same pass weights/entry names and admits
maximum absolute CPU/GPU working-channel error 2e-5. Deterministic mixed-alpha
fixtures at 1x/2x and a repeated-pass fixture exercise the interface in desktop
tests; render tests pin sigma at 1080p/4K, channel energy, transparent coverage,
animated overrides, budget refusal and cancellation. A missing native GPU is a
labelled skip in offline CI and a failure in required-oracle mode. Measurements
and benchmark commands live in the ticket's local report.

## Mask coverage primitives

`mask::FlatMask` rasterizes an evaluated closed contour with nonzero winding;
`prepare_builtin` supplies rectangle, cinematic band, ellipse, heart, diamond
and star contours, with cubic chord tolerance 0.05 output pixels. `SplitMask`
evaluates an unbounded half-plane so frame edges never introduce artificial
feather or stroke. Feather is a symmetric signed-distance ramp: a one-pixel
linear ramp at zero feather, smoothstep over the feather width at >=1 pixel.
Expansion precedes invert, then opacity scales coverage. Stroke uses distances
on the retained/removed sides for inside/centre/outside alignment, with independent
one-output-pixel antialiasing; mask feather affects body coverage only.

`RasterMask::from_image` consumes already-ingressed glyph alpha and reuses text's
Euclidean distance transform. No glyph renderer or transfer conversion lives in
the mask engine. One prepared distance raster serves coverage and stroke passes.
All coverage images use the compositor's premultiplied linear space; `scale_by`
scales every item channel together. Effects consume working pixels before masking. Geometry mismatches refuse before mutation. Contour/scratch,
raster-distance and image allocations hold compositor reservations; cancellation
is checked per row, including text's distance-transform rows/columns.

`Renderer` consumes project mask vectors for every visual layer. Its order is:
item effects, combined coverage in vector order, enabled strokes in vector order,
then item opacity, blend and source-over. None modes are skipped. Strokes remain visible
outside the combined coverage. Only the current mask's raster/distance data is
retained; accumulated coverage and accumulated premultiplied strokes are bounded
working images. Active masks disable video stream copy for Auto and forced-copy
export admission.

Mask geometry uses the placed item's output rectangle. Project-pixel appearance
widths scale by the smaller output/item axis ratio; text uses the existing bundled
font/cache/ingress and height scaling, then a working-space affine sample and one
shared distance transform for coverage and stroke. Cubic flattening and contour
geometry have a 65,536-point admission bound; unsupported/extreme output geometry
refuses before uncontrolled allocation. No mask-local glyph renderer or cache.

`tests/it/masks.rs` exercises the public mask kernels and all built-ins through the
production Renderer at two resolutions. The compositor golden includes masks,
blur, transparent source edges, half item opacity and an encoded/decode oracle;
engine and CLI own edit/history/persistence and real rectangle export checks.

The ignored production-frame benchmark in [tests/it/masks.rs](tests/it/masks.rs)
measures enabled-mask cost against an unmasked baseline. The local task report
records hardware, shared host load, process RSS and quality metrics; these
measurements do not establish codec throughput or UI latency.
## Blend formulas in working space

`blend::composite` is the CPU reference for preview and export, including CLI.
It accepts equal-size working images and finite opacity in 0..=1. Effects and
mask/stroke coverage precede blend; opacity scales every source channel exactly
once after those operations. Zero opacity leaves the backdrop bit-identical and
the renderer skips that item, including its background-blur contribution.

Let `b`, `s` denote straight **linear-light** channel values recovered temporarily
from premultiplied backdrop/source RGB, with backdrop alpha `ab` and effective
source alpha `as = source_alpha * opacity`. Working images remain premultiplied;
there is no sRGB conversion or per-mode transfer choice. With `B(b,s)` below:

`out_rgb = source_rgb * opacity * (1-ab) + backdrop_rgb * (1-as) + as*ab*B`

`out_alpha = as + ab*(1-as)`.

Zero source alpha bypasses division. Zero backdrop alpha uses ordinary over.
Clamp roundoff to `0..out_alpha`. The following table defines each component;
nonseparable functions operate on all three linear components together.

| Mode | B(b,s) |
|---|---|
| normal | s |
| darken | min(b,s) |
| multiply | b*s |
| color-burn | b=1: 1; s=0: 0; otherwise 1-min(1,(1-b)/s) |
| lighten | max(b,s) |
| screen | b+s-b*s |
| plus-lighter | min(1,b+s) |
| color-dodge | b=0: 0; s=1: 1; otherwise min(1,b/(1-s)) |
| overlay | b<=0.5: 2*b*s; otherwise 1-2*(1-b)*(1-s) |
| soft-light | s<=0.5: b-(1-2*s)*b*(1-b); otherwise b+(2*s-1)*(D(b)-b) |
| hard-light | s<=0.5: 2*b*s; otherwise 1-2*(1-b)*(1-s) |
| difference | abs(b-s) |
| exclusion | b+s-2*b*s |
| hue | SetLum(SetSat(s,Sat(b)),Lum(b)) |
| saturation | SetLum(SetSat(b,Sat(s)),Lum(b)) |
| color | SetLum(s,Lum(b)) |
| luminosity | SetLum(b,Lum(s)) |

`D(b)=((16*b-12)*b+4)*b` for b<=0.25, otherwise sqrt(b).
`Lum(c)=0.3*r+0.59*g+0.11*b` is the nonseparable blend convention,
not BT.709 colorimetric luminance. `Sat(c)=max(c)-min(c)`.
`SetSat(c,t)=(c-min(c))*t/(max(c)-min(c))`, or zero for range <=1e-6.
`SetLum(c,t)` adds `t-Lum(c)` to every component, then clips around its
luminosity: for original minimum n<0 use `l+(c-l)*l/max(l-n,1e-6)`;
for original maximum x>1 use `l+(c-l)*(1-l)/max(x-l,1e-6)`.
Plus-lighter here is the registry's saturated additive **blend function** with
source-over alpha, rather than the separate Porter-Duff additive operator.
These are the [W3C blend formulas](https://www.w3.org/TR/compositing-1/#blending),
evaluated in this editor's established linear space.

`tests/fixtures/blend-linear.json` pins a 4x4 working image pair and independent
f64 expected pixels for all 17 modes at opacity 0/0.5/1. CPU tolerance is 3e-6
absolute per working channel. The desktop WGSL path uses the same fixtures and
additional mixed-alpha grids at 1x/2x, with maximum absolute error 2e-5.
`tests/it/blend.rs` checks mask/opacity order and preview identity. Its WebM/VP9
encoded edge and patch oracle compares final decoded RGBA to opaque BT.709 output:
maximum channel difference 20/255, RGBA MSE 8 and flat-patch channel difference
6/255, allowing 4:2:0 chroma edge
reconstruction. Working-space checks retain the tighter float tolerance.
The ignored `blend_frame_benchmark` measures 640x360, ten warm frames per mode.


`Rect::validate_crop` exposes the compositor's existing source-normalized rectangle
admission for reusable desktop crop controls; preview/export keep the same bounds
and geometry validation. It adds no alternate crop renderer.
