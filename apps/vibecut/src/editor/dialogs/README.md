# Editor dialogs

The export and model-manager sheets the editor window owns. Each dialog is one
module: its `State` is the only place its fields live, and its `show` receives
the editor's read-only `Context` and returns requests the frame routes through
the shared desk. Neither dialog reaches another area's state.

`State` in this module holds both dialog states so `Editor` keeps one field
instead of one per sheet. The frame paints the export sheet before the crop and
subtitle sheets and the model sheet after them, preserving modal order, and
drains each sheet's requests after it returns.
