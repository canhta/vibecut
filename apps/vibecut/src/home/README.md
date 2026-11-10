# Projects page

Interface: `State`, `Action`, `show`, `WINDOW`, `ProjectsPreferences`,
`ProjectsSortKey`, `ProjectsView`.

`show` draws the whole window and returns the one lifecycle `Action` the workspace must
carry out. The page never loads or mutates project JSON: a catalog worker runs every
`project.create`, `project.duplicate`, `project.rename`, `project.delete` and
`project.thumbnail` command off the UI thread, and listing is supplied by
[engine library](../../../../crates/engine/src/store/README.md#managed-project-library).

A project path supplied at startup acquires the writer lease before loading and stays
pending until recovery discovery and the managed catalog are both idle. Opening a card
and the new-project entry use the same lifecycle and autosave path as external files. A
failed catalog refresh keeps the last listing and Retry restarts the worker. Engine
lifecycle hooks backfill and refresh thumbnails, including on close without saving.

View, sort key and direction persist in [preferences](../preferences/README.md).
Geometry: [Home window](../../../../docs/product/ui/home.md).
