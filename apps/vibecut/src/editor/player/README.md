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

A double click on a visible text item opens one inline session over its rendered
box: transparent glyphs leave the preview visible, while the caret and selection
use the app accent at the measured caret width and selection opacity. Each change
stages the whole buffer as `text_content` inside `timeline.preview`; Escape,
blur and starting playback commit it as one undo step, Enter inserts a newline,
and an empty buffer is a valid commit. A locked or hidden item never opens the
overlay.
