# Sounds tab

Interface: `Sounds`, `SoundsView`, `SoundsMode`, `SoundsProblem`, `Client`, `CacheView`, and the `music` section (`Music`, `MusicView`, `MusicMode`, `MusicProblem`).

Consumes the libraries sound library. Discover searches the provider when a
credential exists, otherwise the saved local library; Saved always reads the local
library and works offline. Searches use the commercial-only filter by default; the
switch reruns the search without it. Results page 30 at a time and every result shows
author, duration, licence and precomputed attribution.

Save and unsave are local-library operations outside project history. Preview uses the
engine play/stop seam and an app-owned cpal sink on the retained preview worker,
capped at min(EOF, 120 seconds), including ordinary longer tracks. The continuous
resampler clips its crossing frame to the remaining stereo capacity and stops
packet/frame processing immediately at the cap, without draining the rest of the
song. Earlier EOF still drains the decoder/resampler. Encoded input must fit the
existing 32 MiB ceiling; staging that bounded input is separate from decoding its
prefix. Cancellation remains checked before conversion and during sample copying.
Play stays pending until CPAL's stream clock reaches the predicted DAC end of the
last samples, or stop/error; callback submission alone is not drain or proof of
physical audibility. Missing callbacks/device failure are typed NoAudioDevice.
One active worker and one newest pending selection are retained; cancellation
also covers stop before worker entry, and close stops and joins the worker.
The sink copies no pending replacement bytes, checks stop during decoding and
conversion, and holds no state mutex across IO. Encoded copies (32 MiB maximum)
and stereo f32 storage use the existing shared budget through stream destruction.
PCM pre-admits the full 120-second capacity at the actual 8–192 kHz device rate;
copied conversion planes are also charged. Cache reads and codec-internal native
buffers remain with their existing owners. Temporary input uses private
create_new files; collisions preserve the existing object, and cleanup removes
only a file successfully created by this operation. Native audibility needs
separate device evidence.

Add copies the cached preview into project media and
applies `sounds.add_to_timeline` as one undo step; an uncached add first runs the
shared cache job with progress and Cancel and adds only after the preview publishes.

Auth, quota, rate-limit, network, server, malformed-response, size, publication and
no-audio-device failures are typed states with creator copy; a missing token is a
local-only state. Engine's `RuntimeError::CacheBusy` retains the panel's `CacheBusy` category
when a shared download owner refuses an add; it leaves the project and pending add untouched.
The panel changes project JSON only through the add command.

## Music section

The Audio tab's Music page consumes the libraries music library. A section switch at
the top of the 120 px navigation column chooses Music or Sound effects; Music holds
Saved, then one chip per source tag. Discover answers anonymously from the open
source, Saved is the offline local library, and the free-text field sends `q` while a
chip sends `tags`. Rows are 48 px on the existing audio pitch: a 32 px cover with a
24 px play/pause overlay, the title, `artist · duration · licence`, the full
attribution, and 24 px bookmark (save) and add buttons. Every row's accessible label
and hover text also carry the source page. Add downloads into the local cache when
needed and applies `music.add_to_timeline` as one undo step; unsave and clear-all only
reach the cache. Provider, cache, publication, no-audio-device and cancelled failures
are typed states with creator copy; a failed download changes no project bytes.

A header disclosure lists the project's complete used-track credits from
`Project::music_credits` and copies that exact block; unused saved tracks are excluded.
The settings sheet applies or removes the optional source key for this process only,
using a masked field and a direct source-terms link; the key never enters project,
preferences, automation, undo or logs.
