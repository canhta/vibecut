# Visual properties

Persisted visual item properties: transform, blend selection, masks, effects, video transitions, canvas bounds and text typography. Evaluation, coverage math and device work belong to render and the desktop app.

## Transform

`TransformField` owns units, defaults and finite clamps for the five transform channels: position X/Y in project-canvas pixels (−100,000..=100,000, default 0), independent X/Y scale ratios (0.01..=10, default 1) and clockwise rotation in degrees (−360..=360, default 0). The command adapter refuses out-of-bounds input; animated evaluation clamps overshooting curves to these bounds.

## Blend selection and opacity

`Item.blend_mode` persists as required `blendMode` using the 17 kebab-case IDs in `BlendMode::ALL`. Normal is the creation default; audio items refuse a non-normal blend. Render documents the linear-light formulas.

Opacity is owned by the scalar `opacity` animation channel. A missing channel means unity.

## Crop

`Crop` is one rectangle in normalized source coordinates: `x`/`y` are the top-left corner and `width`/`height` the size, as fractions of the source frame. Every field is finite, width and height are positive, and the rectangle stays inside `0..=1`; the default is the full frame. Video and image items carry `VideoParams.crop`; absent means the full frame. Render sampling and the shorts batch path consume this one type, so it survives trim, retime, split, undo/redo and persistence as item content.

## Masks

`Item.masks` is an ordered vector of at most 1,024 `Mask` values. IDs are unique within the item and follow the animation-path identifier grammar. Audio items refuse masks. Empty or whitespace-only text masks are inactive but keep their data. Modes are None/Add/Subtract/Intersect/Lighten/Darken/Difference; render owns coverage math.

Shapes: split, cinematic band, rectangle, ellipse, heart, diamond, star, text and a cubic `MaskPath`. Text masks persist content and font size and render with bundled Inter Regular. Normalized centers are offsets from the placed item center, width/height are item fractions, rotation is degrees and uniform scale multiplies shape size. Path positions and tangent offsets use normalized item space; fill closes open paths. Feather, expansion, stroke width and text size use project-canvas pixels. Color is sRGB RGBA8 `Color`; opacity is 0..1.

Paths admit 4,096 vertices and bounded normalized coordinates. Text admits 4 KiB and 1..4096-pixel font sizes. `masks.ID.params.KEY` animates center_x/center_y, width/height, rotation, scale, feather, expansion, opacity and stroke_width through `Animations`, in item-local time. `Mask::at` clamps animated overshoot to the static admissible range.

## Item effects

`effect_definitions()` is the one catalog of effect kinds: each `EffectDefinition` carries a snake_case `id`, our `name`, its `EffectShelf` (the six library shelves, or `Inspector` for effects that belong to a clip inspector panel and are never listed) and its parameters. Each `EffectParamDefinition` has an `id`, a name, an `EffectParamKind`, a default `EffectValue` and an `animatable` flag. Kinds are `Float` (hard `min..=max` a typed value must satisfy, soft range for sliders), `Angle` (degrees, `-ANGLE_MAX..=ANGLE_MAX`), `Choice` (an index into named options), `Bool`, `Color` and `Point` (frame fractions, `0..=POINT_MAX`). Only scalar kinds (float, angle) may be `animatable`. `Item.effects` is an ordered stack of at most 64 `EffectInstance`s. An instance is `{id, kind, enabled, params}`: `kind` names its definition and `params` maps every parameter id of that definition (no more, no fewer) to a typed `EffectValue` (`Scalar`, `Choice`, `Bool`, `Color`, `Point`) that its kind accepts, finite and in range. Instance IDs are item-local ASCII animation identifiers (1–64 bytes); duplicates refuse. `enabled` bypasses an instance. Audio items refuse visual effects. `EffectDefinition::instance` builds an instance at the defaults and `EffectInstance::with` replaces one checked value.

Built-in kinds: `blur` has `intensity` 0–100 (default 15, animatable). `color_grade` has `brightness`, `contrast`, `saturation` and `hue`, each -50..=50 with 0 identity (`GRADE_MIN`/`GRADE_MAX`), static; the ranges are not measured parity. `chroma_key` has an opaque `key_color` (default green, static) plus `intensity` and `shadow`, each 0–100 with defaults 40 and 0 measured from the reference and animatable; intensity 0 keys nothing. `effects.update_param` carries one scalar: a float or angle takes the number, a choice its index, a bool 0 or 1 and a colour one packed `0xRRGGBB` (`PACKED_COLOR_MAX`, exact integers only, via `packed_color`), so a colour pick is a single command and one undo step. Points need two components and are not addressable by `update_param`. Scalar channels animate at `effects.<instance-id>.params.<id>` for every animatable parameter of the definition (`EffectDefinition::animated_ids`). Removing an effect removes its animation paths; undo restores both.

`EffectInstance::set_scalar` validates a registered parameter id and value before replacing the parameter; unknown ids and invalid values keep the previous parameter.

## Video transitions

`Timeline.transitions` owns `Transition` entities with a project-wide unique `TransitionId`, track, outgoing and incoming item IDs, kind, output start, duration and `TransitionAlignment` (start, center, end, custom). The two items must be adjacent and touching on the same video track and contain video or generated solid overlays. `TRANSITION_DEFINITIONS` is the admitted catalog; each entry carries the kind id, our name, family, browser shelf and typed parameter definitions. It currently holds cross dissolve, dip to black, dip to white and dip to colour, all Dissolve family on the Basic shelf. Unknown kinds return `UnsupportedTransition` with a bounded kind name, including during file ingestion.

`Transition.params` stores only overrides of the catalogue defaults, keyed by parameter id; `Transition::value` resolves override else default. A definition declares each parameter as a float (min, max, default, unit), choice, bool, sRGB colour, point (fractions of the frame, 0..=1), angle (degrees, at most four turns either way) or seed (0..=9999). `Transition::set_param` and validation refuse an unknown id, a value of another type, a non-finite number or a value outside the bounds, and validation refuses the whole project rather than repairing it. Parameters are fixed per transition and never animate.

A transition keeps earlier outgoing IDs in `from_pieces` and later incoming IDs in `to_pieces`, both in timeline order. Each side must form a contiguous chain of video or overlay items on its track. `from`/`to` stay adjacent at the original cut. `contributors(at)` resolves the piece on each side, using the first or last piece's handles beyond the side extent. A split preserves output start, duration and alignment and never renormalizes progress.

`Transition::bounds` intersects the two complete side extents with real source handles: the incoming source in-point bounds the time before the cut, and the outgoing source's remaining known duration bounds the time after it. Unknown outgoing duration permits no extension after the cut. Generated overlays have no handle constraint. `set_duration` clamps positive ticks to those bounds, following alignment; an odd centered duration puts floor(duration/2) before the cut. Custom duration changes keep the span's centre, clamped to stay over the cut. Zero duration or handles and invalid spans refuse. Only one transition may name a cut, and spans on one track cannot overlap. Validation never repairs persisted data. Edit-time revalidation belongs to [engine](../../../engine/src/session/edit/README.md#video-transitions).

`Timeline::rendered_items(from, to)` lists the visible items that draw in a range: those overlapping it plus both clips of every overlapping transition, which render from their handles outside their own extent. Range consumers (export copy and admission, HDR detection, control previews) use it instead of filtering item extents.

## Canvas bounds

Names, notes and text use captions' NFC text owner. Canvas frames have exact positive integer tick durations. Canvas area is at most 2^26 pixels; output audio admits 8–192 kHz. Audio/video source volume and master volume admit -60..20 dB on a 0.01 dB grid. Loudness targets admit -50..-5 LUFS and -20..0 dBTP on the same grid; measured gains admit -60..60 dB for at most 4,096 sorted clips. Font sizes admit 1..4096 pixels.

`Canvas.preset` is the chosen ratio preset (`original`, `16:9`, `4:3`, `2:1`, `9:16`, `1:1`, `3:4`); `None` means a custom size. Fixed presets use a 1920 long edge: 16:9 = 1920x1080, 4:3 = 1440x1080, 2:1 = 1920x960, 9:16 = 1080x1920, 1:1 = 1080x1080, 3:4 = 1080x1440. `original` resolves the stored `original_canvas_size`. The stored width/height stay authoritative.

`CanvasSize::rounded_even` gives the nearest even size on each odd axis (ties up) so encoders can open the frame; an odd maximum that would overflow or exceed the area bound refuses. The first-media capture of `original_canvas_size` applies the same rounding. `Canvas.last_custom_size` is the last committed custom width/height.

## Text typography

Text items carry a 1..256-byte font-family name, line `align` (left, centre, right; default centre), `decoration` (none, underline, line-through), an optional RGBA background box, and x/y canvas percentages in 0..=100 (default 50/84.5). The box has `padding_x`/`padding_y` and `radius` in 0..=128 and `offset_x`/`offset_y` in -1024..=1024 canvas pixels. It follows the text bounds, and its `background.*` animation channels (colour, padding, offset, corner radius) override the stored values while keyed. Sizes are project canvas pixels, scaled with the output. Invalid, unsupported or over-limit values refuse the whole candidate.
