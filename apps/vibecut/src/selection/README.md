# Selection

Interface: `SelectionScope`, `SelectionScopes`, `KeyframeSelection`,
`MaskPointSelection`, `SharedField`, `shared_fields`.

Transient nested scopes and shared inspector values; project edits stay in engine.
`selection_scopes` is app state and `inspect` reports the most specific active scope.

An empty-track drag selects intersecting visible items; Cmd/Ctrl/Shift adds to the
selection at gesture entry. Reversing the drag recomputes intersections; Escape cancels
an active marquee. Otherwise Escape clears mask points, then keyframes, then items.
Timeline Shift clicks toggle; ordered media-list Shift clicks extend from the anchor.
`timeline.deselect_all` clears every scope. Item selection syncs to Session history
before edits and restores on undo and redo; it never enters project JSON.
