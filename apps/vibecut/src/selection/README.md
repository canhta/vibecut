# Selection

An empty-track drag selects intersecting visible items; Cmd/Ctrl/Shift adds to
the selection at gesture entry. Reversing the drag recomputes intersections, and
Escape cancels an active marquee. Otherwise Escape clears mask points, then
keyframes, then items. `selection_scopes` is transient app state; `inspect` reports
the most specific active scope. Timeline Shift clicks toggle; ordered media-list
Shift clicks extend from its anchor. `timeline.deselect_all` clears every scope.
Item selection is synchronized to Session history before edits and restored on
undo/redo; it never enters Project JSON.
