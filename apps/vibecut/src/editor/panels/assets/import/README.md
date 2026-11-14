# Media import

Interface: `ImportQueue`, `ImportEvent`, `ImportFailure`, `ImportProbe`,
`ThumbnailOutcome`, `ThumbnailSource`, `MAX_IMPORT_FILES`.

Import takes one native multi-file picker or a file drop. A batch is capped at 128
paths of 32 KiB each, must admit a 448 MiB probe and work lease from the shared byte
budget, and publishes through a bounded 8-event channel so a paused UI applies
backpressure. Probing and hashing run on an engine import job with one cancellation
token reaching media's FFmpeg probe interrupt and 64 KiB hash reads. Each successful
file becomes an asset immediately, so a later refusal or cancel keeps earlier assets.
Published imports refresh playback against the new snapshot, including ratcheted FPS. A
terminal runner failure is a typed error, never success.

While a batch runs, an overlay shows completed over total, the probe job's within-file
fraction and a 2 px accent bar; it disables import and insert, and offers Cancel.
Partial failures use short recovery copy and a concise toast; typed probe details stay
in diagnostics. A batch that admits nothing and fails every file shows one line,
"Couldn't import this file. It may be damaged.", as a 3 s toast at the top of the media
panel, and adds no tile. The command itself returns once the batch is admitted; the typed
probe error arrives as that batch's `ImportFailure`.

Cards, sorting and insertion: [media library](../media_panel/README.md).

Thumbnail results retain the Artifact owner; the media adapter recognizes its direct Runtime
pressure causes before its existing cache-miss fallback. Shared cache setup stays Runtime-owned.
