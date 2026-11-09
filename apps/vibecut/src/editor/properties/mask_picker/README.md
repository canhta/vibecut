# Mask picker

One popover over the first mask of the selected clip, owned by the properties area because the Basic Mask row is its only entry. `state.rs` owns the capture's seven shape choices and typed field drafts; `view.rs` draws the grid and fields and returns actions for the properties owner.

The capture tile order is None, Horizontal, Mirror, Circle, Rectangle, Heart, Stars. Horizontal is the engine split at a default -90° so it keeps the top half; Mirror is the cinematic-bars band; the rest map straight to ellipse, rectangle, heart and star. Adding uses `masks.add`, changing a shape keeps centre, size and rotation through `masks.update`, and None runs `masks.remove`. A locked track disables every edit.

The reference shows feather 0–100 px (default 0), size W/H in placed-item pixels (default 60 %, ratio locked), position X/Y in pixels (default 0), rotation in degrees (default 0) and invert off. Here size and position are shown as per cent of the placed item, size is independent per axis, and the reference's corner radius is not rendered because the engine has no such parameter. All edits are one `masks.update` undo step; feather commits on release.
