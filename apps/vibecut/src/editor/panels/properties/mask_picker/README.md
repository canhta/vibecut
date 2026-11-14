# Mask picker

Interface: `State`, `Action`, `Page`, `show`, `inline`.

One popover over the first mask of one scope of the selected item, owned by the properties
area. Its popover serves the video Mask row; the layer and Adjust pages draw the same
tiles and fields inline (`inline`, `Page`). `State` owns the scope it was opened for
(`open(item, scope)`, `set_scope`, `scope()`), the seven shape choices and typed field drafts;
`current(item, scope)` is the
first mask of that scope; `show` draws the grid and fields and returns actions for the
properties owner, which creates a new mask with the open scope.

The tile order is None, Horizontal, Mirror, Circle, Rectangle, Heart, Stars, Path.
Horizontal is the engine split at a default -90° so it keeps the top half; Mirror is
the cinematic-bars band; the rest map straight to ellipse, rectangle, heart and star.
Path seeds a closed unit ellipse. Choosing Path again on an existing path keeps its
customized anchors instead of reseeding. Adding uses `masks.add`, changing a shape
keeps centre, size and rotation through `masks.update`, and None runs `masks.remove`.
A locked track disables every edit.

Feather is 0–100 px (default 0) and commits on release; size W/H and position X/Y are
per cent of the placed item with size independent per axis; rotation is in degrees
(default 0) and invert is off. The reference's corner radius is not rendered because
the engine has no such parameter. Stored edits are one `masks.update` undo step.

Each numeric group (feather, size, position, rotation) has a row-end diamond. It keys the
group's `masks.<id>.params.<name>` paths at the playhead through `keyframes.toggle_at`,
or removes them when all are keyed, as one step. Fields show the mask evaluated at the
playhead. An animated field with keys upserts its key at the playhead; one with no keys
replaces its base with `keyframes.set_param`; any other field edits the stored mask.
The arrows seek to the nearest key of the group either side of the playhead, through the
same `Request::Seek` the properties keyed sliders use, and stay disabled when no key exists
on their side.
