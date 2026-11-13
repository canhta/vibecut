# Footage corrections

Interface: `stabilize`, `matte`.

What render applies inside one item's own picture from derived data the caller resolved from the
engine's [footage correction](../../../../engine/src/analysis/correction/README.md) cache. Render
reads no cache and opens no media for either; an item with its switch on and no entry in
`RenderSettings` renders without that correction and never fails the frame, and the engine's
export admission refuses instead. Stabilization: [scene](../README.md#stabilization).

## Person matte

`MatteTrack` holds one source's person plane per frame: 192x192 cells, one byte each
(255 certainly a person), in strictly increasing source time, each plane PackBits-compressed
(a header below 128 copies `n + 1` bytes, from 128 repeats one byte `n - 126` times; every run of
two or more equal bytes becomes a run, so a plane always packs to the same bytes). `MatteBuilder`
admits at most 65,536 frames and 128 MiB encoded. `encode`/`decode` are deterministic,
versioned little-endian bytes (`VCPM` v1: frame count, then per frame an i64 time and the u32
compressed length, then the planes); `decode` refuses a bad length, order or a plane stream that
does not expand to exactly one plane, without expanding it. `RECIPE` names this storage and the
application; the engine puts it in the cache key.

`RenderSettings.person_mattes` maps an item with `VideoParams.background_removal` on to its
track. For the frame covering the source time, `feathered(plane, feather)` gives the coverage
plane: the stored bytes / 255 exactly at feather 0 (the model's own edge), and above 0 two box
passes of radius `ceil(feather / 100 * 8)` cells per axis with clamped edges (the 0..=100 scale,
radius and box shape are our choices; the reference shows no feather).

The matte scales the item's layer before noise reduction and every effect, so effects, masks,
transform, opacity and blend all see the cut-out picture and whatever is below shows through.
`plane_map` composes the layer-pixel to plane-cell map from `draw`'s placement (crop, fit and
destination) and, for a stabilized item, the frame's warp (`Warp::frame_point`), so the matte stays
on the subject of a steadied clip. The plane spans the whole decoded frame stretched, so a matted
frame is never decoded as a horizontal window. Coverage is a bilinear sample of the plane with
edge cells clamped, in f32, and every premultiplied channel is multiplied by it.

`apply` runs that as the `person_matte` sample pass (six uniforms: the affine map). The plane
rides in the pass's instance input, packed as grey texels `0..36864` of a layer-sized image
(`plane_input`), so a layer needs at least 36,864 pixels; the desktop adapter runs the shader
entry and any other adapter, or an Unsupported answer, runs the CPU reference in `OPS`. A smaller
layer (a thumbnail) takes the same arithmetic in place. On the CPU the pass holds at most four
layer-sized images (the layer, the packed plane and the reference's two working buffers, as any
effect instance does), all charged through the frame hooks; the 4K test asserts that bound and
that a cancelled frame stops. The GPU parity row lives in the desktop
`gpu_matches_cpu_working_pixels_at_both_scales` (tolerance 1e-5, identity, scaled and rotated
maps, feather 0, 40 and 100). Preview and export take the one path, so they agree, and the export
stream-copy path refuses an item with removal on.

Evidence (`tests/it/matte.rs`): encoding round trip and determinism; feather 0 is the stored plane
and the identity at the plane's own size; a feather widens the edge band; IoU of
`coverage >= 0.5` against annotated head-and-shoulders fixtures at 640x360, 360x640, 333x333,
1280x720 and 160x90 meets the recorded floors 0.95 (feather 0 and 25) and 0.88 (feather 100),
printed as `MATTE_IOU`; a matted scene keeps the clip where the plane is 255 and shows the canvas
where it is 0, preview equals export, and a missing matte renders unmatted. The fixtures stand in
for the model output; the model card's own mIoU is 0.9164.
