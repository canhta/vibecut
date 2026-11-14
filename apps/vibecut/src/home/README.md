# Projects page

Interface: `State`, `Action`, `Context`, `show`, `WINDOW`, `ProjectsPreferences`,
`ProjectsSortKey`, `ProjectsView`, `Catalog`.

`show(state, Context, ui)` draws the whole window and returns the lifecycle `Action`s the composition root must
carry out. The page never loads or mutates project JSON: a catalog worker runs every
`project.create`, `project.duplicate`, `project.rename`, `project.delete`, `project.restore`,
`project.purge` and `project.thumbnail` command off the UI thread, and listing is supplied by
[engine library](../../../../crates/engine/src/store/README.md#managed-project-library).

`Catalog` is the worker's open library, lent by the composition root to the shorts window (the library holds one exclusive lease, so no window opens a second handle). Each worker request and each lent call holds it for its own duration; the lent handle is not kept between calls, so Retry can reopen the catalog. Runs of the shorts window live in the same library and are never listed here.

The toolbar's Trash control swaps the grid for the trashed entries without changing page. A
trashed row shows its remaining retention days; Restore and Delete forever are its row menu
and the selection actions, and Delete forever passes through its own confirmation. Delete on
the Projects grid is the move to Trash, so a creator can always undo an accidental removal for
the retention the engine states.

The agent card in the tool row takes a prompt: sending it asks the catalog worker for a new project like the banner does, and the resulting `Action::OpenWithPrompt` carries the prompt to the composition root, which hands it to the editor that registers for that path; the editor opens its agent chat and starts the first turn once the creator accepts the consent sheet.

A project path supplied at startup acquires the writer lease before loading and stays
pending until recovery discovery and the managed catalog are both idle. Opening a card
and the new-project entry use the same lifecycle and autosave path as external files. A
failed catalog refresh keeps the last listing and Retry restarts the worker. Catalog-only
operations retain the library-owned result; mixed composition converts through engine Error
at its actual boundary, preserving safe messages and the original cause. Engine
lifecycle hooks backfill and refresh thumbnails, including on close without saving.

A project directory the library cannot read (older schema, damaged) is not migrated and does not fail the listing: the page shows one row per directory, "Couldn't read project" with its ID and the engine's reason, above the cards. New project and duplicate pick an ID that no directory, readable or not, already owns.

View, sort key and direction persist in [preferences](../preferences/README.md).
Geometry: [Home window](../../../../docs/product/ui/home.md).
