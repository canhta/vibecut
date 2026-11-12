# Effect passes

## Imported cubes

`render::lut` owns strict bounded 3D cube parsing, canonical serialization, resource resolution and trilinear evaluation. Only UTF-8 `.cube` 3D tables with 2..65 samples per axis are admitted; 1D/combined shapers and other directives refuse. Original input is capped at 24 MiB, canonical input at 16 MiB, headers at 16 KiB and lines at 1 KiB. Quoted TITLE text admits at most 256 UTF-8 bytes after NFC; its normalization scratch is reserved before allocation. Rows are finite RGB in 0..1 with exact N³ count, red fastest; domains default to 0..1 and require each finite 0≤min<max≤1 with span at least 1e-6. No camera-log/HDR or gamut inference occurs.

The grade stages run first, in the order basic, lift, gamma, gain, RGB curves and luma curve, followed by the LUT. LUT evaluation unpremultiplies once, encodes sRGB, clamps to the domain, interpolates eight corners with f64 weights, decodes to linear, blends wet/dry in linear light and re-premultiplies with unchanged alpha. None, disabled, zero strength and detected default-domain identity preserve exact working pixels; None/disabled/zero strength need no resource IO. Alpha zero produces zero RGB. Imported passes are explicit CPU `Lut3d` passes; the existing run splitter handles supported device neighbours and retries only Unsupported.

Active tables resolve before decoder/device work into one closed immutable per-frame snapshot. The hash covers the same bounded bytes parsed. Input, actual sample capacity, Arc overhead and the resource map are reserved through the exact frame hooks; pass readers retain the table lease until their last drop. No uncharged global or foreign-policy table is reused. This first implementation retains no idle tables between frames; filesystem-warm frames still read/parse and are measured separately, with no realtime claim. `InvalidLut` carries bounded line/reason, `LutUnavailable` carries only identity/reason; IO keeps its typed source and never substitutes an ungraded success.

`effects.rs` owns the device-free pass model and both CPU references. An evaluator turns one effect instance's parameters at item-local time into an ordered pass list: a normalized separable convolution, a pixel-local op with typed uniforms, or a sample op that computes one output pixel from its coordinate, the flat uniforms and two readable images (`current`, the previous pass's output, and `input`, the image the instance started with). Sample ops read exact texels or bilinear samples with clamp, mirror, wrap or transparent edges on either image, so each has a one-to-one shader entry. `CpuEffects` runs the list row by row with cancellation and reservation, and clamps every sample result to the premultiplied working-image contract once, at the pass exit. An unknown entry is `Unsupported`; `supports(entry_point)` reports which entries an adapter can run, so the renderer splits a list into consecutive device and CPU runs.

`EffectInputs.input` is required and immutable across every run and Unsupported retry of one instance; the current image is passed separately. All input dimensions are validated before pixel work, and retaining the instance image uses a hook-reserved working copy. A successive instance starts with the previous instance's final output.

`apply_effect` is the single execution entry for scene instances and thumbnail jobs.
It owns one instance input, borrows the unchanged backdrop, admits geometry/passes
before copying, and splits consecutive adapter-supported and CPU runs. Unsupported
from the adapter alone retries that run on CPU with the same immutable input;
budget, cancellation, invalid-data and other errors propagate. Empty pass lists
return exact working pixels through the same accounted ownership. Copies reserve
through the caller's hooks before allocation.

EffectFrame resolves one typed value per parameter at real item-local time. Missing
animation uses static straight sRGB bytes divided directly by255. Animated colours use
existing independent linear-light RGBA channels and their explicit base, including
zero-key overrides. Wrong kinds and nonfinite samples refuse; finite overshoot clamps.
Only sampled RGB casts to f32 at the shared sRGB transfer, then widens into the frame's
encoded f64 RGBA representation; widening does not restore precision. Alpha stays f64,
and uniforms cast encoded RGB to f32 without byte quantization. E1 colour operations
use RGB only and preserve image coverage under alpha keys; chroma-key colour stays static.

The existing per-instance plan reservation covers these bounded resolved values as
well as pass storage: the catalogue's largest kind has19 parameters and a grade
instance emits at most eight pixel passes (basic, lift, gamma, gain, three RGB curves,
luma) plus its optional LUT; Image's area bound limits admitted Blur to203
iterations/406 passes against the reserved512 pass/kernel slots. No additional image,
unreserved image copy or kernel is introduced by colour interpolation.

The pixel-local colour references unpremultiply once and re-premultiply at the exit.
Monochrome uses linear Rec.709 luma; levels, gamma, posterisation, channel mixing
and RGB inversion operate on encoded sRGB. Keep Colour compares encoded RGB
distance and greys rejected pixels with linear luma. Swap Colour either substitutes
the encoded replacement or retains input HSL lightness. Two-tone maps encoded
linear-luma between its encoded endpoints. Threshold writes linear black/white
coverage-colour while preserving alpha. Strobe gates at real item-local time and
mixes its decoded colour with the original in linear light. Other than Alpha
Invert, these operations preserve input alpha.

Lift, tone gamma and gain are linear-light straight-colour stages: `x + L*(1-x)`,
`x^(1/g)` and `g*x`, each clamped to 0..=1 and re-premultiplied with unchanged
alpha. Their uniforms are three f32 coefficients in red/green/blue order, finite
in `[-1,1]`, `[0.25,4]` and `[0,4]`. The four curve passes flatten to exactly 65
uniforms: an integer count 2..=32, then 32 ordered `x,y` pairs zero-padded;
validation refuses a malformed count, off-grid or unordered input, moved
endpoints, nonfinite values and nonzero padding. Interpolation is piecewise
linear on the authored f32 knots; no cubic or LUT approximation is used. The
desktop adapter runs lift/tone gamma/gain natively and leaves all curve entries
to the CPU until a 65-value native admission exists.

Invert's blend is the original-picture fraction. Alpha Invert blends coverage
between its complement and the original, preserving straight colour; undefined
colour at zero input alpha is black. RGB is re-premultiplied by that blended alpha,
including transparent input. Levels retains raw normalized input bounds in its
uniforms and floors only their difference in the pixel equation, so equal or
reversed authored bounds remain admissible. Independent expected working pixels
establish colour correctness; packed-byte fingerprints only detect regressions.

The registry also names an optional content-time remap per kind. `ContentTime` receives an `EffectFrame` resolved at real item-local time and a separate evolving content coordinate, both in ticks, and returns `Result<Tick>` with typed parameter/time failures. Each remap composes the preceding content coordinate while every frame and keyframe lookup keeps the original real clock. `content_time` applies the enabled effects' remaps in stack order before the item's content is fetched: only the media, text or sticker frame is held at the returned item-local time (clamped into the item's range), while transform, crop, opacity and every parameter keep the real time, and audio is unaffected. Without a remap the time is unchanged, so transition handles outside an item's extent keep sampling exactly. `EffectFrame.fine` carries the renderer's preview flag; a coarse evaluation may lower sample counts, iterations or scales only, never geometry, colour mapping or which pixels are affected.

`image_ops.rs` owns the shared primitives the ports use: an edge-aware bilinear sample and a gaussian blur at a given sigma over the same kernel builder, plus the box downsample and bilinear upsample pair. Sampling uses texel-index coordinates: integers address exact texels. Image-edge geometry places pixel centres at `x+0.5` / `y+0.5` and subtracts 0.5 exactly once after mapping, before lookup (mirror and upsample alike). Each reserves its transients through the hooks and checks cancellation per row chunk.

The desktop adapter keeps one pipeline per shader entry and binds the instance input as the second readable image; it answers `Unsupported` for entries it has no pipeline for. CPU/GPU parity tolerance is 2e-5 per working channel.

Mirror admits centres only within the closed image-edge rectangle `[0,width] x [0,height]` and unit normals with components in `[-1,1]`, squared length within eight f32 epsilons of one. It refuses non-unit, near-zero, nonfinite and out-of-frame values as `Unsupported` before allocation, on CPU and GPU through the same admission function. With Image's area cap, reflection remains finite and inside the shader's signed index range; the sampler also checks neighbour arithmetic and returns transparent for nonfinite coordinates. No host-dependent bounds are used.

Clamp repeats a border texel, Mirror reflects around the border texel, Wrap repeats across the opposite edge, and Transparent uses zero outside every axis, including singleton axes. A half-outside bilinear footprint gets half coverage; it never repeats a singleton texel in Transparent mode.

`EffectGeometry` requires Image-admitted dimensions and positive finite axis gains
(working pixels per full-resolution clip pixel). Scene resolves crop/fit before item
transform; thumbnail callers supply author geometry. `evaluate_effect` consumes it;
content remaps do not. Streak's displacement is
`(2*length*sin(direction)*gx, -2*length*cos(direction)*gy)` in working pixels.
It gathers `ceil(span)` Clamp taps, bounded to 2..64, in premultiplied RGBA,
and is identity below a .5-pixel span. Span is the displacement's Euclidean
length; equal gains give `2*length*gain` at every direction (length 2 and gain 1.5
give span 6 and six taps, including 45 degrees). Equal axis gains evaluate this
direction-independent length directly, before the tap-count ceiling, so rounding
the trigonometric components cannot add a tap. Unequal gains retain the
displacement norm; spans genuinely above an integer still take the next tap.
Fine/coarse use equal effort.
Displacement uniforms, tap fractions, offsets and absolute texel-index coordinates
use f32 at the sample-op boundary; coordinate rounding precedes bilinear lookup,
including when a tap crosses a transparent placed-content edge. Working RGBA and
sample accumulation also use f32. Unrepresentable extents are Invalid, malformed
raw uniforms Unsupported; Blur keeps its own normalization.

Edge Lines is one fixed output-pixel 3x3 Sobel over premultiplied linear luma with
clamped integer borders; geometry gains deliberately do not scale it. Its invert flag
selects `m` or `1-m`, and blend mixes the original picture back. Colour Relief samples
the current image twice at `+/-` (`cos(a)*relief*max(gx,.25)`,
`-sin(a)*relief*max(gy,.25)`) with Clamp in texel-index coordinates. Neighbour luma is
straight linear (transparent alpha zeroes it), added twice the contrast fraction to the
encoded centre RGB before the original-picture blend; the result decodes and
re-premultiplies centre alpha. Both emit no pass at their stated identities
(Edge Lines blend100; Colour Relief relief0/contrast0/blend100) and reject
unrepresentable extents before execution copies.

Sharpen convolves the current image with a gaussian of
`max(min(gx,gy),.6)` output pixels over the existing kernel builder, then a
`detail_boost` sample op adds `amount/100` times the difference between the
immutable instance input and that blurred copy, per channel, preserving input
coverage; the pass exit clamps to the working contract. Amount 0 emits no pass.
The source's single uniform scale is the equal-gain case of this admitted
smaller-axis extension; a threshold uniform (0 for Sharpen) gates each
difference against that threshold times input coverage.

Pixelate turns each authored block count into a block size
`max(axis/count,1)` output pixels and reads the immutable input at block
centres in texel-index coordinates, clamped at the picture edge. Softness
blends the nearest tile colour toward the bilinear mix of the four surrounding
tile centres; the sharp flag forces the nearest colour, so it changes the
picture only with softness above zero. One block per axis on both axes emits
no pass.

## Whole-image entries (265)

`whole_image.rs` owns the CPU references of the 26 E2 entries. Warps and generators are
one `Sample` pass; glows compose `highlights`/`sobel_edges` (Pixel or Sample), the shared
`gaussian` Convolve passes and a `glow_add` sample that reads the blurred light and the
immutable instance input. `alpha_plane` plus a convolve feeds Torn Edges' depth field.
Drop Shadow blurs first and shifts alpha in its sample; Long Shadow marches the source
alpha over a bounded tap count. Every op returns premultiplied linear RGBA and the
shared pass exit clamps it once.

Determinism: time reads only the item-local `frame.seconds` (wave phase, shake noise,
leak drift), so a given item-local time always produces the same pixels. Colour-key
distance is BT.709 chroma plus 0.15·|ΔY| on encoded sRGB; tolerance spans
`0.01 + 0.6·tolerance/255` and Edge Thin shifts it by `thin/5·0.1`. Luma Key ramps
encoded luma between cutoff and threshold, or mirrors it from white when Invert is on.

Quality: this port's preview budget is one 60 Hz frame at 1080p (16.7 ms). Entries above
it lower sample counts in preview (`fine=false`) and run full quality in export, without
changing geometry, colour mapping or affected pixels: Turbulent Displace caps noise
octaves at 3, Paint Strokes at 8 of up to 24 taps, Sparkle at 10 of 24 streak taps, Sun
Rays at 12 of 48 radial taps, Torn Edges at 3 octaves, Long Shadow at 12 of 40 march
taps, Echo Glow at 3 of 6 blurs and Bloom at one of three scales; Glow's edge-width blur
runs in fine only. The ignored `measure_whole_image_1080p` probe prints every entry's
release cost and the five slowest; those numbers stay unverified until it runs.
