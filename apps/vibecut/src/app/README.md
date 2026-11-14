# Composition root

Interface: `Workspace`, `EditorWindow`, `poll_workspace`, `background`.

The top app layer: it owns the window registry, focus, the stores it lends and the control
attach, and imports every window. A window never names it; each window module (`home`,
`shorts`, the editor) is driven through its own `show` and `Action` values.

| Module | Responsibility |
|---|---|
| `registry` | `Workspace`: Home's lifecycle worker, the project windows by identity, focus, Save As rekeying, lane-preference pruning and the per-frame tick. |
| `viewports` | Draws Home, each project viewport and the Shorts viewport, and carries their `Action` values out. |
| `window` | `EditorWindow`: one editor, its writer lease, pending save and close flow. |
| `chrome` | The window placeholders and prompts: loading, recovery, error, close. |
| `picker` | Native file pickers a window requests, polled without blocking and delivered to its editor. The shorts window's source and export dialogs are requested through `Action::Pick`, owned by the workspace and delivered to shorts state. |
| `work` | Background lifecycle jobs (discover, open, create, recover, persist), their worker and their polling. |
| `runs` | The shorts window's run store over the library the Home catalog worker holds (`home::Catalog`). |
| `lending` | The one lending point for process stores and the model service. |
| `geometry` | Window size and position memory inside the bounded preference document. |
| `control` | `poll_workspace` and `background`: the control attach. |

Home, Editor and Shorts are independent native viewports. `Workspace::windows` registers
project windows by ProjectId; each owns its editor, history, playback, lease, picker,
pending persistence operation and close flow. Reopening a canonical path focuses the
existing window. Save As runs the registry command `project.save_as` (a copy under a fresh
ProjectId at a path that does not exist), reopens the same window on that copy and retires
the original session with its recovery journal. The window keeps its viewport and lane
memory; history and playback restart on the copy. The picker confirmed the destination, so
the command runs with `replace: true`; the project's own file is still refused. Opening the source afterwards makes a separate window, and reopening the
destination focuses the issuing window. Home
hosts catalog and startup recovery prompts; each accepted recovery becomes a project
window. The editor's Home action (`window.home`) shows Home and closes that project; a project with no unsaved changes closes without the save prompt. Closing Home hides it until Window > Home; the event loop exits after the last
window closes.

`window_ui(project_id, ui)` drives a project surface in kittest with unchanged automation
ids. `focused_editor`, `focused_editor_mut` and `editor_mut(ProjectId)` select the
session explicitly; Home has no active-editor slot. `EditorWindow::import_picked_files`
(through `focused_window_mut`) applies picker results through the drop import queue, so kittests inject there. Import
shutdown keeps its probe and work allowance reserved until the cancelled worker has
joined.

The app shares catalog, preferences, model presentation, lifecycle runner and aggregate
media accounting. Each project's media budget is a 512 MiB child of the 768 MiB
aggregate; autosave uses separate 64 MiB project children of a 256 MiB persistence
allowance, so export cannot consume journal or save admission. Model transfers stay
app-owned across project saves and closes. `lending` opens the process stores once and
is the only place an editor receives them: the saved-sound library, the music session, the
brand kit (opened and held; `Workspace::brand_kit` hands it out, no request reads it yet),
the model service (lent for a frame and taken back) and the one process
`MediaAccess` under `<root>/media` (lent the person segmenter over the same verified models cache), whose mutex instance is lent to every editor's
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

## Control

The optional control listener and background attach compose the CLI-owned adapter over
the live `Workspace`; requests run against the same `Workspace` and `Session` as the
visible app, reach a window only through that window's own edit path, and an omitted
project target means the focused window. Syntax and boundaries:
[MCP control](../../../../docs/product/agent-control/mcp.md).
