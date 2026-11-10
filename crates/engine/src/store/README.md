# Store

Interface: `library`, `recovery`; project-cover capture is private.

## Managed project library

`library::ProjectLibrary::open(root, budget)` uses an explicit application-supplied native
directory, independent of external-file open and save. It holds one exclusive catalog lease;
another handle returns Busy. A managed project lives in
`<root>/<16-digit lowercase hex ProjectId>/project.vibecut`, with its project-owned metadata
sidecar and private `media/<16-digit hex AssetId>` files. There is no index database: `list()`
derives name, main-scene duration, timestamps, 8-character lowercase hex ID suffix and thumbnail
from those owners. IDs keep all 64 bits; the suffix is display-only and must never identify a
command target. The listing admits 1024 projects, 4096 root entries and 16 MiB of retained name
and thumbnail payload; corrupt managed entries return errors instead of disappearing.

`execute(target, Command)` handles registered `project.create`, `project.rename`,
`project.duplicate { id }`, `project.delete`, `project.thumbnail`, `undo` and `redo`. The target
is the full `ProjectId`; create and duplicate supply the new full ID. Rename refuses blank or
colliding names. Duplicate keeps the exact project content and name except the new ProjectId and
copied-media paths, and starts fresh metadata timestamps: it streams and hashes every asset into
a private sibling directory, checks content identity, syncs files, writes the project and
sidecar, then publishes the whole directory by rename. Sources must be regular files; missing or
changed files, IO failure, ID collision or cancellation leave no visible partial project.
Copying admits at most 1 TiB per project and uses 64 KiB streaming buffers. `execute_cancellable`
checks the caller's predicate before commands, between copy chunks and before publication. Calls
block and belong off the UI thread. Noncooperating filesystem replacement races and physical
power-loss durability are outside this contract.

Library-local history keeps 32 rename and duplicate steps for the handle lifetime. Undoing a
duplicate moves its complete directory into hidden history storage; redo restores those same
bytes and copied media. Closing the handle or running a new command discards abandoned redo
copies. Unexpected project content changes refuse undo, and open-project writer leases refuse
catalog mutations. Delete is permanent, not undoable, and clears library history: the adapter
must obtain confirmation before sending the command. Failed post-delete cleanup can leave an
unlisted hidden directory; it is never promoted or reused.

Undo, redo and delete use the project-owned
[directory-move barrier](../../../project/src/persistence/README.md#writer-lease-and-save-acknowledgement).
The library enables it before publishing entries; it excludes new project writers across the
adjacent-handle close and atomic sibling rename on Windows. Redo also leases and validates its
hidden source. Existing targets (even empty directories) and missing sources refuse without
advancing history. An interrupted move leaves one complete source or hidden destination,
including project, metadata and media. After restart, hidden directories stay unlisted and
untouched; recovery can inspect their project identity and restore a complete `.undo-*`
directory to its absent ID directory while the library is closed. `.delete-*` directories
reflect confirmed permanent deletion and are never restored. macOS behaviour of the
directory-move barrier requires platform verification.

`project.thumbnail` captures the active scene at output tick zero through the production CPU
compositor, at most 128 pixels per edge, independent of playhead or preview buffers. Rendering
failure, including a denied allocation, stores Unavailable, for which adapters show a
placeholder. Autosave backfills Missing on open, refreshes on saves, and accepts the same command
outside content history. Desktop exit invokes it even for a journal-only close. The project
sidecar owns the bounded raw RGBA bytes; no compressed image decoder runs during catalog reads.
