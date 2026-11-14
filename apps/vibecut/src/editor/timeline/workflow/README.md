# Timeline gestures and zoom

Interface: the `State` gesture methods for item drag, bookmark drag, keyframe retime,
volume and fade drags, and `apply` with the gesture publish and discard steps in `apply.rs`.

The ruler is adaptive and frame-snapped; [its view](../view/ruler/README.md) owns tick
selection and labels.

Zoom: the exponential slider, ×1.7 buttons and Cmd/Ctrl+wheel share a dynamic fit minimum
(content fills 25% of the viewport), a 100× maximum and 75%→15% trailing padding. Zooms
above the 15% slider threshold hold the playhead's viewport offset; crossing back below
restores the pre-anchor scroll. Wheel scrolls vertically; Shift or a horizontal wheel
scrolls horizontally in egui's content direction. Horizontal scroll clamps to the content
plus trailing padding; vertical scroll clamps to the track stack. Dragging near an edge
auto-scrolls in a 100 px zone at up to 15 px per frame, stopping at the content boundary or
when the pointer leaves; movement compensates only the clamped shift.

Ruler seeking moves the shared transport. Move, edge trims, bookmark drag, split, delete,
ripple delete and track edits dispatch engine commands; zoom, scroll and selection stay
presentation state. Ripple is a persisted toolbar preference (off by default), bindable in
Shortcuts, outside app undo history so Undo restores the last project edit. The editor sets it on
the session when it changes and sends plain commands; gap-closing semantics belong to the [engine ripple
policy](../../../../../../crates/engine/src/session/edit/README.md#edit-semantics-and-limits).

Move and edge trim stage the group contract: each pointer update sends an absolute
`timeline.move_items` or `timeline.trim` command through `timeline.preview`, the panel
paints `preview_project()` while the committed project stays byte-identical, and release
either publishes one undo step with `timeline.commit_preview` or discards with
`timeline.discard_preview` (Escape, gesture reversal, refused candidate). A move drop
shows a clip-length slot in the landing row or a thin line for a created track, and the dragged
clip wears a white outline; a main-track item at the origin is not pinned, so moving it right
paints a placeholder from 0 and the gap stays (the engine's move never ripples; the magnet
closes only delete and split gaps), and nothing moves before 0; a refusal paints the engine's
reason above the scrollbar.

Snapping is a persisted preference (on by default) behind the magnet control. The
timeline's `snapping.rs` owns it: it converts 10 logical pixels to ticks at the current
zoom and ranks item start/end, playhead, bookmark and keyframe targets by pixel distance,
then stable source order, with `frame_snap` for pointer times. The preview canvas ranks
with the same `closest` rule from the [editor context](../../context/README.md) and its
own measured 5 px threshold. The indicator shows only while a candidate is active. Move,
trim and bookmark drag honour the toggle; Shift bypasses it.
