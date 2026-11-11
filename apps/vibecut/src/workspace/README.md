# Windows and budgets

Interface: `Workspace`, `Window`.

Home, Editor and Shorts are independent native viewports. `Workspace::windows` registers
project windows by ProjectId; each owns its editor, history, playback, lease, picker,
pending persistence operation and close flow. Reopening a canonical path focuses the
existing window. Save As rekeys the same window to the acknowledged destination's fresh
ProjectId, keeping its viewport, playback and history; opening the source afterwards
makes a separate window, and reopening the destination focuses the issuing window. Home
hosts catalog and startup recovery prompts; each accepted recovery becomes a project
window. Closing Home hides it until Window > Home; the event loop exits after the last
window closes.

`window_ui(project_id, ui)` drives a project surface in kittest with unchanged automation
ids. `focused_editor`, `focused_editor_mut` and `editor_mut(ProjectId)` select the
session explicitly; Home has no active-editor slot. `Workspace::import_picked_files`
applies picker results through the drop import queue, so kittests inject there. Import
shutdown keeps its probe and work allowance reserved until the cancelled worker has
joined.

The app shares catalog, preferences, model presentation, lifecycle runner and aggregate
media accounting. Each project's media budget is a 512 MiB child of the 768 MiB
aggregate; autosave uses separate 64 MiB project children of a 256 MiB persistence
allowance, so export cannot consume journal or save admission. Model transfers stay
app-owned across project saves and closes. The workspace opens the one process
`MediaAccess` under `<root>/media` and lends the same mutex instance to every editor's
timeline display and to the desktop MCP inbox, so generation and display always share
cache entries and a second open of that root composes
[`RuntimeError::CacheBusy`](../../../../crates/engine/src/runtime/README.md#background-jobs-and-byte-policy)
through `engine::control::Error::Runtime`. The native adapter
keeps an audio stream per project and shares the GPU adapter. Preferences persist bounded
logical size, position and maximization by Home, Shorts and ProjectId (64 retained
geometry entries, within the [preference store bound](../preferences/README.md)).

Root Home, Editor `window_ui` and the immediate Shorts viewport call the shared
[text reader service](../services/README.md#focused-text-reads) before text widgets.
The composition retains one injectable reader; it claims and completes each viewport’s
request directly, including immediate children whose framework action queue is unserviced. Editor ownership includes
Session epoch and active scene so a late native result cannot follow a scene/window switch.
