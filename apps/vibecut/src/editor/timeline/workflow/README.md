# Timeline gestures and zoom

The timeline draws tracks, filmstrip/waveform placeholders, bookmarks and the
playhead; an adaptive ruler picks frame/second major and minor ticks from zoom
and project fps, labels them with time's shared DF/NDF timecode, and never
overlaps labels. The exponential zoom slider, ×1.7 buttons and Cmd/Ctrl+wheel
share a dynamic fit minimum (content occupies 25% of the viewport), a 100×
maximum and 75%→15% trailing padding; zooms above the 15% slider threshold hold
the playhead's viewport offset and crossing back below restores the pre-anchor
scroll. Wheel scrolls vertically, Shift/horizontal wheel scrolls horizontally,
and both clamp to content plus padding. Dragging near an edge auto-scrolls in a
100 px zone at up to 15 px per frame and stops at the content boundary or when
the pointer leaves. Movement compensates only the clamped viewport shift;
horizontal wheel follows egui content direction.
`cargo xtask ui-audit timeline-zoom-low timeline-zoom-high` captures both zoom
levels in dark/light at 1x/2x outside CI. Zoom, horizontal scroll, frame-snapped
ruler seeking,
single/Cmd-or-Shift multi-selection, drag move, edge trims, bookmark drag, split,
delete and ripple delete all dispatch engine commands. Ripple editing is a
persisted toolbar preference (off initially), also bindable in Shortcuts. It
stays out of app undo history so Undo restores the last project edit. The
app passes it explicitly into each atomic engine command; the [engine ripple
policy](../../../../../../crates/engine/src/session/edit/README.md#edit-semantics-and-limits) owns the gap-closing
semantics. Move and edge trim stage
the group contract: each pointer update sends absolute `timeline.move_items` or
`timeline.trim` params through `timeline.preview`, the panel paints
`preview_project()` while the committed project stays byte-identical, and the
release either publishes one undo step with `timeline.commit_preview` or
discards with `timeline.discard_preview` (Escape, gesture reversal and a refused
candidate). A move drop outlines the landing row or draws an insertion line for
the track a release creates; a refusal paints the engine's reason above the
scrollbar; a main-track item at the origin is pinned with an accent edge and
cannot move left. Snapping is a persisted preference (on by default) exposed by the
magnet toolbar control; it converts 10 logical pixels to ticks at the current
zoom and ranks item start/end, playhead, bookmark and animation-keyframe targets
by pixel distance then stable source order. The indicator line is drawn only
while a candidate is active. Move, trim and bookmark drag honour the toggle and
Shift bypasses it; the gesture preview changes no committed history or project
JSON until drop, and Escape restores the entry bytes.
