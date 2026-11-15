# GPU effects

Interface: `GpuEffects`.

`GpuEffects` implements render's device-free effect interface on the eframe-owned wgpu
device and queue. Native startup attaches it to each preview player; headless players
use CPU. Shader entry names and ordered separable passes are data. Bindings expose the
current run and immutable instance input (binding 6). The shader set reads
track-below only for blend and the instance input only where render's table says
(`ReadImage::Instance`); only the resident blend reads coverage.
Gamma and Invert read only the current run. Their ordered uniforms are `[gamma]`
and `[channel, blend]`. They share the CPU reference's straight linear to encoded
sRGB conversion, operation and decoded premultiplication. Invert's Alpha choice
changes coverage and re-premultiplies the original straight colour.
Black & White, Channel Mixer and Two-Tone also read only the current run and
preserve alpha. Black & White has no uniforms and computes linear Rec.709 luma.
Channel Mixer takes three encoded-sRGB rows of `[r,g,b,constant]`, clamped before
decoding. Two-Tone takes encoded black RGB, white RGB and amount; it maps encoded
linear luma, mixes in encoded space and decodes once.
Levels takes `[input black, input white, output black, output white, exponent]`;
its encoded remap floors only the input span, retaining equal/reversed bounds.
Strobe takes encoded RGB and original-picture fraction, decodes the colour once
and mixes in linear light. Both read only current input and preserve alpha.
Strobe time gating stays entirely in the existing effect evaluator, including
idle no-pass phases. Reduce noise is a Sample entry reading only the current run (5x5 bilateral, uniforms `[spatial sigma, range sigma]`). Person matte is a Sample entry that reads the instance input as data: the 192x192 matte plane in the alpha of its first 36,864 texels, sampled bilinearly through the six-value affine map `[a, b, c, d, e, f]` and multiplied into the current run ([person matte](../../../../../../crates/render/src/scene/correction/README.md#person-matte)). Keep Colour, Swap Colour, Posterise and Threshold have no entry and stay on the render CPU path: their hard colour and luma thresholds flip on the device's transfer roundoff.

The 264 spatial and 265 whole-image entries (Highlights, Alpha Plane, Colour Key, Luma Key, Magnifier, the generators, glows, shadows, Paint Strokes, Torn Edges, Find Edges, Pixelate, Detail Boost, Lens Blur, Wrap Shift, Streak Blur, Edge Lines and Colour Relief) mirror their render references line by line with the same uniform layout. Shared helpers reproduce `image_ops::sample` (clamp, wrap and transparent edges, the 0..1 clamp; a clamp-edge coordinate is first limited to `[-1, extent]` and a transparent one beyond its footprint is zero, which leaves the result unchanged and keeps the integer conversion in range), the sample pass exit clamp, `f32::round`, `powf` at zero and `powi`'s square and multiply. The noise entries (Paint Strokes, Torn Edges, Light Leaks) hash in u64 like the reference, as `(low, high)` u32 pairs with 16-bit partial products, so cell hashes are exact. Lens Blur runs only the direct gather: a disc the CPU reduces to a whole-image halving (`render::effects::shader_computes` false) is `Unsupported` and the run falls back to the CPU. Point parameters arrive evaluated in output pixels from render's evaluators.

Parity table: `heavy_gpu_matches_cpu_working_pixels_at_both_scales` runs every pass of each 264/265 effect with a shader entry at its defaults and two non-default rows on the 64x36 and 128x72 fixtures, on the CPU and the adapter from the same current image, prints `entry-parity ... max_abs=` per pass and fails for any entry over 2e-5. `heavy_pixel_effects_native_1080p_measurement` prints `measurement-effect kind= cpu_ms= gpu_ms=` per effect at 1080p preview quality (CPU on up to eight workers; `gpu` is the production run split). Both run on the host adapter with:

`cargo xtask build -- cargo test --release -p vibecut --test it -- effects::heavy_gpu_matches_cpu_working_pixels_at_both_scales effects::heavy_pixel_effects_native_1080p_measurement --include-ignored --nocapture`

An entry ships only when every row is within 2e-5 on the host adapter; one that is not has no WGSL entry (`Wgsl::Absent`) and stays on the CPU. Measured 2026-11-15 on Intel UHD Graphics 770 (Vulkan, release): the 25 shipped entries are within 2e-5 (largest: `roughen_edges` 1.13e-5, `magnify` 6.6e-6, `rgb_split` 4.9e-6; Sparkle `glint_rays` 7.2e-7, Long Shadow 6.0e-8, Pixelate 1.9e-6). Removed for their measured error, all from trig, division or perspective coordinate roundoff moving a bilinear tap: `twirl` 1.41e-3, `wave_warp` 6.34e-4, `camera_shake` 4.71e-4, `turbulent_displace` 1.43e-4, `spherize` 3.27e-5, `corner_pin` 3.05e-5, `basic_3d` 3.05e-5, `lens_distortion` 2.62e-5.

The five slowest effects at 1080p preview (median of three after one warm-up, CPU on eight workers vs the production split, ms): Sparkle (`glint`) 416.2 vs 93.8, Paint Strokes (`brush_strokes`) 242.6 vs 68.5, `echo_glow` 139.5 vs 163.7, `volumetric_rays` 138.0 vs 67.8, `directional_blur` 109.3 vs 56.2. Each GPU call still uploads and reads back the full frame, so single-pass effects cost about 37-45 ms on either side on this adapter; `echo_glow` (128 passes) is slower on the device.
All supplied input geometry is validated before allocation, including unused inputs.
Unused read-only bindings follow the current ping-pong buffer; the writable output
and any required auxiliary snapshot remain distinct. Blur does no colour or alpha conversion.

`frame` acquires one nonblocking adapter permit before any scene source/output work.
Contention chooses CPU for that entire frame. Standalone calls use the same scoped
implementation. Each scope fixes its first geometry and exact borrowed hooks object.
No device or GPU dependency crosses the render interface.

Export attachment follows the [export dialog policy](../../../editor/dialogs/export/README.md).

Mapped uploads make all staging explicit: work A/B, readback and upload-current cost
4I; Sample/blend add a distinct auxiliary/upload pair for 6I. Non-Sample calls drop
the auxiliary pair. Metadata has distinct slots per pass, charged at 1024 bytes/pass
(up to 512), with at most 49 values each; actual fixed scope records are charged
separately within 4096 bytes. These bounds exclude native driver/pipeline memory and
do not guarantee export admission.

Device geometry support and [aggregate byte admission](../../../../../../crates/engine/src/runtime/README.md#background-jobs-and-byte-policy)
are separate: supported dimensions can still refuse once caller/result images,
decoding and encoding consume the shared budget.

`composite_layers` admits exactly two layers: an opaque Normal backdrop with no effects or coverage, and one Normal layer with any opacity whose effect instances pass render's `effects::resident` (gaussian, grade, chroma key and mirror only) and whose optional coverage is prepared on the CPU. It uploads the layer, the backdrop as the auxiliary image and the coverage, runs every instance's passes in order in one submission, then the `blend` entry with weights `[opacity, mode, coverage flag]`: a set flag scales the source by coverage alpha first. An instance with a mirror pass first copies the evolving image into a device-only checkpoint (1I) that its sample passes read, so a sample never sees another instance's input. That is one upload per image, one dispatch per pass plus the blend, one submission and one readback, charged as the 6I plan above plus 1I for the checkpoint and 2I for the coverage pair, every lease taken before any buffer exists; other plans (three layers, another mode, any other entry) answer `Unsupported` before device work and render's [`composite_layers`](../../../../../../crates/render/src/scene/README.md#effects-and-desktop-adapter) recomputes them on the CPU, as it does for a runtime loss. `GpuEffects::work` reports cumulative uploads, dispatches, submissions and completed readbacks. Native Metal and Vulkan runs are required before widening this case.

Completed scratch reuses buffer allocations only within its frame scope, not image
contents. Every apply/composite/frame still packs and uploads its inputs, submits and waits
for native work, reads the output back and constructs a separately charged CPU Image.
CPU runs, masks, transforms, transitions and the next decode release idle capacity.
Submitted completion owns
buffers, leases and the permit through timeout/unwind; a pending submission cannot
be recycled. Completion is serviced by native polling, with no detached poller. A
30-second wait and total one-second mapping receipt ceiling return Unsupported;
Budget, Cancelled and Invalid propagate without publishing a backdrop. A driver
that never confirms completion retains bounded charged quarantine. No unsafe casts.
