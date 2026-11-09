# Projects page

`cargo run -p vibecut -- /native/path/to/project.json` acquires the project writer lease before loading. A supplied path stays pending until recovery discovery and the managed catalog are both idle.

With no file, the Projects page lists the managed catalog with thumbnail, name, main-scene duration and created date, in a persisted grid or list view and sort order. Search filters names live; click and shift-click select; the toolbar duplicates or permanently deletes the selection, with one confirmation naming it and no typed gate. Rename, open and project info (duration, created, modified, 8-character display ID) are card actions.

A catalog worker runs every `project.create`, `project.duplicate`, `project.rename`, `project.delete` and `project.thumbnail` command off the UI thread; the page never loads or mutates project JSON. Opening a card and the new-project entry use the same lifecycle/autosave path as external files. A failed catalog refresh keeps the last listing; Retry restarts the worker. Engine lifecycle hooks backfill and refresh project thumbnails, including on close without saving. Listing is supplied by [engine library](../../../../crates/engine/src/store/README.md#managed-project-library).
