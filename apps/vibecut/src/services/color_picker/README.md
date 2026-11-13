# Colour picker

Interface: `Picker`, `PickerEvent`, `NativeEyedropper`, `EyedropperState`, `palette`.

The target-free controls serve the properties and marker colour popovers, drawn as an
area under the field or marker that opened them. Callers provide an initial colour,
the anchor and recent colours, and own the draft's lifetime. Recents, the eyedropper
row (only where sampling can run), the 7x3 recommended swatches, and the hex and
HSVA entries with their sliders return preview, commit and cancel events without
applying a project edit. A press outside the popover cancels, as Escape does. An opaque presentation option hides
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
120 seconds. Windows and Linux start `EyedropperState::Unavailable`, and the popover
omits the eyedropper row there.
