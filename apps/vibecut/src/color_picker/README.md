# Colour picker

Interface: `Picker`, `ColorTarget`, `PickerEvent`, `NativeEyedropper`.

One reusable colour draft serves text, its background box, graphic fill and stroke,
mask strokes, the marker colour and the empty inspector's canvas background. Hex,
HSV and alpha entries, sliders, swatches and recent colours share it. Stable IDs:
`color.hex`, `color.h/s/v/a` with `.slider` children, `color.swatch.N`,
`color.recent.N`, `color.eyedropper`.

During a slider drag a temporary engine Session supplies only the preview snapshot;
the editable Session, persistence and undo history stay unchanged. Release publishes
one command and one undo step. Escape or closing discards the draft; selection or
snapshot changes fence stale drafts. Recent colours are unique, newest-first, capped
at sixteen in [preferences](../preferences/README.md).

The screen sampler is app-only: on macOS it calls AppKit `NSColorSampler` through the
system JXA bridge, asynchronously, and RGB replaces the draft hex while keeping
alpha. Closing the draft cancels and joins the child; an abandoned request ends after
120 seconds. Windows and Linux return typed `EyedropperState::Unavailable` with
localized copy.
