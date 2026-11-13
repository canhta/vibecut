# Visual properties

Persisted visual item properties: transform, blend selection, masks, effects, video transitions, canvas bounds and text typography. Evaluation, coverage math and device work belong to render and the desktop app.

## Imported LUT records

`ProjectData.luts` retains at most 16 unique content identities with bounded NFC provenance and absolute losslessly serializable bundle directories. Model validation is structural and refuses every effect reference absent from this collection; filesystem identity belongs to the [copied-resource owner](../../../engine/src/store/README.md#imported-lut-bundles). `color_grade` has one optional static LUT reference and static strength in 0..100, default 100. A reference is a typed effect value, never a scalar or an animation channel. `EffectParamUpdate` admits a number, a tone-curve point array or the closed required `{"lut":hash-or-null}` object.


## Transform


`TransformField` owns units, defaults and finite clamps for the five numeric transform channels: position X/Y in project-canvas pixels (−100,000..=100,000, default 0), independent X/Y scale ratios (0.01..=10, default 1) and clockwise rotation in degrees (−360..=360, default 0). The command adapter refuses out-of-bounds input; animated evaluation clamps overshooting curves to these bounds.

`FlipAxis::{Horizontal, Vertical}` persists as the held Boolean `transform.flipX` / `transform.flipY` channels; absence means not flipped. A reflection reverses that local axis about the placed item centre before clockwise rotation and keeps the numeric scale magnitudes positive, so it is not a negative scale. Video, image, text, overlay and graphic items carry it; audio and caption items refuse it in whole-project validation and in the command reducer.

`ItemAlignment::{Left, HorizontalCenter, Right, Top, VerticalCenter, Bottom}` and `DistributionAxis::{Horizontal, Vertical}` are command inputs, not saved fields. Alignment translates the existing position channels so an item's transformed envelope meets a canvas edge or centre; distribution spaces three or more envelopes with equal gaps while pinning the first and last outer edges. Video, image, text, overlay and graphic items admit both; captions own their cue layout and audio has no box.

## Blend selection and opacity

`Item.blend_mode` persists as required `blendMode` using the 17 kebab-case IDs in `BlendMode::ALL`. Normal is the creation default; audio items refuse a non-normal blend.

Opacity is owned by the scalar `opacity` animation channel. A missing channel means unity.

## Crop

`Crop` is one rectangle in normalized source coordinates: `x`/`y` are the top-left corner and `width`/`height` the size, as fractions of the source frame. Every field is finite, width and height are positive, and the rectangle stays inside `0..=1`; the default is the full frame. Video and image items carry `VideoParams.crop`; absent means the full frame. Render sampling and the shorts batch path consume this one type, so it survives trim, retime, split, undo/redo and persistence as item content.

## Fit mode

`FitMode` (`fit`, `fill`, `stretch`, `none`, `manual`) is `VideoParams.fit` and decides how a video or image item's cropped source meets the canvas; `fill` is the creation default. `FitMode::extent` and `FitMode::place` are the one computation of the placement: Fit scales the source inside the canvas keeping aspect, Fill covers the canvas and crops, Stretch maps the source onto the whole canvas, None keeps the source pixel size centred, and Manual is a Stretch base that only the item's transform places. `fit_box` reads the asset's first video stream and the crop for it; without a known source size the item fills the canvas. Render evaluates the policy per frame from the current canvas, so it is never stored in the transform. Audio, text, caption and generated items have no fit mode and refuse the `fit` item patch.

## Masks

`Item.masks` is an ordered vector of at most 1,024 `Mask` values. IDs are unique within the item and follow the animation-path identifier grammar. Audio items refuse masks. Empty or whitespace-only text masks are inactive but keep their data. Modes are None/Add/Subtract/Intersect/Lighten/Darken/Difference; render owns coverage math.

Shapes: split, cinematic band, rectangle, ellipse, heart, diamond, star, text and a cubic `MaskPath`. Text masks persist content, font family, weight, italic, font size, letter spacing and line height; `MaskShape::text` defaults to bundled Inter Regular. Normalized centers are offsets from the placed item center, width/height are item fractions, rotation is degrees and uniform scale multiplies shape size. Path positions and tangent offsets follow the [MaskVertex contract](mask.rs); fill closes open paths. Feather, expansion, stroke width and text size use project-canvas pixels. Color is sRGB RGBA8 `Color`; opacity is 0..1.

`MaskPath::ellipse`, `rectangle` and `polygon` build closed editable seeds. `MaskPath::split` performs a de Casteljau split on one segment for `t` in `2^-20..=1-2^-20`; `without` removes anchors while keeping at least three. Both leave the stored anchors and tangent offsets exact, so only rendering flattens. Paths admit 4,096 vertices and bounded normalized coordinates. Text admits 4 KiB and 1..4096-pixel font sizes. `masks.ID.params.KEY` animates center_x/center_y, width/height, rotation, scale, feather, expansion, opacity and stroke_width through `Animations`, in item-local time. `Mask::at` clamps animated overshoot to the static admissible range.

## Item effects

`effect_definitions()` is the one catalog of effect kinds: each `EffectDefinition` carries a snake_case `id`, our `name`, its `EffectShelf` (the six library shelves, or `Inspector` for effects that belong to a clip inspector panel and are never listed) and its parameters. Each `EffectParamDefinition` has an `id`, a name, an `EffectParamKind`, a default `EffectValue` and an `animatable` flag. Kinds are `Float` (hard `min..=max` a typed value must satisfy, soft range for sliders), `Angle` (degrees, `-ANGLE_MAX..=ANGLE_MAX`), `Choice` (an index into named options), `Bool`, `Color`, `Point` (frame fractions, `0..=POINT_MAX`), `Lut` and `Curve`. Float, angle and explicitly declared colour parameters may be `animatable`. `Item.effects` is an ordered stack of at most 64 `EffectInstance`s. An instance is `{id, kind, enabled, params}`: `kind` names its definition and `params` maps every parameter id of that definition (no more, no fewer) to a typed `EffectValue` (`Scalar`, `Choice`, `Bool`, `Color`, `Point`, `Lut`, `Curve`) that its kind accepts, finite and in range. Instance IDs are item-local and every one must follow the [animation-path identifier grammar](../animation/README.md#parameters) (1..=64 ASCII letters, digits, underscore or hyphen), whether or not its parameters animate; duplicates refuse. `enabled` bypasses an instance. Audio items refuse visual effects. `EffectDefinition::instance` builds an instance at the defaults and `EffectInstance::with` replaces one checked value.

Built-in kinds: `blur` has `intensity` 0–100 (default 15, animatable). `color_grade` has twelve Basic controls (`saturation`, `temperature`, `hue`, `brightness`, `contrast`, `shine`, `highlight`, `shadow`, `sharpness`, `vignette`, `fade`, `grain`, in `GRADE_BASIC` order, each -50..=50 with 0 identity and animatable, `GRADE_MIN`/`GRADE_MAX`), eight HSL bands (`HSL_BANDS`, each with `hsl_<band>_hue`, `_saturation` and `_lightness` on the same range and animation), three animatable lift controls (`lift_r`, `lift_g`, `lift_b`, -100..=100 with 0 identity, `LIFT_MIN`/`LIFT_MAX`), three animatable gamma controls (`gamma_r`, `gamma_g`, `gamma_b`, 25..=400 with 100 identity, `GAMMA_MIN`/`GAMMA_MAX`), three animatable gain controls (`gain_r`, `gain_g`, `gain_b`, 0..=400 with 100 identity, `GAIN_MIN`/`GAIN_MAX`) and four static curves (`curve_red`, `curve_green`, `curve_blue`, `curve_luma`). The ranges and defaults are new decisions, not measured parity; only the twelve Basic ranges and 0 defaults are measured. `chroma_key` has an opaque `key_color` (default green, static) plus `intensity` and `shadow`, each 0–100 with defaults 40 and 0 measured from the reference and animatable; intensity 0 keys nothing. An `EffectValue::Color` parameter persists as an sRGB `Color` RGBA byte value; the engine's packed `0xRRGGBB` command scalar (`PACKED_COLOR_MAX`) is converted by `packed_color`. Points carry two components. Parameters animate at `effects.<instance-id>.params.<id>` for every animatable parameter of the definition (`EffectDefinition::animated_ids`). `EffectValue::update` is the scalar/curve/LUT projection the `effects.update_param` command carries. Effect parameter edits and their undo step belong to the [engine command reference](../../../../docs/product/agent-control/commands.md).

`EffectInstance::set_scalar` validates a registered parameter id and value before replacing the parameter; unknown ids and invalid values keep the previous parameter. Definition-specific animation admission belongs to this owner; [whole-project validation](../model/README.md#project) calls it for each instance.

`ToneCurve` is one piecewise-linear, monotone map: 2..=32 finite points, fixed `[0,0]`/`[1,1]` endpoints, strictly increasing `x` on the exact 1/4096 grid with at least one grid step between neighbours, and nondecreasing `y`. Deserialization refuses a 33rd point before retaining it. Nothing is sorted, deduplicated or repaired. Curve parameters are static topology; an addressed `effects.<instance-id>.params.curve_*` animation path refuses validation instead of being ignored. `EffectParamUpdate::Curve` carries the same validating point array.

Pixel-local monochrome, channel mixing, selective colour, threshold, gamma,
inversion, levels, posterisation, strobe and two-tone definitions use the same typed
parameter admission. Their hard/soft ranges, insertion defaults and scalar animation
flags live in [the catalogue](effects/catalog.rs). Keep Colour's colour, Swap Colour's
target/replacement, Two-tone's endpoints and Strobe's colour animate through the existing
linear-light RGBA composite in [animation](../animation/README.md#parameters). A present
composite owns its base even with no keys; missing data keeps the static sRGB bytes.
Definition-specific admission requires actual Color composites for these colours and
scalar Value channels for numeric controls; held text colours are not effect animation.
Whole-project animation validation checks component values/keys and shared budgets.
Chroma-key colour, choice, boolean and Point values remain static. No path/component
shape or serialized meaning changes.

Streak Blur animates direction (degrees) and length (full-resolution clip pixels).
[The catalogue](effects/catalog.rs) owns ranges/defaults/shelf; render owns crop/fit axis conversion before transform.

Edge Lines is Stylize with an invert flag and an animatable 0–100 blend percent.
Colour Relief is Stylize with an animatable direction (degrees), relief (full-resolution
clip pixels, 0–10), contrast (percent, 0–500) and blend percent. The static invert flag
has no animation; every numeric control uses the existing scalar/enum shape.

Sharpen is Blur & Detail with one animatable amount (percent gain, hard 0–4000,
slider 0–100, default 0 = identity).

Pixelate is Stylize with animatable horizontal and vertical block counts (hard 1–4000,
slider 1–200, default 10), an animatable softness percent and a static sharp flag.
The counts are blocks per axis, not pixel sizes; one block per axis on both axes is
identity.

The 26 whole-image entries sit on the Distort, Glow & Light, Stylize and Overlay & Key
shelves: twirl, spherize, magnify, wave_warp, lens_distortion, turbulent_displace,
corner_pin and basic_3d on Distort; echo_glow, edge_glow, glint, wonder_glow,
volumetric_rays, light_leaks, lens_flare and rgb_split on Glow & Light; brush_strokes,
roughen_edges, camera_shake and posterize_time on Stylize; drop_shadow, long_shadow,
gradient, four_color_gradient, color_key and luma_key on Overlay & Key. Ranges, defaults
and animation flags live in [the catalogue](effects/catalog.rs). Author-level lengths
and radii are full-resolution clip pixels; render converts them through the item's
axis gains before sampling. `corner_pin`, the Magnifier/Sphere/Twirl centres, the two
Gradients' points and the Sun Rays source are `Point` values in frame fractions. Lens
Flare's authored parameters are `center`, `brightness` (0–300, default 100) and `blend`
(0–100, default 0), read from the source definition; the renderer's extra legacy
fields are not authored. `posterize_time.rate` is the one content-time remap: it holds
the item's media at `floor(t·rate)/rate` while parameters keep real item-local time.

Point parameters are defined, validated, persisted and duplicated with the instance,
but no Properties control or edit command addresses them yet; that is the deferred
Point contract from the earlier E1 port, not a second representation.

The `test-kinds` cargo feature appends one catalog-only `hold_time` kind on `Inspector`; it is off in every production build.

## Filters

A filter is a `FilterDefinition` in `filter_definitions()`: an id, our name, a `FilterShelf` (Featured, Movies, Retro, Night, Mono, Life or Tools) and the `color_grade` scalars and tone curves it changes from neutral. `FilterDefinition::instance(id)` builds the validated colour-grade instance at full strength. The catalogue holds 20 looks (the 8 procedural looks and 4 creative presets of the reference implementation, renamed, plus 8 authored ones) and the 4 Tools grades, which are not counted among the looks. Looks are procedural grades over the basic, lift, gain, HSL and curve controls; no third-party LUT or picture is used, and no generated LUT is built yet. The values are new decisions, not measured parity.

A filter clip is not a new item kind: it is an effect item (`ItemParams::Effect` on an Effect track, the adjustment layer) whose stack holds that one instance, and its Intensity is the item's `opacity` channel (0..=1 shown as 0..=100, default 100). Intensity 0 leaves the picture below unchanged. The engine's `filters.insert` builds that item in one edit. `is_filter_clip(item)` recognises it by the `FILTER_INSTANCE_ID` instance on an effect item, and `filter_look(item)` names the catalogued look whose instance it still equals; a filter clip refuses user-added effects.

## Video transitions

`Timeline.transitions` owns `Transition` entities with a project-wide unique `TransitionId`, track, outgoing and incoming item IDs, kind, output start, duration and `TransitionAlignment` (start, center, end, custom). The two items must be adjacent and touching on the same video track and contain video or generated solid overlays. `TRANSITION_DEFINITIONS` is the admitted catalog; each entry carries the kind id, our name, family, browser shelf and typed parameter definitions. Entries fill the Basic, Overlay, Light, Blur, Wipes, Glitch & Distort, Slides, 3D and Camera shelves. Unknown kinds return `UnsupportedTransition` with a bounded kind name, including during file ingestion. Field-wipe feather admits 0..=2000 project pixels independently of the 0..500 soft slider range; Spin Smear's blur angle is a Float in degrees, 0..=180 (default 40), rather than the multi-turn Angle type.

`Transition.params` stores only overrides of the catalogue defaults, keyed by parameter id; `Transition::value` resolves override else default. A definition declares each parameter as a float (min, max, default, unit), choice, bool, sRGB colour, point (fractions of the frame, 0..=1), angle (degrees, at most four turns either way) or seed (0..=9999). `Transition::set_param` and validation refuse an unknown id, a value of another type, a non-finite number or a value outside the bounds, and validation refuses the whole project rather than repairing it. Parameters are fixed per transition and never animate.

A transition keeps earlier outgoing IDs in `from_pieces` and later incoming IDs in `to_pieces`, both in timeline order. Each side must form a contiguous chain of video or overlay items on its track. `from`/`to` stay adjacent at the original cut. `contributors(at)` resolves the piece on each side, using the first or last piece's handles beyond the side extent. A split preserves output start, duration and alignment and never renormalizes progress.

`Transition::bounds` intersects the two complete side extents with real source handles: the incoming source in-point bounds the time before the cut, and the outgoing source's remaining known duration bounds the time after it. Unknown outgoing duration permits no extension after the cut. Generated overlays have no handle constraint. `set_duration` clamps positive ticks to those bounds, following alignment; an odd centered duration puts floor(duration/2) before the cut. Custom duration changes keep the span's centre, clamped to stay over the cut. Zero duration or handles and invalid spans refuse. Only one transition may name a cut, and spans on one track cannot overlap. Validation never repairs persisted data. Edit-time revalidation belongs to [engine](../../../engine/src/session/edit/README.md#video-transitions).

`Timeline::rendered_items(from, to)` lists the visible items that draw in a range: those overlapping it plus both clips of every overlapping transition, which render from their handles outside their own extent. Range consumers (export copy and admission, HDR detection, control previews) use it instead of filtering item extents.

## Canvas bounds

Names, notes and text use captions' NFC text owner. Canvas frames have exact positive integer tick durations. Canvas area is at most 2^26 pixels; output audio admits 8–192 kHz. Audio/video source volume and master volume admit -60..20 dB on a 0.01 dB grid. Loudness targets admit -50..-5 LUFS and -20..0 dBTP on the same grid; measured gains admit -60..60 dB for at most 4,096 sorted clips. Font sizes admit 1..4096 pixels.

`Canvas.preset` is the chosen ratio preset (`original`, `16:9`, `4:3`, `2:1`, `9:16`, `1:1`, `3:4`); `None` means a custom size. Fixed presets use a 1920 long edge: 16:9 = 1920x1080, 4:3 = 1440x1080, 2:1 = 1920x960, 9:16 = 1080x1920, 1:1 = 1080x1080, 3:4 = 1080x1440. `original` resolves the stored `original_canvas_size`. The stored width/height stay authoritative.

`CanvasSize::rounded_even` gives the nearest even size on each odd axis (ties up) so encoders can open the frame; an axis that would overflow returns `None`. The edit boundary rejects a rounded size outside the area bound. The first-media capture of `original_canvas_size` applies the same rounding. `Canvas.last_custom_size` is the last committed custom width/height.

## Text typography

Text items carry a 1..256-byte font-family name, line `align` (left, centre, right; default centre), `decoration` (none, underline, line-through), `case` (as typed, upper, lower, title; the stored content never changes), an optional RGBA background box, and x/y canvas percentages in 0..=100 (default 50/84.5). The box has `padding_x`/`padding_y` and `radius` in 0..=128 and `offset_x`/`offset_y` in -1024..=1024 canvas pixels. It follows the text bounds, and its `background.*` animation channels (colour, padding, offset, corner radius) override the stored values while keyed. Sizes are project canvas pixels, scaled with the output. Invalid, unsupported or over-limit values refuse the whole candidate.
