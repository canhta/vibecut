# Timeline viewport

Interface: `State::scroll_bounds`, `State::scroll_input`, `State::scrollbars`, `fit_zoom`,
`anchored_scroll`, `edge_auto_scroll`, `max_scroll_seconds`, `slider_to_zoom` and
`zoom_to_slider`.

Maintains the timeline viewport: zoom, scroll, auto-scroll and fit arithmetic.

Playhead follow: `follow_transport` runs each frame after the scroll input. While playback runs
it pages the view to the playhead (`follow_scroll`, page-style); a wheel scroll or scrollbar drag
ends following until the next play. The rule and its reasons are in
[UI design](../../../../../../../docs/product/ui/editor.md).
