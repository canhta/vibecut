# Media library

The Media tab is the live asset library. Grid and list views share one persisted
mode; the sort dropdown orders by name, type, duration or file size, flips the
active key between ascending and descending, and persists key plus direction in
the versioned preference store. Sorting is stable, so equal keys keep project
order. File size is filesystem metadata read for display only (missing files show
no size); the project record stays unchanged. Cards and rows reuse the import
cache's decoded thumbnails and capability badges, with type, duration, size and
probe fields. Click, Shift range over the
displayed order, Cmd/Ctrl toggle, and a blank-area marquee share one asset
selection with insert and delete; a blank click or Escape clears it. The context
menu removes the whole selection through the registered `media.remove` command
after a confirmation dialog. In-use assets refuse with a typed reason listing the
timeline items; unused removals are undoable and restore the exact asset records.
`media.reveal` (also the timeline clip context menu's Reveal media) switches to
the Media tab, scrolls the asset into view and highlights it for 1.5 s without
touching project JSON. During an import batch the overlay reports progress and
disables view, sort, import and insert; cancel stays available.

A pending import row is emitted before probe/hash execution and remains transient
until validated metadata is added through the engine command. Audio and video
follow the same probing -> ready/unsupported presentation. Restored assets
(including undo/redo) default to probing and reopen capabilities in a background
import job before timeline insertion; persisted thumbnails are requested through
the same content cache. Missing, changed or refused sources remain blocked.

Queue and editor retain at most 64 shared thumbnail handles each, charged by
engine `RetainedThumbnail` for CPU/upload/GPU storage. Undo prunes both owners;
entry limits and aggregate budget pressure evict images and textures together.
Import retries work admission after dropping retained thumbnails on pressure.
The batch work allowance is 448 MiB, covering bounded native decoding, codec
scratch, packet/probe buffers, hashing and handoff; retained images have separate
leases. Media owns the stricter thumbnail/recovery geometry and worker bounds.
