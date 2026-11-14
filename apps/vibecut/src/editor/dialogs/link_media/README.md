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

`picked` starts a match and returns at once; the hashing never runs on the UI thread. A worker thread (the same
channel-and-cancel-flag pattern the media import uses) takes the candidates one file at a time: a file pick checks the
asset it was asked for, a folder pick searches breadth first (four levels, at most 20 000 entries) for each missing
asset's file name. Each candidate goes through `SourceAsset::relink_with_cancel`, which checks the cancel flag before every
64 KiB read and reports bytes hashed, and accepts only a file whose content hash equals the recorded one. The dialog
polls the worker every frame and shows `Checking <name>` with a progress bar for the file being read; the row clicks, the
folder checkbox and `Link media` are disabled meanwhile, and `Cancel` stops the match and links nothing (a cancelled match
commits no partial result). Escape or closing the dialog drops the job, which raises the same flag. Only when the worker is
done do the hits become one `Session::transaction` of `media.relink` commands, one undo step, so the project is untouched
before the match is complete. Nothing found leaves `No file there matches the missing media.`; when every listed row is
linked the dialog closes with a toast.

`media.relink` is a Human-grant edit, like `import.add_asset`: its `asset` argument carries a path, so the
agent policy refuses it with the path reason. An agent that needs a lost clip back uses the existing
`timeline.replace_media` against a source the project already holds, or asks the person to link it.

Tests: `tests.rs` mounts the dialog alone over the one-area bench with a missing source in a temp dir,
checks the measured list and button sizes, the pick requests, the folder search, the hash check and the undo
step, and runs the match through an injected gated hasher (`set_hasher`) to assert progress shows, the dialog stays
responsive, Cancel stops it and nothing links; `apps/vibecut/tests/it/missing_media.rs` drives the full editor.
