# Projects page

`cargo run -p vibecut -- /native/path/to/project.json` acquires the project writer
lease before loading. Startup keeps a supplied project path pending until
recovery discovery and the managed catalog are both idle, regardless of which
worker finishes first. With no file the managed Projects page lists the catalog
with thumbnails, name, main-scene duration and created date in a persisted grid
or list view and sort order. Search filters names live; click and shift-click
select, and the toolbar duplicates or permanently deletes selected projects.
Delete shows one confirmation naming the selection and has no typed gate.
Rename, open and project info (duration, created, modified and the 8-character
display ID) come from each card's actions. A catalog worker executes every
`project.create`, `project.duplicate`, `project.rename`, `project.delete` and
`project.thumbnail` command off the UI thread; the page never loads or mutates
project JSON itself. Opening a card and the new-project entry point then use the
same lifecycle/autosave path as external project files. Managed project listing
is supplied by [engine library](../../../../crates/engine/src/store/README.md#managed-project-library); the projects-page feature owns its presentation.
`cargo xtask ui-audit projects projects-rename projects-delete projects-info`
captures the populated page and its modal dialogs offscreen in both themes at
1x/2x. Failed catalog refresh retains the last listing; Retry restarts its worker.
Engine lifecycle hooks backfill and refresh project-owned thumbnails, including
close without saving editable content. Preview,
timeline basics, transport and the export dialog are present; text/audio controls
are integrated by their owning feature tickets.
