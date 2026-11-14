# Store

Interface: `library`, `recovery`, `luts`; project-cover capture is private. `ProjectLibrary::open` and `Autosave::new`/`open` take the lent footage lookup that covers render with: `Lookup`, a function from a project to its `render::Corrections`, defined in the cover owner and re-exported as `footage::Lookup`, or `footage::uncorrected()` without a media cache.

## Imported LUT bundles

Shared native copy/verify and staging operations return the runtime resource result, preserving
IO/Invalid/Limit/budget/cancel and lower Kernel validation/entropy causes. Pure provenance
record validation returns Kernel EditError. Store-owned open/quota/import/reuse/relink and
publication consume `luts::Error` / `luts::Result`: transparent Edit/Runtime lower causes,
owned IO (`job/cache IO failed`) and render (`render execution failed`) sources. Only direct
Runtime composition forwards transience and scheduling policy; foreign render cancellation
and explicit Failed stay opaque. Higher workflows compose through `engine::control::Error::Lut`.
The artifact import callback also bounds JSON and keeps its real mixed Control result;
repair/reuse matches actual direct helper errors without peeling wrappers.

`luts::LutStore` owns durable copied user resources under the process media root's `luts/<ProjectId>/<LutId>` directory. Ordinary artifact eviction never removes them. One store lease and per-content publication leases serialize publishers; quota admission uses no mutex across IO. Admission counts at most 4096 filesystem entries, 1024 bundles and 256 MiB of retained plus pending bytes. Staging conservatively reserves a complete 40 MiB-plus-metadata bundle, releases on every failure/cancel/unwind, and may retain a complete unregistered resource after a stale/evicted result. There is no destructive automatic garbage collector.

Each synced directory contains canonical cube, the original source bytes (`original`, `.cube` or `.3dl`) and bounded version-1 provenance. BLAKE3 of canonical bytes is the resource ID; original source hash/name/licence are separate first provenance. Matching semantic imports reuse it; explicit conflicting metadata refuses. A matching-ID relink may supply different raw formatting: copied-source hash records the actual replacement while first provenance remains intact. Damaged resources require an explicit matching relink, foreign entries and symlinks refuse, and no source/project/metadata file is overwritten. Native paths are lossless. Publication seals job cancellation only after validation; complete-directory rename is atomic, with the existing filesystem owner's limits on hostile replacement races and power loss.

Managed publication copies and verifies all three files into `media/luts/<hash>`, rewriting only the bundle path; duplicate/history/trash retains that directory. Save As retains immutable resource references, as media does, and does not promise a self-contained package. Moving the selected original is harmless; loss/change of the copied canonical file is a typed render failure, repaired only with matching canonical content.

## Managed project library

`library::Error` / `library::Result` own the complete managed catalog family. Lower Kernel
refusals compose transparently through Edit, runtime resource failures through Runtime, and
direct IO/media keep their safe static wrapper diagnostics and original source. Classification
delegates to the declaration owner; only direct Runtime composition forwards its transience
and scheduling policy. Failed or foreign media cancellation remains opaque. Higher workflows
compose through `engine::control::Error::Library` at their own mixed boundary.

`library::ProjectLibrary::open(root, budget)` uses an explicit application-supplied native
directory, independent of external-file open and save. It holds one exclusive catalog lease;
another handle returns Busy. A managed project lives in
`<root>/<16-digit lowercase hex ProjectId>/project.vibecut`, with its project-owned metadata
sidecar and private `media/<16-digit hex AssetId>` files. There is no index database: `list()`
derives name, main-scene duration, timestamps, 8-character lowercase hex ID suffix and thumbnail
from those owners. IDs keep all 64 bits; the suffix is display-only and must never identify a
command target. The listing admits 1024 projects, 4096 root entries and 16 MiB of retained name
and thumbnail payload. A managed directory that cannot be read (older schema, damaged or foreign) never fails `list()`: it is skipped there and returned by `list_unreadable()` as an `UnreadableProject` (full ID, path and the innermost cause as its reason), so it neither hides other projects nor blocks create, rename or duplicate; its ID stays taken.

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

Library-local history keeps 32 rename and duplicate steps for the handle lifetime.
An empty undo/redo queue composes the kernel's `SessionError::NoHistory` before changing
history or touching disk; [Session](../session/README.md) owns its refusal category. Undoing a
duplicate moves its complete directory into hidden history storage; redo restores those same
bytes and copied media. Closing the handle or running a new command discards abandoned redo
copies. Unexpected project content changes refuse undo, and open-project writer leases refuse
catalog mutations. Delete is not undoable and clears library history: the adapter must obtain
confirmation before sending the command.

Delete moves the complete project directory to a hidden sibling `.trash-<16-digit hex ID>`,
so project, metadata, media and their links stay together; a store-owned `trash.json` records
the Unix milliseconds of the move and is published before the rename through the atomic record
owner, so it never truncates a hard-linked project or media inode and a failed move leaves the
entry active. Trash is not part of the active listing: `list_trash()` derives its rows
from each trashed directory's own project file and record. A trashed project cannot be opened
or edited, and neither name nor identity is released: a project that reuses the same ID makes
restore refuse with `Conflict` rather than overwrite. Restore moves the directory back to its
ID directory and removes the record, so media links work again; a second restore or a purge of
an entry that is no longer trashed refuses. Purge moves the directory to a unique `.delete-*`
tombstone and removes it, which is the only permanent deletion. A missing or corrupt record
keeps an entry listed but untimed, so nothing is purged without an authoritative move time.
An unavailable wall clock refuses a new move instead of recording an epoch, and it disables
automatic expiry for that open.

Trash keeps entries for `RETENTION_DAYS` (30) whole days, measured from the recorded move and
presented by `ProjectEntry::trash_days_left`. `ProjectLibrary::open` purges expired entries
before it lists anything: each candidate passes the same regular-file, project-identity and
writer-lease admission and the same tombstone move as an explicit purge, so a corrupt,
mismatched or busy entry stays complete and is retried on the next open. An entry that cannot
be removed stays listed at zero days. Failed post-purge cleanup can leave an unlisted hidden
directory; it is never promoted or reused.

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

### Shorts runs

A run of the shorts window is an ordinary managed project whose `shorts_run` is set ([project model](../../../project/src/model/README.md#shorts-run)); no second store exists. `list` and `list_trash` never return runs, `list_runs` returns only them, so Home neither shows nor trashes them. `save_run(project, poster)` publishes the project by the same staged rename as every entry but keeps the source file where it is instead of copying media: a run holds only suggestions and cues, and its source may be hours of video. A moved source is reported when the run reopens. The poster is the first short's frame in the metadata thumbnail. `MAX_RUNS` (24) bounds the kept runs; saving one more removes the least recently saved. `remove_run` accepts only a run and deletes then purges it (no Trash entry); it never touches exported files. `load_run` reads one run read-only. The composition root lends the library (see the [app](../../../../apps/vibecut/src/app/README.md)); calls block.

`project.thumbnail` captures the active scene at output tick zero through the production CPU
compositor, at most 128 pixels per edge, independent of playhead or preview buffers. Rendering
failure, including a denied allocation, stores Unavailable, for which adapters show a
placeholder. Autosave backfills Missing on open, refreshes on saves, and accepts the same command
outside content history. Desktop exit invokes it even for a journal-only close. The project
sidecar owns the bounded raw RGBA bytes; no compressed image decoder runs during catalog reads.
