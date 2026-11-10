# Project lifecycle

Interface: `EditorSession`, `EditorAction`, `fresh`.

`EditorSession` wraps the sole engine `Autosave`; the memory variant serves headless
editor injection. The workspace owns loading, exiting, error and recovery presentation
and sends blocking open, discovery, save, flush and discard work through engine's job
runner. New, Open and Save As use rfd's asynchronous native picker (the one picker
dependency) so autosave keeps ticking; paths stay native through the writer lease.
Loading can be cancelled without accepting its eventual result.

Header rename commits on Enter and cancels on Esc through `project.rename`. The timeline
restores scale multiplier and logical-pixel horizontal scroll through
`project.set_view_state`, outside edit history; seeking and navigation save the view,
deferring writes until an active edit gesture ends. The playhead restores through the
seek path.

Save and exit wait off-thread for engine acknowledgement; an error keeps the active
editor. Window close is intercepted with `CancelClose`: save and close, keep a flushed
recovery copy and close, or cancel. New or recovered work requires Save As.

Startup discovers only abandoned journals and shows corrupt-candidate diagnostics.
Recovery publishes a new never-saved session journal before discarding the old candidate
and leaves the original project untouched. Other candidates stay deferred while the
recovered editor is active and reappear after Save As or leave. Recovery cannot replace
an active editor outside its normal save or leave boundary.
