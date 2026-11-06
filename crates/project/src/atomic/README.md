# Streaming output

`atomic_write` is the shared replacement primitive for later preferences/bundles.
It stages privately in the destination directory, writes, restores the previous
file's permissions, syncs, then atomically replaces. Failed staging/publication
cleans up and preserves prior bytes. Destination symlinks are refused; parent
sync after publication is best effort. It does not preserve ownership, ACLs,
extended attributes or hard links. Hardware power-loss durability and Windows
runtime behavior remain unverified. Callers serialize writes to one target.

`atomic::StreamingStage::new(destination, overwrite, max_bytes)` creates a private
same-directory `.vibecut-export-*` temporary file. `writer()` yields the sole
bounded seekable `StreamingWriter`, including media's mux-sink interface. Writes
and seeks reject positions beyond the authorized maximum before touching bytes.
`finalize()` syncs and restores prior permissions, yielding `FinalizedStage`;
only that type exposes `publish()`. `Overwrite::Refuse` uses atomic no-clobber
publication, including a competing destination created after admission.
`Replace` is explicit authorization to replace the target at publication.

Dropping or failing a stage removes its temporary file and leaves existing bytes
untouched. `AtomicFs` injects sync/publication failure for boundary tests. A crash
may leave an orphan temporary file; it is never promoted, reused or automatically
swept. The caller may present cleanup after restart. Destination symlinks and
nonregular files are refused. Callers serialize writes and close their mux writer
before finalizing; cancellation arbitration belongs to their execution owner.
Parent-directory sync remains best effort after commit: there is no fallible
post-publication result. Hardware power loss, racing parent replacement and
Windows runtime behavior remain unverified.
