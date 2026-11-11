# Editor player

Interface: `State`, `show`, `Action`, `PreviewAxis`, `PreviewGuide`, `TextCaret`, `crop`.

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
snapshot directory persist in the preference document. The header and context menus offer
the next/previous bookmark actions, enabled and dispatched through the frame's one
clamped, no-wrap bookmark lookup; an actual jump shares `playback.seek` and preserves play
state, selection and history.

## Crop

The toolbar Crop modal edits a transient `project::Crop` through the shared `crop`
control, locked to the selected ratio (Custom resizes freely, Original keeps the source
shape), and writes it with one `timeline.set_item_param` patch on Apply, so closing never
mutates the project.
