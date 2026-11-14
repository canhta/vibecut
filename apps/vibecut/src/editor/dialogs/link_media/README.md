# Link media dialog

Interface: `State`, `Action`, `Pick`, `LinkMediaView`, `show`, `apply`, `picked`.

The modal that reconnects sources whose file is gone. The window opens it once when a project
opens with missing sources (`Editor::offer_link_media`, called by the app's store lending); the
properties panel's `Link media` button opens it again through `Request::LinkMedia`. Opening lists
the missing sources (name, last path, duration) in project order and stays closed when none is
missing. A row is linked when its asset's path is no longer the one listed.

Layout is measured on the reference's Link media dialog: [Missing media](../../../../../../docs/product/ui/editor.md#missing-media).
The footer's `Link media` button asks the window for a native pick through `Pick`: a folder while the
folder checkbox is on (default), otherwise one file for the first unlinked row; a click on a row asks for one
file for that source. The window serves the pick in `app/picker.rs` (`PickerAction::LinkMedia`) and hands the
path back through `Editor::link_picked`, which calls `picked`.

`picked` never trusts a name. A file relinks the asset it was asked for; a folder is searched breadth
first (four levels, at most 20 000 entries) for each missing asset's file name. Every candidate goes through
`SourceAsset::relink`, which accepts only a file whose content hash equals the recorded one. The hits are
one `Session::transaction` of `media.relink` commands, so one undo step. Nothing found leaves the line
`No file there matches the missing media.` and changes nothing. When every listed row is linked the dialog
closes with a toast. Hashing runs on the UI thread, as the import path's probe does today; a very large file
blocks a frame while it is read.

`media.relink` is a Human-grant edit, like `import.add_asset`: its `asset` argument carries a path, so the
agent policy refuses it with the path reason. An agent that needs a lost clip back uses the existing
`timeline.replace_media` against a source the project already holds, or asks the person to link it.

Tests: `tests.rs` mounts the dialog alone over the one-area bench with a missing source in a temp dir,
checks the measured list and button sizes, the pick requests, the folder search, the hash check and the undo
step; `apps/vibecut/tests/it/missing_media.rs` drives the full editor.
