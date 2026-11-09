# Colour picker

One reusable colour draft serves text, its background box, graphic fill/stroke, mask strokes and the empty inspector's canvas background. Hex/HSV/alpha entries, sliders, swatches and recent colours share it. Stable IDs: `color.hex`, `color.h/s/v/a` with `.slider` children, `color.swatch.N`, `color.recent.N`, `color.eyedropper`.

During a slider drag a temporary engine Session supplies only the preview snapshot; the editable Session, persistence and undo history stay unchanged. Release publishes one command and one undo step. Escape or closing discards the draft; selection or snapshot changes fence stale drafts. Recent colours are unique, newest-first, capped at sixteen.

The screen sampler is app-only: on macOS it calls AppKit `NSColorSampler` through the system JXA bridge, asynchronously. RGB replaces the draft hex and keeps alpha. Closing the draft cancels and joins the child; an abandoned request ends after 120 seconds. Windows and Linux return typed `EyedropperState::Unavailable` with localized copy.
