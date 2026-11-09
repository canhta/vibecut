# Colour picker

`color_picker` owns one reusable colour draft for text, its background box,
graphic fill/stroke, existing mask strokes and the empty inspector's canvas
background. Hex/HSV/alpha entries, sliders, swatches and the sixteen most recent
committed colours share it. Stable IDs are `color.hex`, `color.h/s/v/a`, their
`.slider` children, `color.swatch.N`, `color.recent.N` and `color.eyedropper`.
During a slider drag a temporary engine Session supplies only the preview snapshot;
the editable Session, persistence and undo history remain unchanged. Release
publishes one command/undo step. Escape or closing discards the active draft;
selection/snapshot changes fence stale drafts. Recent colours are unique,
newest-first and capped at sixteen.

The app-only native screen sampler invokes AppKit `NSColorSampler` through macOS's
system JXA bridge asynchronously; no browser EyeDropper API, screenshot capture,
new dependency or core OS import is used. RGB replaces the draft hex and retains
alpha. Closing the draft cancels/joins its owned child, and 120 seconds bounds
an abandoned request. Windows/Linux return typed `EyedropperState::Unavailable`
with localized copy; native macOS interaction is unverified on Windows.
`cargo xtask ui-audit inspector-transform inspector-color` captures both new scenes
at 1x/2x in both themes. The headless kittest saves a screenshot under `target/`.
