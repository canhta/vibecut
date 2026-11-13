# Editor dialogs

Interface: `State`, `Action`, `show_export`, `show_models`, `show_preferences`, `show_voiceover`.

The export, model-manager and AI preferences sheets the editor window owns. Each dialog is one
module: its `State` is the only place its fields live, and its `show` receives the
editor's read-only context and returns requests the frame routes through the shared
write context. Neither dialog reaches another area's state.

`State` holds the dialog states so `Editor` keeps one field instead of one per
sheet. The frame paints the export sheet before the crop and subtitle sheets and the
model sheet after them, preserving modal order, and drains each sheet's requests after
it returns.

`voiceover.open` (the timeline toolbar's mic button) opens the voice-over recorder, a modal
over the [capture service](../../services/voiceover/README.md). Record opens the chosen or
default input, shows a live meter and the elapsed sample time, and Stop inserts the finished
take through `import.add_asset` and `timeline.insert` in one transaction (one undo step) at
the playhead of the moment Record was pressed, then selects it. Cancel, Escape, a device
failure or a refused insertion leaves no file and no item and shows a typed line. A running
take also blocks global key dispatch. Tests inject the input host and the media folder with
`Editor::set_voiceover`.

`preferences.open` in the Window menu opens Settings / AI models. Each remote
provider/feature row puts its shipped default first, then explicitly refreshed IDs,
recent IDs and Custom. Apply validates a custom ID before one preference-history
edit. Refresh is enabled only with that provider's valid key and runs one bounded
background list request. Loading, empty and error states keep manual selection;
key changes cancel and fence stale results. EN/VI rows share the existing field
system; these new scenes have no matching desktop reference.
