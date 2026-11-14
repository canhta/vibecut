# Footage corrections

Interface: `stabilize`, `matte`, `brush`, `flicker`.

What render applies inside one item's own picture from derived data the caller resolved from the
engine's [footage correction](../../../../engine/src/analysis/correction/README.md) cache. Render
reads no cache and opens no media for either; an item with its switch on and no entry in
`RenderSettings` renders without that correction and never fails the frame, and the engine's
export admission refuses instead. Stabilization: [scene](../README.md#stabilization).

## Flicker

`FlickerTrack` holds one source's brightness per frame: the mean linear luma (Rec. 709 weights on
the decoded frame's sRGB values taken to linear light), a shot-start flag and the frame time, in
strictly increasing source time (at most 262,144 frames, 13 bytes each). `encode`/`decode` are
deterministic versioned little-endian bytes (`VCFL` v1: frame count, the i64 times, the f32 means,
one flag byte per frame); `decode` refuses a bad length, order, a mean outside `0..=1`, a flag other
than 0 or 1 and a flag on the first frame. `RECIPE` names this storage and the application; the
engine puts it in the cache key.

The gain of a frame is derived from the track when it is built, so the stored bytes carry no level
(the reference's `Remove flicker` toggle shows none). The reference is the Hann-weighted mean
(`0.5 (1 + cos(pi k / (h + 1)))`, `k` in `-h..=h`) of the means in a window of one second of
frames, `W = round(fps)` made odd (31 at 30 fps, `h = (W - 1) / 2`), clipped to the frame's own
shot, so a cut resets it and a frame at a shot edge takes the one-sided window; the gain is that
reference over the frame's own mean, clamped to 0.5..2 (our choice, unmeasured). A dark frame
(mean under 1e-4) takes gain 1. `fps` is read from the track's own frame times. A long window
flattens genuine slow changes and a short one leaves slow flicker: the study found a 1 s Hann window
removes flicker down to 1.7 Hz and keeps a 6 s fade; flicker below about 1 Hz cannot be told from a
real brightness change.

`RenderSettings.flicker` maps an item with `VideoParams.remove_flicker` on to its track. The item's
layer takes one `gain` pass of the frame covering the source time, the same factor on red, green and
blue, before noise reduction and every effect (the gain pass is the linear-light `straight * gain`
stage in [effects](../effects/README.md), native on the desktop adapter and a CPU reference
elsewhere, so preview and export take the one path). Trims and splits read the same track and the
same frame gain, whatever part of the source an item shows. An item with the switch on and no stored
track renders uncorrected and never fails the frame; the engine's export admission refuses instead,
and the stream-copy export path refuses an item with the switch on.

Evidence (`tests/it/flicker.rs`, and the engine's `footage_correction.rs` on lossless clips): the
bounds recorded by the footage clean-up study for a 1 s window: mains flicker (the 10.09 Hz alias
and the 1.7 Hz beat, 8 % deep) leaves interior RMS at most 0.6 % and maximum at most 2 %; a fade
under flicker is held to 0.6 % RMS; a hard cut with flicker to 0.5 % RMS with the reset, where
without it the error is over 5 %. The study measured 0.27/1.32, 0.39/1.46, 0.37 and 0.24 on its
film clip. The fixtures here model the measured series (and in the engine, 160x90 clips whose linear
light is modulated), not that clip, and their brightness drift is a test input: a shot whose
brightness moves fast next to a cut is biased by the one-sided window (0.52 % RMS and 2.6 % maximum
at a drift of 17 to 20 %/s on both sides of a cut, against the recorded 0.5 % RMS), so the cut
fixtures drift at most 7 %/s. The
study did not record the drift of its own clip. Not measured: rolling-shutter banding (a frame gain
cannot remove per-row flicker), real footage, flicker below 1.7 Hz, and fast real brightness
changes such as a light switched on (partly flattened).

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

`brush::refine` applies `VideoParams.matte_strokes` over that coverage plane, after the feather and before the plane is sampled, so the CPU reference and the shader entry read the same refined plane and parity needs no row of its own. A stroke refines only the matte frame whose index equals `covering(stroke.at)` (strokes are keyed by source time, so trim, retime and split carry them). Points are decoded-frame units, the units of the stretched plane; the tip is a circle in frame pixels of radius `radius * min(width, height)`, antialiased over one plane cell (the larger cell axis in pixels). Add moves a cell to `c + (1 - c) w` and remove to `c (1 - w)` with `w` the tip weight, strokes in stored order, each stroke's own tip weight being the maximum over its segments, so overlapping segments of one stroke never apply twice. The brush is not part of the cached matte: the recipe and cache key do not change. A stabilized clip's strokes are placed on the unwarped frame, as the stored plane is. `Renderer::frame_point` (`scene/viewport.rs`) gives the player the same map backwards, from a screen point to the decoded-frame point under crop, fit and transform.

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
for the model output; the model card's own mIoU is 0.9164. `matte_strokes_add_and_remove_over_the_computed_matte_on_their_own_frame_only` renders add and remove strokes over a halves matte (clip inside the add tip, canvas inside the remove tip, nothing changes outside, later stroke wins, same bytes twice, a stroke on another matte frame changes nothing) and `a_screen_point_maps_back_to_the_decoded_frame_through_crop_and_fit` checks the screen-to-frame map.
