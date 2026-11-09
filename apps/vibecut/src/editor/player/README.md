# Editor player

Retains preview-only view state and renders the player panel from read-only
editor context.

`transform` draws the selected item's four corner and two side scale handles and
its rotation grip below the box from `render::viewport` geometry, then turns
pointer gestures into `timeline.set_item_param` patches inside `timeline.preview`.
One drop publishes one `timeline.commit_preview` step; Escape or a gesture that
returns to its origin discards the candidate. Hidden and locked items expose no
handles and cannot be hit. Handle face, hit target, grip gap and colours come
from `kit` tokens.

`snap` resolves the canvas targets a gesture can meet: the canvas centre and
edges and every other visible item's transformed box edges, in canvas pixels.
Move snaps the box's centre and both edges per axis, scale snaps the dragged
handle point per axis, and rotation snaps to quarter turns. The threshold is the
shared screen-pixel policy converted at the preview scale; `Preferences::snapping`
turns it off and Shift bypasses it for the gesture, the same rule the timeline
uses. A dashed accent guide is painted for each active target and cleared on
release, so the committed project and its undo history stay untouched until the
drop.

A double click on a visible text item opens one inline session over its rendered
box. Transparent input glyphs leave the preview visible; the caret and selection
are painted at the text crate's layout positions for the item's own font, size,
letter spacing, line height and alignment, mapped through the item's placement
transform so rotation and scale keep them on the rendered glyphs. Pointer press,
double-click word selection and drag selection invert that geometry and use the
text crate's point-to-index query; egui only supplies key and IME input. They use
the app accent at the measured caret width and selection opacity. Each change
stages the whole buffer as `text_content` inside `timeline.preview`; Escape, blur and
starting playback commit it as one undo step, Enter inserts a newline, and an
empty buffer is a valid commit. A locked or hidden item never opens the overlay.
