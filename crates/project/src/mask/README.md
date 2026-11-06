# Masks

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
