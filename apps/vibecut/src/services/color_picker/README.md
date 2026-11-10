# Colour picker

Interface: `Picker`, `PickerEvent`, `NativeEyedropper`, `EyedropperState`, `palette`.

The target-free controls serve the properties and marker colour windows. Callers
provide an initial colour and recent colours and own the draft's lifetime. Hex,
HSV and alpha entries, sliders, swatches and recents return preview, commit and
cancel events without applying a project edit. An opaque presentation option hides
Alpha and keeps RGB picks opaque; the default remains full RGBA. Properties uses
this only for RGB-only effect inputs and preserves stored animation alpha itself. Stable IDs:
`color.hex`, `color.h/s/v/a` with `.slider` children, `color.swatch.N`,
`color.recent.N`, `color.eyedropper`.

The service has no project target, command or history knowledge. The properties
owner maps events to its draft-preview and commit workflow; the marker editor maps
commits to its frame actions. Release returns a commit event; Escape or Cancel
returns a cancel event. Palette and window geometry are shared by both callers.

The screen sampler is app-only: on macOS it calls AppKit `NSColorSampler` through the
system JXA bridge, asynchronously, and RGB replaces the draft hex while keeping
alpha. Closing the draft cancels and joins the child; an abandoned request ends after
120 seconds. Windows and Linux return typed `EyedropperState::Unavailable` with
localized copy.
