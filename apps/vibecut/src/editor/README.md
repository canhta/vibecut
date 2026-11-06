# Editor presentation


`editor.rs` retains the public `Editor` facade; `editor/state.rs` holds its
shared presentation context, `editor/widgets.rs` its automation-recording
controls and `editor/timing.rs` the tick-grid time formatting. `editor/frame.rs`
owns window composition and editor input routing. Its `State` owns the editor's
rename draft and splitter gesture; `show` returns window actions for the frame to
route. The read-only `editor::Context` carries the session, selection, locale and
theme into area interfaces. Each panel receives its complete slot, including its
own header strip; the frame supplies only the outer insets and draggable gutters.
The assets, player, properties, timeline, captions, export and models area files
are folder interfaces with private `view` and `workflow` implementations; existing
state projections live with their area. Responsibilities follow
[structure](../../../../docs/engineering/structure.md).

The physical extraction preserves existing facade calls and their ordering.
The player owns its retained view state and renders from read-only context,
returning requests for the frame to route. The assets State owns its navigation
drafts and prepared workflow projection; `show` consumes the shared
`editor::Context` and returns Action values routed by the frame. Its offscreen
audit uses the same surface. Remaining State/show/action migrations belong to
their consumer UI tickets; retained cross-area calls are recorded in each report.

`Editor::ui` runs without a window/device. `Playback::new` injects `Sources`, which
creates render's `SourceProvider`; production uses `FileSources` exactly as export
does. `AudioPort::fill` accepts a headless device callback for transport tests.
`Editor::activate` maps stable widget IDs to engine `execute` for every project
edit and to app-owned transport/preferences/selection. `inspect` exposes
enabled/focus/rect/current view, playing, position, selection, zoom, snapping
and the active snap-indicator target. The project remains the Session's immutable
snapshot. Each interactive
control, including every timeline item and trim handle, exposes the same stable
`author_id` through AccessKit, independent of locale and layout. Headless tests
query those IDs and click or drag through the real widget/engine path.
