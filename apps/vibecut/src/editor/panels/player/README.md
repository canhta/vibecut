# Editor player

Interface: `State`, `show`, `Action`, `PreviewAxis`, `PreviewGuide`, `TextCaret`, `Guide`,
`OverlayVisibility`, `crop`.

Owns the preview panel: its chrome, transport toolbar, timecode field, zoom controls,
transform handles, inline text and the crop dialog. Transport, preview frames and the crop
model are the [services](../../../services/README.md) owners this panel calls.

`transform` draws the selected item's four corner and two side scale handles and its
rotation grip below the box from `render::viewport` geometry, then turns pointer gestures
into `timeline.set_item_param` patches inside `timeline.preview`. One drop publishes one
`timeline.commit_preview` step; Escape or a gesture that returns to its origin discards the
candidate. Hidden and locked items expose no handles and cannot be hit. Handle face, hit
target, grip gap and colour come from [UI tokens](../../../../../../docs/product/ui/tokens.md).

`snap` resolves the canvas targets a gesture can meet: the canvas centre and edges and
every other visible item's transformed box edges, in canvas pixels. Move snaps the box's
centre and both edges per axis, scale snaps the dragged handle point per axis, and
rotation snaps to quarter turns within 5°. The threshold is the shared 5 px preview policy
converted at the preview scale; `Preferences::snapping` turns it off and Shift bypasses it
for the gesture, the same rule the timeline uses. A dashed accent guide is painted for
each active target and cleared on release, so the committed project and its undo history
stay untouched until the drop.

`matte_brush` is the person-matte refinement brush. While the properties panel arms it
(`State.matte_brush`, the clip and an add or remove mode), the canvas shows a crosshair, takes the
drag instead of the transform gizmo, maps each pointer position through `Renderer::frame_point`
(crop, fit and transform undone) to a decoded-frame point, draws a translucent trail, and on release
sends one `Action::PaintMatteStroke` carrying a `project::MatteStroke` stamped with the item's source
time at the playhead. Escape, a selection change or leaving the panel disarms it; the stroke itself
is one undo step ([properties](../properties/README.md#remove-bg-sub-tab)). The tip radius is a fixed
fraction of the frame's shorter side because no reference value exists.

`mask_path` edits the single selected item's active closed path mask. Anchors and the
selected anchors' in/out tangents are drawn through `render::viewport::MaskGeometry`,
the same mask-box, item and viewport affine that draws the contour, so handles stay on
the rendered curve under mask and item rotation and scale. Clicking an anchor selects
it, Shift toggles it, dragging a selected anchor moves the canonical point set by one
mask-local delta and dragging a tangent changes only that side. A press on empty canvas
that releases without moving clears the point scope, or the item selection when no point
is selected; one that moves draws a marquee and, on release, selects the anchors inside
its box (Shift adds them to the scope). Clicking the curve
within 6 logical pixels inserts one anchor at the nearest cubic parameter; Delete
removes the selected anchors through `masks.remove_point` as one undo step and never
deletes the item. Escape cancels a live drag, then clears the point scope, then falls
through to the item scope. A completed drag stages `masks.set_path` inside
`timeline.preview` and publishes one undo step.

`mask_box` adds the whole-mask gizmo to the same target: the mask box is outlined with
four corner handles and a rotation grip, all mapped through `MaskGeometry`. Dragging a
corner scales the mask uniformly about its centre (`scale`, the freeform path's own uniform
size mode), the grip rotates it (`rotation`), and a press inside the box that meets no
anchor, tangent, handle or curve moves it (`center_x`/`center_y`); a Shift press inside the
box starts the additive marquee instead. Anchors win where they coincide with a corner. A
gesture stages `masks.update` with absolute values from the press inside `timeline.preview`
and one drop publishes one undo step; like the inspector it edits the static fields.
Snapping uses `snap`: the moved box's edges and centre meet the canvas and other items'
lines, the dragged corner meets them per axis, and rotation meets quarter turns within 5°;
the preference turns it off and Shift bypasses it. The inserted point's cubic parameter is
the nearest one clamped to 0.001..0.999 and split by de Casteljau, the reference's
`insertPointIntoFreeformSegment` rule. Selected anchors' tangents and the gizmo handles
expose the `preview.mask.tangent.<n>.<in|out>` and `preview.mask.box.*` targets.

A double click on a visible text item opens one inline session over its rendered box.
Transparent input glyphs leave the preview visible; the caret and selection are painted at
the text crate's layout positions for the item's own font, size, letter spacing, line
height and alignment, mapped through the item's placement transform so rotation and scale
keep them on the rendered glyphs. Pointer press, double-click word selection and drag
selection invert that geometry and use the text crate's point-to-index query; egui only
supplies key and IME input. Caret width and selection opacity are the `TEXT_EDIT_*` tokens
listed in [UI tokens](../../../../../../docs/product/ui/tokens.md). Each change stages the
whole buffer as `text_content` inside `timeline.preview`; Escape, blur and starting
playback commit it as one undo step, Enter inserts a newline, and an empty buffer is a
valid commit. A locked or hidden item never opens the overlay.

## Transport and timecode

The panel renders the transport toolbar, an editable `HH:MM:SS:FF` timecode with the
project duration, and Fit / 25 / 50 / 75 / 100 / 150 / 200% zoom presets with continuous
0.25-16x steps. Enter dispatches `playback.seek`; Esc or invalid text restores the previous
value. Ctrl/Cmd+wheel and pinch zoom; plain wheel, two-finger scroll and middle-drag pan
only above 100%, clamped to the canvas bounds. Fullscreen covers the window and sends the
native viewport request; Esc or the toggle exits. The fit flag, explicit scale and last
snapshot directory persist in the preference document. The canvas context menu offers the
next/previous bookmark actions, enabled and dispatched through the
[action registry's bookmark lookup](../../../actions/README.md#bookmark-navigation); an
actual jump shares `playback.seek` and preserves play state, selection and history.

## Crop

The toolbar Crop modal edits a transient `project::Crop` through the shared `crop`
control, locked to the selected ratio (Custom resizes freely, Original keeps the source
shape) and writes it with one `timeline.set_item_param` patch on Apply, so closing never
mutates the project. The dialog reads the session, theme and error line through the frame's
`Port` and never names `Editor`.

## Guides and overlays

`overlay` holds the model: the `Guide` choice (none, a grid with rows and columns in 1–24, a platform: TikTok, Instagram Reels, YouTube Shorts, whose `covered_regions` are fractions of the frame, or a custom set of four margins in whole percent of the canvas, each 0–99 and each axis's pair at most 99),
per-overlay `OverlayVisibility` and the paint planes. Planes paint
back to front: `Scene` (the frame image, painted first), `Viewport` (the guide, fixed over the
displayed frame) and `Hud` (bookmark notes, screen-anchored top-left). The transform handles
are drawn after all three.

The header menu holds the guide picker (None, Grid, a platform or Custom), the grid size fields, the custom margin fields and the guide
switch; the canvas context menu holds the notes switch. Each choice is one `preview_guide` or
`preview_overlays` preference edit recorded in the app's preference history, so it is undoable
and never touches the project. Grid sizes follow the properties scrub rule: a drag or typed
edit drafts the sizes in `State::grid_draft` (shown, not stored), and one `preview_guide` edit
publishes the final value when the drag releases or the field loses focus. Escape, or closing the menu, drops the draft and publishes nothing. A drag across
several values is one undo step; a drag that ends where it started records nothing. Notes show each bookmark whose span covers the playhead, start and end inclusive;
a bookmark with an empty note shows nothing. Guides and notes are painted only here, so export
never sees them.

Platform guides shade the regions the platform's UI covers, fractions and sources in
[the editor UI doc](../../../../../../docs/product/ui/editor.md#preview-guides-and-notes); Spotlight has no primary source and is absent. Custom margins shade the four bands they leave at the canvas edges; a zero margin shades nothing. Margin edits follow the grid-size rule: a drag or typed edit drafts the values and one preference edit publishes the valid final value when the gesture ends, while an invalid draft keeps its inline error and publishes nothing. Every value in this section is [new]; no capture exists for the picker or for notes.
