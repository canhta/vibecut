# Media library

Interface: `MediaView`, `MediaSort`, `MediaSortKey`, `MediaSortOrder`, `MediaKind`,
`next_sort`, `asset_name`, `ordered_assets`, `file_size`.

The Media tab is the live asset library. Grid and list share one persisted mode. The
sort dropdown orders by name, type, duration or file size, flips direction on the active
key, and persists key and direction in [preferences](../../../../preferences/README.md); sorting
is stable. File size is filesystem metadata for display and sort only (missing files
show none) and never enters the project record. Cards and rows reuse the
[import](../import/README.md) cache's decoded thumbnails and capability badges.

Click, Shift range over the displayed order, Cmd/Ctrl toggle and a blank-area marquee
share one asset selection with insert and delete; blank click or Escape clears it. The
context menu removes the whole selection through `media.remove` after confirmation.
In-use assets refuse with a typed reason listing the timeline items; unused removals are
undoable and restore the exact records. `media.reveal` (also the clip context menu's
Reveal media) switches to the Media tab, scrolls the asset into view and highlights it
for 1.5 s without touching project JSON. During an import batch the overlay reports
progress and disables view, sort, import and insert; cancel stays available.

The search box filters rows by case-insensitive name substring. The query is presentation
state only: it is session-scoped and never enters the project or preferences. No match shows
a Clear search row above the empty state, which restores the full list.

Export clips sends the selected assets to one registry `export.clips` job, after a folder
pick, with overwrite off. The context menu item is enabled only when every targeted asset
is video and no export runs; otherwise it is disabled with its reason. While the job runs a
strip above the results shows progress and Cancel (`job_cancel`). The finished, cancelled or
failed job reports once as a toast. A failure says finished clips stay in the folder and that per-clip results are not reported, since the job snapshot carries none. A folder the engine refuses reports as "did not start" with the engine's reason. Layout of the strip and clear row is open in docs/product/ui/editor.md Gaps.

A pending import row is emitted before probe and hash and stays transient until
validated metadata is added through the engine command. Audio and video follow probing
to ready or unsupported. Restored assets (including undo and redo) start as probing and
reopen capabilities in a background import job before timeline insertion; persisted
thumbnails come through the same content cache. Missing, changed or refused sources stay
blocked.

Media-owned ephemeral assets stay in the project for internal commands but are hidden
from rows and selection. Multi-selected assets insert through Add to timeline in one
undo step via engine auto placement; the engine owns track choice and creates secondary
tracks on demand. Insert synchronizes playback to the new snapshot.

Thumbnail retention and its leases belong to
[engine artifacts](../../../../../../../crates/engine/src/artifacts/README.md).

A non-generated asset whose source file is gone draws its tile with the missing body fill and one centred `Media missing` line in place of a thumbnail (`AssetRow::missing`, read from `VisualQuery::availability` while the rows are projected); the rest of the tile is unchanged. Measurements: [Missing media](../../../../../../../docs/product/ui/editor.md#missing-media).
