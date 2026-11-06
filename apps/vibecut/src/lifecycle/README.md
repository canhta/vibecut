# Project lifecycle


`Workspace` owns loading/exiting/error/recovery presentation and dispatches blocking
open, discovery, save, flush and discard work through engine's job runner. The
persistent `EditorSession` wraps the sole engine `Autosave`; the memory variant
supports the existing headless editor injection interface. Home creates “New
project”; a genuinely missing route path creates “Untitled Project” once. Other
load failures retain typed errors and Retry/Back controls. New/Open/Save As use
rfd's asynchronous native picker so the event loop keeps autosave ticking; paths stay native through the writer lease.

Header rename commits on Enter and cancels on Esc through `project.rename`.
The timeline restores its scale multiplier and logical-pixel horizontal scroll
through `project.set_view_state`, outside edit history; seeking and navigation save
the current view, deferring metadata writes until an active edit gesture finishes.
Playhead is restored through the existing seek path. Save/exit wait off-thread for engine acknowledgement; an error retains the
active editor. A window close is intercepted with `CancelClose`: save and close,
keep a flushed recovery copy and close, or cancel. New/recovered work requires
Save As. Loading can be cancelled without accepting its eventual result.

Startup discovers only abandoned journals and shows corrupt-candidate diagnostics.
Recovery publishes a new never-saved session journal before discarding the old
candidate; the original project is untouched. Other candidates remain deferred
while that recovered editor is active, and reappear after Save As/leave. Recovery
cannot replace an active editor outside its normal save/leave boundary. rfd 0.17.2 is MIT and is the one
native picker dependency. `tests/it/lifecycle.rs` exercises lifecycle boundaries and
real rename/close/recovery widgets without writing images.
Native picker interaction and Windows runtime need separate observed evidence.
