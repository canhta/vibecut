# Persistence and publication

Versioned project JSON, the writer lease, atomic files, recovery snapshots, optional metadata and bounded streaming output.

## Envelope and limits

`encode`/`decode` use one JSON envelope: `format`, `schema_version`, `generator`, `project`. The current schema is `SCHEMA_VERSION` (36). Older schemas return `TooOld` and newer ones `TooNew`; nothing is migrated or defaulted. Unknown envelope or project fields are refused.

ProjectId is stored as a canonical decimal string to keep all u64 bits for control clients; other IDs are numeric. Leases, hashes and receipts are not persisted in the envelope.

Ingestion and serialization cap JSON at 16 MiB, nesting at 64, individual raw JSON strings at 1 MiB and combined comma/colon separators at 2,000,000, including ignored fields. These aggregate budgets can be reached before model caps: 10,000 assets, 256 tracks, 100,000 total items and bookmarks, 100,000 cues per caption item with a shared style table of at most 1,024 distinct styles, 1 MiB per stored text. Exceeding a budget returns an error; nothing is truncated or repaired. JSON diagnostics expose line and column only. Raw serde use skips the ingestion budgets.

Read-only `load` reads at most 16 MiB + 1 byte, including when a file grows while reading. `WriterLease::save` serializes completely before touching the destination.

## Writer lease and save acknowledgement

`WriterLease::acquire(path)` takes an exclusive OS lock on the stable adjacent `.FILENAME.lock` before reading project bytes. Hold the lease from load through editing and saving. Drop unlocks explicitly before closing the handle, so descriptors inherited by concurrent subprocesses cannot extend the lock. `WriterLease::load` refuses a changed baseline; standalone `load` is for read-only ingestion. `acquire_wait(path, duration, cancelled)` polls contention and returns `Busy` on timeout or cancellation; cancellation is checked between native calls. Unsupported lock paths, permissions or OS locking return typed errors and never authorize an unlocked save.

Project data and writer control files have disjoint namespaces. Atomic publication and writer acquire/save-as refuse a basename that begins with `.` and ends with `.lock` (ASCII case-insensitive), including not-yet-created sidecars. Admission checks supplied and canonical names, so symlinks and directory or case aliases cannot publish project bytes over a lease. Windows also refuses trailing spaces and dots and alternate-stream names. Existing project files with reserved names stay readable; save them to an admitted destination.

A lease keeps its original lock object and verifies the current sidecar identity at acquisition and before and after bounded reads. Unix compares device/inode from the held descriptor. Windows opens without delete sharing and uses full 128-bit file IDs; where identity is unavailable the lease is refused. A missing, replaced, symlinked or differently identified sidecar returns `Conflict`, even when both are regular files with identical bytes. An invalidated lease cannot load, save or save-as.

`save(&mut self, project)` compares the current bounded file's full BLAKE3 content with the acquired or last-saved baseline, then stages through the atomic writer. After staging and sync it rechecks content and lease identity immediately before publication; a refused final precondition cleans staging and returns no receipt. A changed, removed or unexpectedly created file returns `Conflict`. A failed save keeps the baseline and issues no receipt. A successful `SaveReceipt` holds the canonical native path and the hash of the exact envelope bytes published. Engine binds its saved revision and still-current path to the receipt before acknowledging clean state; revision tracking and journal scheduling are engine's.

`Project::fork` copies editable content under a fresh random ProjectId and refuses on entropy failure without changing the source.

`save_as` acquires the destination while keeping the source lease, publishes a fork, then replaces the held lease only on success. Its receipt exposes the published `project_id`; the supplied snapshot and source file stay untouched. Saving to the held canonical path (including aliases) keeps the identity. `save_to(&self, &mut destination, project)` publishes to an already acquired destination while keeping the source; engine uses it to defer lease adoption until revision and receipt acknowledgement. Both refuse an invalidated source lease. Busy or failure preserves the source path and both files.

Existing symlinks resolve to the target, including symlinked parent directories; relative paths resolve once. Case and Unicode aliases follow the native filesystem's rules, including for adjacent lock files. Native `OsStr` names are kept without lossy conversion. The lock name adds six native characters, so near-limit filenames are refused rather than truncated into colliding locks.

The lock file is private on Unix, never truncated or removed, and holds no project data. Dropping the lease or killing its process releases the OS lock. Locks are advisory: content checks detect external edits present before publication but are not an atomic compare-and-swap against another application racing the final check and rename. Lock-file or directory replacement seen at a validation boundary invalidates the lease. Hard-link preservation is outside the atomic writer contract. Remote filesystem locking is unverified.

Managed-directory moves opt in with `WriterLease::enable_directory_moves(root)` before projects are published. It creates the permanent private control file `<root>/.vibecut-directory-moves.lock`, outside every movable child. Ordinary writer acquisitions beneath a child take shared admission on that file through adjacent-lock acquisition, then release it; independent open projects do not block each other. Acquisitions begun before the barrier was installed recheck admission after locking the sidecar. `WriterLease::move_directory(destination)` consumes a held project lease, takes exclusive admission, validates source, content and an absent sibling destination, closes the adjacent handle, and renames while still holding exclusive admission. New writers at either path receive Busy throughout; existing writers stay excluded by the adjacent project lease. Admission files share the reserved control namespace and must never be deleted or replaced. External-file projects without the opt-in use the ordinary adjacent-lock protocol.

A refused move leaves its source and the caller's history unchanged. An interruption before or after the atomic sibling rename leaves the complete directory at its source or destination. The library owns hidden-directory retention, recovery and permanent deletion. Physical power-loss durability and noncooperating replacement of directories or control files are outside this advisory protocol.

## Atomic files

`atomic::OwnedDirectory` retains native directory handles and named identity for a canonical root and one immediate child. Open resolves an intentionally supplied root alias once; child names and existing child objects must pass the public admission contract. A child retains and validates its root after the caller drops the root guard. Call `validate` before filesystem work and again at publication boundaries; a bare `path()` does not perform that check. Deeper nesting is refused instead of building recursive ancestry. The guard owns no contents, locks, control files, cleanup or publication protocol; its consumer owns those operations.

Unix compares device/inode from the retained descriptor and current name. Windows opens directory/reparse handles with backup semantics, shares read/write but excludes delete sharing, refuses all native reparse attributes, and obtains full high-resolution IDs through the existing safe `file-id` dependency. Native identity failure never yields a guard. Unix rename can invalidate the next check; Windows can prevent the rename itself. These are advisory operation boundaries, not arbitrary post-check race or physical power-loss guarantees. Native identity code is shared with the unchanged writer-lease protocol, not a second lease implementation.

`atomic_write` is the shared replacement primitive. It stages privately in the destination directory, writes, restores the previous file's permissions, syncs, then atomically replaces. Failed staging or publication cleans up and preserves prior bytes. Destination symlinks are refused; parent sync after publication is best effort. Ownership, ACLs, extended attributes and hard links are not preserved. Callers serialize writes to one target.

`stage_validated` stages bytes in the destination directory, lets the caller validate the complete synced staging file, then publishes with `atomic_write`'s admission and failure guarantees. A refused validation preserves the prior destination and removes staging. Background caches use it so unvalidated bytes never replace a destination.

`admit_destination` exposes the reserved-namespace and native filename admission to background publication consumers before they stage data. Source and active-project identity checks stay with the execution owner. `metadata_path` gives that owner the native sidecar name.

## Autosave snapshots

`autosave::save(path, project, keep)` writes into `<path>.autosave` with increasing 20-digit snapshot numbers and no clock dependency. Retention admits 1..=1000. Only canonical regular snapshot files participate; unrelated files, symlinks and directories are preserved. Enumeration caps at 10,000 entries. Publication never overwrites a snapshot and happens before pruning; pruning errors are returned while the complete new snapshot remains. Autosave directories cannot be symlinks. A crash can leave a temporary file, an extra complete snapshot or an incomplete pruning pass; no partial snapshot is promoted.

## Optional project metadata

`WriterLease::metadata` reads `<canonical project filename>.meta.json`, bounded to 512 KiB, metadata schema version 4. Missing, unreadable, corrupt, oversized or unsupported metadata defaults without blocking the project. The sidecar is outside the project envelope and does not change its schema.

`Metadata` holds created and modified Unix milliseconds, the per-project `proxy_preview` choice (preview reads cached proxies; presentation only, so the project file never names a proxy and export reads sources), the current `ViewState` and up to 256 scene-ID-keyed views (positive zoom at most 1000, horizontal scroll 0–1e9 logical pixels, nonnegative integer-tick playhead). Nonfinite values are refused. Thumbnail is Missing, Unavailable or Ready with 1..=128-pixel dimensions and exactly width × height × 4 sRGB RGBA bytes (at most 64 KiB). Invalid dimensions, byte counts and oversized sidecars default on read and refuse on write. No image decoder runs on metadata. No project index is stored here.

`read_metadata(path)` supports bounded read-only catalog display; mutations go through the WriterLease. `WriterLease::validate` checks the baseline and stable lock identity without requiring a project file to exist.

`WriterLease::save_metadata` validates, then atomically replaces the sidecar under the project lease and content precondition. Sidecar symlinks are refused. Project writer destinations reserve the `.meta.json` suffix, including case variants and canonical aliases, so saving a project cannot replace another project's metadata. A sidecar write error is returned and does not roll back a previously successful project publication. Callers acknowledge a save only after both writes succeed.

## Streaming output

`StreamingStage::new(destination, overwrite, max_bytes)` creates a private same-directory `.vibecut-export-*` temporary file. `writer()` yields the sole bounded seekable `StreamingWriter`, which is also media's mux-sink interface. Writes and seeks beyond the authorized maximum are rejected before touching bytes. `finalize()` syncs and restores prior permissions, yielding `FinalizedStage`; only that type exposes `publish()`. `Overwrite::Refuse` uses atomic no-clobber publication, including against a destination created after admission. `Replace` explicitly authorizes replacing the target.

Dropping or failing a stage removes its temporary file and leaves existing bytes untouched. A crash may leave an orphan temporary file; it is never promoted, reused or swept automatically, and the caller may offer cleanup after restart. Destination symlinks and nonregular files are refused. Callers serialize writes and close their mux writer before finalizing; cancellation arbitration belongs to their execution owner. Parent-directory sync after commit is best effort. Hardware power loss and racing parent replacement are unverified.
