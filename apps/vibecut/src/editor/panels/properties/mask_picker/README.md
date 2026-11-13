# Mask picker

Interface: `State`, `Action`, `show`.

One popover over the first mask of the selected clip, owned by the properties area
because the Basic Mask row is its only entry. `State` owns the seven shape choices and
typed field drafts; `show` draws the grid and fields and returns actions for the
properties owner.

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
Previous and next key seeks are not wired here, so the arrows stay disabled.
