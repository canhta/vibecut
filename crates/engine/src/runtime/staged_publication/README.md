# Completed export publication

`sanitize_file_name` is the shared portable name sanitizer used by the desktop export dialog. It normalizes NFC, replaces forbidden characters, trims trailing dots and spaces, escapes reserved device stems case-insensitively even with extensions, and limits output to 255 UTF-8 bytes without splitting a grapheme. Empty export names use `Untitled Project`. `default_file_name` reserves the extension inside that limit. Snapshot names keep their `-` replacement and `snapshot` fallback, with space reserved for the timecode and PNG suffix.

The `publication` module owns admission and staged writing for completed exports and snapshots. It admits the captured project sources and the adapter's protected active project and metadata paths before staging and again after sync, then seals cancellation before publishing through project's atomic writer. Canonical paths (including case-folded missing names on Windows), hard links, live source content identity and protected directories refuse project and source aliases even when a source changed since import. The atomic writer also refuses reserved writer-control destinations. Refusal or storage failure preserves the previous destination and removes staging files.

`submit_export_publication` copies a retained completed render through this owner with a bounded 64 KiB buffer on the job runner. No admission, copy or sync runs on the UI thread.

`submit_sequence_publication` does the same for a finished PNG sequence directory: it admits the destination as above, copies the frames in name order under `<destination name>_%05d.png` into render's `sequence_stage`, and promotes the directory with one rename after the same fence. An earlier sequence of that name is replaced; a directory holding other files refuses whole.

Cancellation before the fence and storage failures preserve the prior destination and the completed input. After the fence a committed destination reports Completed. The adapter retains the input until the job terminates and joins the runner before cleanup on shutdown. Individual filesystem calls cannot be interrupted; external processes racing path replacement are outside the atomic owner's guarantees.

Publication admission, normalization and same-file resource checks return Runtime results.
Render and Media preserve their original typed sources/static wrapper diagnostics and reuse
Runtime execution's foreign projections; Project validation composes through Kernel Edit.
Only direct Runtime cancellation/transience delegates scheduling policy; foreign cancellation
and Failed remain opaque. Higher workflows compose guard failures at their own boundary.

The existing writer operation accepts a caller error E:From<RuntimeError> and returns that
same E. Writer callback refusal is forwarded unchanged; only admission/stage/writer/finalize/
hook/publish guards convert through Runtime into E. Pure PNG byte writers consume the Runtime
result, while mixed sidecar/export workflows retain their actual caller result and source.
