# Selection

Interface: `SelectionScope`, `SelectionScopes`, `KeyframeSelection`,
`MaskPointSelection`; `Editor::selection`, `Editor::select`.

The engine `Session` is the only owner of the selected items: areas read them through
`Context::selection` or `Session::selection`, and every write goes through the one
`select` entry (`Port::select` for an area), which calls `Session::set_selection`. The session refuses a change
while a gesture is open; a click, marquee or `Editor::select` therefore ends the
inspector's scrub, wheel, curve or picker gesture and the timeline graph popover first
(the same reconciliation the inspector runs every frame), and any other open gesture
leaves the selection unchanged with the refusal on the error line. A marquee is not an
engine gesture, so it writes the selection on every pointer move. The selection never
enters Project JSON; the session retains it with history and restores it on undo and
redo, and prunes vanished items itself.

This module holds only the presentation scopes nested inside that selection:
`SelectionScopes` carries the selected mask points, keyframes and the selected
transition, and `active` reports the most specific one. They are view state of the
properties and player areas and clear once the session selection no longer contains
their item. `intersections` is the marquee hit test and `click` the toggle and ordered
range rule shared by the timeline and the media list.

An empty-track drag selects intersecting visible items; Cmd/Ctrl/Shift adds to the
selection at gesture entry. Reversing the drag recomputes intersections; Escape cancels
an active marquee. Otherwise Escape clears mask points, then keyframes, then items.
Timeline Shift clicks toggle; ordered media-list Shift clicks extend from the anchor.
`timeline.deselect_all` clears every scope.
