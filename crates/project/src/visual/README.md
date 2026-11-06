# Visual properties

Persisted visual item properties and their bounds: transform, blend selection,
masks, effects, video transitions, canvas bounds and text typography. Evaluation,
coverage math and device work belong to render and the desktop app.

| Module | Responsibility | Contract |
|---|---|---|
| `transform` | Owns transform units, defaults and clamps. | [Transform](#transform) |
| `blend` | Defines blend modes and opacity ownership. | [Blend selection and opacity](#blend-selection-and-opacity) |
| `mask` | Defines item masks and their animation paths. | [Masks](#masks) |
| `effects` | Defines the item effect stack and parameters. | [Item effects](#item-effects) |
| `transition` | Defines video transition entities and bounds. | [Video transitions](#video-transitions) |
| `bounds` | Holds JSON, canvas, audio and volume bounds. | [Canvas bounds](#canvas-bounds) |
| `typography` | Defines text item typography. | [Text typography](#text-typography) |

## Transform

`TransformField` owns the editor's units, defaults and finite clamps for the five
existing transform channels: position X/Y in project-canvas pixels
(−100,000..=100,000, default 0), independent X/Y scale ratios
(0.01..=10, default 1), and clockwise rotation in degrees
(−360..=360, default 0). The command adapter refuses out-of-bounds input;
animated evaluation clamps overshooting curves to these bounds. These channels
reuse the existing persisted animation representation without a schema change.

## Blend selection and opacity

`Item.blend_mode` persists as required `blendMode` using the 17 kebab-case IDs in
`BlendMode::ALL`. Normal is the creation default; audio items refuse a non-normal
blend. The render owner documents the linear-light formulas. Opacity has one
owner: the existing scalar `opacity` animation channel. Missing channels mean
unity; constant channels and keyframed channels share persistence and trim/retime.

## Masks

`Item.masks` stores an ordered vector of project-owned `Mask` values. IDs are
unique within the item and use the animation-path identifier grammar. Empty or
whitespace-only text masks are inactive while retaining their editable data. The vector
admits 1,024 entries for resource safety; presentation limits do not define the
model. Audio items refuse masks. Modes are None/Add/Subtract/Intersect/Lighten/
Darken/Difference; render owns their coverage math. Commands/history belong to
engine. Schema 19 requires `canvas.preset`/`canvas.last_custom_size`, `settings.loudness`, timeline transitions, scene arrays, per-item mask vectors, `blendMode`, text style and typography, audio control fields (audio source is optional, default upload), optional imported cue style and optional clip `fades` (`offset` defaults to 0); schema 18 and earlier refuse with `TooOld`.

Shapes include split, cinematic band, rectangle, ellipse, heart, diamond, star,
text and a cubic `MaskPath`. Text masks currently persist content and font size
and render with bundled Inter Regular; richer typography uses the shared text
contract when available. Normalized centers are offsets from the placed item
center, width/height are item fractions, rotation is degrees and uniform scale
multiplies shape size. Path positions/tangent offsets use normalized item space;
fill closes open paths. Feather, expansion, stroke width and text size use
project-canvas pixels. Color is the existing sRGB RGBA8 `Color`; opacity is 0..1.

IDs, finite ranges, scalar animation types and duplicate IDs validate with the
whole candidate. Paths admit 4,096 vertices and bounded normalized coordinates;
text admits 4 KiB and 1..4096-pixel font sizes. `masks.ID.params.KEY` uses the
existing `Animations` owner for center_x/center_y, width/height, rotation, scale,
feather, expansion, opacity and stroke_width. Values evaluate in item-local time;
`Mask::at` clamps animated overshoot to the static admissible range. No second
keyframe store or path interpolator is introduced.

## Item effects

`effect_definitions()` exposes the admitted effect names and scalar parameter bounds,
defaults and animation capability for UI consumers. `Item.effects` is an ordered stack of at most 64 visual `EffectInstance`s. Instance
IDs are item-local ASCII animation identifiers (1–64 bytes); duplicates refuse.
`EffectParams::Blur` stores finite intensity 0–100 (default 15); `enabled` bypasses
an instance. `EffectParams::ColorGrade` stores finite brightness, contrast,
saturation and hue, each -50..=50 with 0 identity (controls and ranges are new, not
measured parity; `GRADE_MIN`/`GRADE_MAX`); it has no animation paths. Audio items refuse visual effects. Evaluation/device work belongs to
render/the desktop app. Animated overrides use the existing `Animations` map and
`Param` storage at `effects.<instance-id>.params.intensity`, with scalar channels
only. No second keyframe store is introduced. Removing an effect removes its
animation paths; undo restores both. Schema 4 adds the required stack and refuses
older envelopes through the existing typed version errors.

`EffectParams::set_scalar` validates a registered scalar key and value before
replacing the static parameter; unsupported keys and invalid values retain the
previous parameter. Definition metadata and typed parameter validation share the
project owner.

## Video transitions

`Timeline.transitions` owns first-class `Transition` entities with project-wide
unique `TransitionId`, track, outgoing/incoming item IDs, kind, output start,
duration and `TransitionAlignment` (start, center, end, custom). The cut-adjacent items must
be adjacent and touching on the same video track and contain video or generated
solid overlays. `TRANSITION_DEFINITIONS` is the admitted catalog: cross dissolve,
dip to black and dip to white in the Dissolve family. Unknown kinds return
`UnsupportedTransition` with a bounded kind name, including during file ingestion.

A transition retains earlier outgoing IDs in `from_pieces` and later incoming
IDs in `to_pieces`, both in timeline order. Each side must form a contiguous chain
of video/overlay items on its track. `from`/`to` remain adjacent at the original
cut. `contributors(at)` resolves the appropriate piece on each side, using the
first/last piece's handles beyond the side extent. A split preserves the output
start, duration and alignment; it never renormalizes transition progress.

`Transition::bounds` intersects the two complete side extents with real source handles:
the incoming source in-point bounds the time before the cut and the outgoing
source's remaining known duration bounds the time after it. Unknown outgoing
source duration permits no extension after the cut. Generated overlays have no
media-handle constraint. `set_duration` clamps positive ticks to those bounds,
following alignment; an odd centered duration puts floor(duration/2) before the
cut. Custom duration changes keep the span's centre, clamped to stay over the cut.
Zero duration/handles and invalid spans refuse. Only one transition may name a cut, and spans on one track cannot overlap. Validation never repairs
untrusted persisted data. Edit-time revalidation belongs to [engine](../../../engine/src/session/edit/README.md#video-transitions).
`Timeline::rendered_items(from, to)` lists the visible items that draw in a range: those
overlapping it plus both clips of every overlapping transition, which render from their
handles outside their own extent. Range consumers (export copy and admission, HDR
detection, control previews) use it instead of filtering item extents.

## Canvas bounds

Names, notes and text use captions' NFC text owner. Canvas frames must have exact,
positive integer tick durations. Canvas area is at most 2^26 pixels; output audio
admits 8–192 kHz. Audio/video source volume and master volume admit -60..20 dB on a 0.01 dB grid; loudness targets admit -50..-5 LUFS and -20..0 dBTP on the same grid, and measured gains -60..60 dB for at most 4,096 sorted clips; font sizes admit 1..4096 pixels.

`Canvas.preset` records the chosen ratio preset (`original`, `16:9`, `4:3`,
`2:1`, `9:16`, `1:1`, `3:4`); `None` means a custom size. Fixed presets use a
1920 long edge: 16:9 = 1920x1080, 4:3 = 1440x1080, 2:1 = 1920x960, 9:16 =
1080x1920, 1:1 = 1080x1080 and 3:4 = 1080x1440. `original` resolves the retained
`original_canvas_size`. `CanvasSize::rounded_even` gives the nearest even size
on each odd axis (ties up) so encoders can open the frame: one pixel becomes
two, and an odd maximum that would overflow or exceed the area bound refuses.
The first-media capture of `original_canvas_size` and project load apply the same
rounding (load repairs older odd canvases instead of refusing the file).
`Canvas.last_custom_size` remembers the last committed custom width/height.
Both fields are schema 20. Preset labels are UI text; the
stored width/height remain authoritative.

## Text typography

Text items also carry a 1..256-byte font-family name, line `align` (left, centre,
right; default centre), `decoration` (none, underline, line-through), an optional RGBA
background box and x/y canvas percentages in 0..=100 (default 50/84.5 reproduces
the original lower-third box). The box has `padding_x`/`padding_y` and `radius` in 0..=128
and `offset_x`/`offset_y` in -1024..=1024 canvas pixels; it follows the text bounds, and
its `background.*` animation channels (colour, padding, offset, corner radius) override
the stored values while keyed. Weight, slant, spacing and line height are the
typography fields above; sizes are project canvas pixels, scaled with the output. Typed invalid/unsupported/limit errors refuse a whole candidate.
