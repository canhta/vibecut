# Voice-over capture

Interface: `InputHost`, `Opened`, `Sink`, `Recorder`, `TakeWriter`, `WriterFactory`, `Finished`, `Failure`, `CpalHost`, `QUEUE_BLOCKS`, `MAX_TAKE_SECONDS`.

One take from an input device to one promoted audio file. The dialog that drives it and the insertion at the playhead live in [`editor/dialogs`](../../../editor/dialogs/README.md).

## Seam

`InputHost` is the native boundary: `devices` lists input names (default first) and `open` starts capture on a named device or the default, handing the host a `Sink`. Production composes `CpalHost`; tests inject a scripted host that plays the audio callback, so no microphone, permission prompt or native app is involved. `Opened` carries the negotiated rate and channel count and a guard whose drop stops the device and releases the callback.

`CpalHost` builds, plays and drops the stream on one capture thread (the native handle never crosses threads) and waits at most five seconds for it. It reads f32, i16, i32 and u16 input (in that preference order), preferring 48 kHz and otherwise the device default; the callback converts to f32 straight into the queued block (`Sink::push`, `push_i16`, `push_i32`, `push_u16`), so conversion adds no allocation. A device offering none of these is `Unsupported`. Operating-system refusals map to `PermissionDenied`, `NoDevice`, `Unavailable` and `Unsupported`; a rerouted default device is not a failure. cpal 0.18.2 was already in the lockfile for playback and the sound preview.

## Queue and worker

`Sink::push` runs in the audio callback and never blocks: it records the peak for the meter, then `try_send`s whole-frame blocks of at most 16 384 samples into a queue of `QUEUE_BLOCKS` (64), so memory is bounded at about 4 MiB. A full queue sets `Overrun`; a take with a gap is never kept. `Sink::fail` is the stream's error callback.

One encoder worker owns the take writer (`TakeWriter`; the default wraps media's `AudioFileWriter`, and `Recorder::start_with` injects another, which a test gates to fill the queue by construction), downmixes to mono (voice), drops non-finite samples and writes the take into `<name>.m4a.part`. Time is the encoded sample count over the device rate, never a clock. A take ends by itself at `MAX_TAKE_SECONDS` (two hours, about 57 MB), which the dialog treats as Stop.

## Stop, cancel, promotion

`stop` drops the device guard, lets the worker drain and finish the file, then renames `.part` to the final name; the final name exists only for a complete, non-empty take. `cancel`, a failure and dropping the `Recorder` stop the device, join the worker and delete the unfinished file. Free names are `voiceover-N.m4a` in the caller's directory.

| Failure | Cause |
|---|---|
| `PermissionDenied` | the operating system refused microphone access |
| `NoDevice` | no input exists or the named one is gone |
| `Unavailable` | the device vanished, is busy or stopped mid-take |
| `Unsupported` | no f32, i16, i32 or u16 input format |
| `Overrun` | the queue filled; take discarded |
| `Storage` | the folder or file could not be created, written or promoted |
| `Encode` | the encoder refused the audio |
| `Empty` | stopped before any sample arrived |

Input latency from the device timestamps is not compensated; A/V alignment is exact in samples from the moment capture starts, and a native-device offset is unverified.
