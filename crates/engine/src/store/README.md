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
`project.duplicate { id }`, `project.delete`, `project.restore`, `project.purge`,
`project.thumbnail`, `undo` and `redo`. The target is the full `ProjectId`; create and
duplicate supply the new full ID. Rename refuses blank or colliding names. Duplicate keeps
the exact project content and name except the new ProjectId and copied-media paths, and
starts fresh metadata timestamps: it streams and hashes every asset into a private sibling
directory, checks content identity, syncs files, writes the project and sidecar, then
publishes the whole directory by rename. Sources must be regular files; missing or changed
files, IO failure, ID collision or cancellation leave no visible partial project. Copying
admits at most 1 TiB per project and uses 64 KiB streaming buffers. `execute_cancellable`
checks the caller's predicate before commands, between copy chunks and before publication.
Calls block and belong off the UI thread. Noncooperating filesystem replacement races and
physical power-loss durability are outside this contract.

Library-local history keeps 32 rename and duplicate steps for the handle lifetime. Undoing a
duplicate moves its complete directory into hidden history storage; redo restores those same
bytes and copied media. Closing the handle or running a new command discards abandoned redo
copies. Unexpected project content changes refuse undo, and open-project writer leases refuse
catalog mutations. Delete is not undoable and clears library history: the adapter must obtain
confirmation before sending the command.

Delete moves the complete project directory to a hidden sibling `.trash-<16-digit hex ID>`,
so project, metadata, media and their links stay together; a store-owned `trash.json` records
the Unix milliseconds of the move and is written before the rename, so a failed move leaves
the entry active. Trash is not part of the active listing: `list_trash()` derives its rows
from each trashed directory's own project file and record. A trashed project cannot be opened
or edited, and neither name nor identity is released: a project that reuses the same ID makes
restore refuse with `Conflict` rather than overwrite. Restore moves the directory back to its
ID directory and removes the record, so media links work again; a second restore or a purge of
an entry that is no longer trashed refuses. Purge moves the directory to a unique `.delete-*`
tombstone and removes it, which is the only permanent deletion. A missing or corrupt record
keeps an entry listed but untimed, so nothing is purged without an authoritative move time.

Trash keeps entries for `RETENTION_DAYS` (30) whole days, measured from the recorded move and
presented by `ProjectEntry::trash_days_left`. `ProjectLibrary::open` purges expired entries
before it lists anything; an entry that cannot be removed stays listed at zero days and is
retried on the next open. Failed post-purge cleanup can leave an unlisted hidden directory;
it is never promoted or reused.

Undo, redo, delete, restore and purge use the project-owned
[directory-move barrier](../../../project/src/persistence/README.md#writer-lease-and-save-acknowledgement).
The library enables it before publishing entries; it excludes new project writers across the
adjacent-handle close and atomic sibling rename on Windows. Redo also leases and validates its
hidden source. Existing targets (even empty directories) and missing sources refuse without
advancing history or listing state. An interrupted move leaves one complete source or hidden
destination, including project, metadata and media. After restart, hidden directories stay
unlisted and untouched; recovery can inspect their project identity and restore a complete
`.undo-*` directory to its absent ID directory while the library is closed. `.delete-*`
directories from a purge's cleanup reflect confirmed permanent deletion and are never
restored, and recovery never promotes a `.trash-*` directory: Trash is listed only through
`list_trash()` and restored through `project.restore`. macOS behaviour of the directory-move
barrier requires platform verification.

`project.thumbnail` captures the active scene at output tick zero through the production CPU
compositor, at most 128 pixels per edge, independent of playhead or preview buffers. Rendering
failure, including a denied allocation, stores Unavailable, for which adapters show a
placeholder. Autosave backfills Missing on open, refreshes on saves, and accepts the same command
outside content history. Desktop exit invokes it even for a journal-only close. The project
sidecar owns the bounded raw RGBA bytes; no compressed image decoder runs during catalog reads.
