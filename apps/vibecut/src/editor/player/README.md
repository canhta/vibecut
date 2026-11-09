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
