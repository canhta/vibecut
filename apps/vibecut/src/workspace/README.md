# Windows and budgets

Home, Editor and Shorts are independent native viewports. `Workspace::windows`
registers project windows by ProjectId; each owns its editor, history, playback,
lease, picker, pending persistence operation and close flow. Canonical path
reopens focus the existing window.
Save As rekeys that same window to the acknowledged destination's fresh ProjectId,
retaining its native viewport, playback and history. Opening the source afterwards
creates a separate window; reopening the destination focuses the issuing window.
Home continues to host catalog and startup recovery prompts; each accepted recovery
becomes a project window. Closing Home
hides it until Window > Home focuses it, while the event loop exits after
the last window closes. The old editor back-to-Projects action is removed.
`window_ui(project_id, ui)` drives that same project surface in kittest;
automation ids remain unchanged inside each viewport. `focused_editor` / `focused_editor_mut` and `editor_mut(ProjectId)` select
the registered session explicitly; Home never contains an active-editor slot.

The app shares catalog, preferences, model presentation, lifecycle runner and
aggregate media accounting. Each project's media budget is a 512 MiB child of
the 768 MiB aggregate; autosave has separate 64 MiB project children of a 256 MiB
persistence allowance, so export cannot consume journal/save admission. Model
transfers remain app-owned across project saves/closes. The native adapter keeps
an audio stream for each project and shares the GPU adapter. Preferences version
4 persists bounded logical size, position and maximization by Home, Shorts and
ProjectId (64 retained geometry entries, within the store's 16 KiB bound).
Native multi-window smoke is maintainer-gated; macOS/Linux window paths are
unverified on the Windows development host.

## Injection and inspection

`Workspace::import_picked_files` applies native picker result paths through the
same import queue as drops; kittests inject at this boundary without opening a
dialog, then use real library/insert widgets. Import shutdown keeps its probe/work
allowance reserved until the cancelled worker has joined.
