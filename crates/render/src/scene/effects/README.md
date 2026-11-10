# Effect passes

`effects.rs` owns the device-free pass model and both CPU references. An evaluator turns one effect instance's parameters at item-local time into an ordered pass list: a normalized separable convolution, a pixel-local op with typed uniforms, or a sample op that computes one output pixel from its coordinate, the flat uniforms and two readable images (`current`, the previous pass's output, and `input`, the image the instance started with). Sample ops read exact texels or bilinear samples with clamp, mirror, wrap or transparent edges on either image, so each has a one-to-one shader entry. `CpuEffects` runs the list row by row with cancellation and reservation, and clamps every sample result to the premultiplied working-image contract once, at the pass exit. An unknown entry is `Unsupported`; `supports(entry_point)` reports which entries an adapter can run, so the renderer splits a list into consecutive device and CPU runs.

`EffectInputs.input` is required and immutable across every run and Unsupported retry of one instance; the current image is passed separately. All input dimensions are validated before pixel work, and retaining the instance image uses a hook-reserved working copy. A successive instance starts with the previous instance's final output.

`apply_effect` is the single execution entry for scene instances and thumbnail jobs.
It owns one instance input, borrows the unchanged backdrop, admits geometry/passes
before copying, and splits consecutive adapter-supported and CPU runs. Unsupported
from the adapter alone retries that run on CPU with the same immutable input;
budget, cancellation, invalid-data and other errors propagate. Empty pass lists
return exact working pixels through the same accounted ownership. Copies reserve
through the caller's hooks before allocation.

The registry also names an optional content-time remap per kind. `ContentTime` receives an `EffectFrame` resolved at real item-local time and a separate evolving content coordinate, both in ticks, and returns `Result<Tick>` with typed parameter/time failures. Each remap composes the preceding content coordinate while every frame and keyframe lookup keeps the original real clock. `content_time` applies the enabled effects' remaps in stack order before the item's content is fetched: only the media, text or sticker frame is held at the returned item-local time (clamped into the item's range), while transform, crop, opacity and every parameter keep the real time, and audio is unaffected. Without a remap the time is unchanged, so transition handles outside an item's extent keep sampling exactly. `EffectFrame.fine` carries the renderer's preview flag; a coarse evaluation may lower sample counts, iterations or scales only, never geometry, colour mapping or which pixels are affected.

`image_ops.rs` owns the shared primitives the ports use: an edge-aware bilinear sample and a gaussian blur at a given sigma over the same kernel builder, plus the box downsample and bilinear upsample pair. Sampling uses texel-index coordinates: integers address exact texels. Image-edge geometry places pixel centres at `x+0.5` / `y+0.5` and subtracts 0.5 exactly once after mapping, before lookup (mirror and upsample alike). Each reserves its transients through the hooks and checks cancellation per row chunk.

The desktop adapter keeps one pipeline per shader entry and binds the instance input as the second readable image; it answers `Unsupported` for entries it has no pipeline for. CPU/GPU parity uses the existing 2e-5 working-channel tolerance.

Mirror admits centres only within the closed image-edge rectangle `[0,width] x [0,height]` and unit normals with components in `[-1,1]`, squared length within eight f32 epsilons of one. It refuses non-unit, near-zero, nonfinite and out-of-frame values as `Unsupported` before allocation, on CPU and GPU through the same admission function. With Image's area cap, reflection remains finite and inside the shader's signed index range; the sampler also checks neighbour arithmetic and returns transparent for nonfinite coordinates. No host-dependent bounds are used.

Clamp repeats a border texel, Mirror reflects around the border texel, Wrap repeats across the opposite edge, and Transparent uses zero outside every axis, including singleton axes. A half-outside bilinear footprint gets half coverage; it never repeats a singleton texel in Transparent mode.
