# Colour picker

Interface: `Picker`, `PickerEvent`, `palette`.

The target-free controls serve the properties and marker colour popovers, drawn as an
area under the field or marker that opened them. Callers provide an initial colour,
the anchor and the project's recent colours, and own the draft's lifetime. Recents, the
eyedropper row, the 7x3 recommended swatches, and the hex and HSVA entries with their
sliders return preview, commit and cancel events without applying a project edit. A
press outside the popover cancels, as Escape does. An opaque presentation option hides
Alpha and keeps RGB picks opaque; the default remains full RGBA. Properties uses this
only for RGB-only effect inputs and preserves stored animation alpha itself. Stable IDs:
`color.hex`, `color.h/s/v/a` with `.slider` children, `color.swatch.N`,
`color.recent.N`, `color.eyedropper`, `color.eyedropper.status`.

The service has no project target, command or history knowledge. The properties
owner maps events to its draft-preview and commit workflow; the marker editor maps
commits to its frame actions. Release returns a commit event; Escape or Cancel
returns a cancel event.

The eyedropper samples the player, on every platform. Its row arms `sampling`; the
player reports its next click as a frame pixel, which the owner hands to `sampled`.
The next pass applies it as a commit, keeping the draft's alpha. While armed, Escape
leaves sampling without closing the popover, and an outside press does not cancel.
The owner derives the player's armed state from `sampling` on every frame.
