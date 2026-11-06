# Timeline

Each item has a 16 px chevron that opens its keyframe lanes; the open set is
presentation state persisted as `timeline_lanes` in the preference document,
keyed by the owning project so item ids that restart elsewhere cannot alias it.
Save As carries the issuing project's set to the copy's fresh ProjectId while the
original keeps its own; a refused Save As changes neither. Projects removed from
the library are pruned when preferences next save.
A lane is 20 px high, one per nonempty animation channel (audio always offers
its volume lane), and shows the channel's diamonds. Click selects, Cmd/Ctrl/Shift
toggles, a lane marquee selects, drag retimes with the same snapping candidates
as clips and commits one engine gesture, and Delete removes every selected
diamond through `keyframes.delete_selection` in one undo step. Right-clicking a
lane offers before/after Hold or Linear through `keyframes.set_extrapolation`.
Audio clips draw a draggable volume line; its position uses the ported linear-gain
squared map over -60..20 dB, the drag commits on the 0.1 dB grid while field and
stored edits keep 0.01 dB, the tooltip shows the current value, and volume
keyframes appear on the same line and in the expanded lane. The offscreen audit
capture is written under `target/`.

## Track controls

Timeline header and row geometry follows the [UI design](../../../../../docs/product/ui/editor.md#density),
using embedded regular SVG kind icons, output toggles and accessibility-only
names. Logical row sizes stay constant across DPI scaling; vectors are cached at
the display density. Empty rows have dashed placeholders. Header context
menus remove unlocked secondary tracks; main has a persisted magnet control in
its header and the timeline toolbar. All edits use registered engine commands and
refresh the shared preview snapshot. Paste is supplied by the clipboard owner.
Media insertion reuses an unlocked compatible track only when the playhead range
fits; otherwise it creates a secondary track at the top of its group. Solid colour
is generated Video content, distinct from Sticker tracks.

Header context menus also rename tracks inline (Enter commits, Escape cancels)
and toggle the engine's lock barrier. Lock protects item structure and properties,
masks, effects and keyframes; selection, playback and track metadata stay usable.
Dragging a header reorders within its overlay or audio section; main stays fixed.
Escape cancels the reorder. The explicit Add track toolbar action uses the active
item's compatible kind above its track, or Video at the overlay section end when
there is no active item. Commands and undo/persistence belong to
[engine](../../../../../crates/engine/src/session/edit/README.md#edit-semantics-and-limits).

## Scene switcher

The timeline toolbar centres a split button with the current scene name and a
scene-list control. The sheet lists all scenes and switches through
`scene.switch`; Select mode permits multi-delete while disabling main. Delete
opens a confirmation dialog explaining that the action can be undone. Closing or
Escape dismisses the sheet. Per-scene zoom/scroll/playhead restore through the
Session metadata view; item/keyframe/mask-point selection clears on switching.
The sheet's plus action names the scene `Scene N`, runs `scene.create` as one
undoable edit and then switches to it; the sheet stays open. A row's context menu
opens an inline rename editor for any scene, including main: Enter commits through
`scene.rename` and Escape cancels. New scenes inherit the project canvas, frame
rate and background and start with an empty timeline. English/Vietnamese copy uses
the existing catalog.
A delete confirmation takes Escape before the rename editor. If a command or undo
removes the edited scene, its pending name edit is discarded.

`tests/it/scenes_ui.rs` operates the real widgets and engine at 1x/2x density, checks
switching, protected main, create/rename history, persistence and atomic
multi-delete/undo without writing images.
`cargo xtask ui-audit scene-create scene-rename` captures the created scene and
inline editor separately in both themes at 1x/2x.
