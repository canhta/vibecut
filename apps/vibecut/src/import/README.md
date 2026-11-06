# Media import

Media import uses one native multi-file picker or a file drop. One batch is capped
at 128 paths (32 KiB each), must admit a 448 MiB probe/work lease from the shared
byte budget, and publishes results through a bounded 8-event channel, so a paused
UI applies backpressure instead of accumulating metadata. Probing and hashing run
on an engine import job with one cooperative cancellation token carried into
media's FFmpeg probe interrupt and 64 KiB hash reads; each successful file becomes
an asset immediately, so a later refusal or cancel keeps earlier assets. Published
imports refresh playback against the updated snapshot, including ratcheted FPS. The media
panel omits media-owned ephemeral assets from rows and selection while retaining
them in the project for internal commands. Cards show a filename and duration;
probe type, resolution and audio format are available on hover. The panel can multi-select assets,
and an "Add to timeline" action that inserts only the selected assets in one undo
step through engine auto placement. The engine chooses compatible tracks and
creates secondary tracks on demand; the app owns no placement policy. While a batch runs, an overlay shows completed/total
progress and a 2 px accent bar, disables import/insert, and offers Cancel. Partial
failures use short recovery copy and close with a concise toast; typed probe
details remain in diagnostics. A terminal runner failure stays a typed
error and never reports success. Insert synchronizes playback against the new
immutable project snapshot. Image placement remains with its later feature
ticket.
