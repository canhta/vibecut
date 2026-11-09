# Clip batch export

`ClipBatch::prepare(source_project, ClipBatchRequest)` owns one export job per accepted short. Desktop and CLI submit this same batch and add no renderer, cue mapper or retry loop. A short is an ordered `ai::clips::ClipSourceSegment` list in play order (`ClipShort::from_suggestion` keeps it), a title and an optional normalized crop. One request names the source asset, the source-timed `CueTrack`, the existing output directory, none or one or two distinct caption languages, sidecar formats (none, SRT, WebVTT or both), overwrite and degraded consent, and a per-artifact byte ceiling.

## Preparation

Preparation builds one derived immutable Project per short without touching media. Video items keep each piece's exact source range at cumulative output starts, so source gaps add no output duration. Audio comes from each piece's source audio. The output is 1080x1920 (9:16) with render's Cover fit: center crop by default, or the supplied normalized crop for every piece. Joining forces `ExportMode::Encode`.

Cues map through captions' `EditMapping` over the ordered `(piece range, cumulative output start)` pairs: fully cut cues drop, partial cues clip at piece edges, gaps remain gaps and play-order gaps never shift text. Each piece's caption item stores the same clipped source-coordinate cues, so burn-in and `subtitle_file` SRT/VTT read one mapping. A requested language missing from any selected cue refuses that short at preparation.

Output names derive from the NFC title. Characters illegal on Windows (`< > : " / \ | ? *`), controls and bidi overrides become `-`. Trailing spaces and dots are trimmed, reserved device names are prefixed, the base is limited to 200 UTF-8 bytes on grapheme boundaries, and duplicates get deterministic `-2`, `-3` suffixes. Outputs are `<base>.mp4` plus `<base>.srt` and `<base>.vtt`. Empty titles fall back to `clip-NN`. No user text appears in job errors.

`prepare` accepts 1..=1000 shorts, 1..=32 pieces per short, nonnegative nonoverlapping source ranges inside a known asset duration, one or two distinct languages, at most one of each sidecar format, a 1 byte..=1 TiB artifact ceiling and at most 256 MiB of prepared cue content across the batch. Request-level failures are `Invalid`, short-level failures are `EditError::ClipExport { short, reason }`, and a source asset without video is refused.

## Jobs and retry

`submit_one(runner, index)` enqueues one prepared short and refuses an out-of-range or already submitted index before touching the runner. `submit(&mut runner)` enqueues every remaining short in order, and a repeated `submit` adds nothing. The runner owns concurrency and queue bounds. `job_id` and `snapshot` expose per-short progress, `cancel` and `cancel_all` stop queued or running work, and `retry` resubmits a failed or cancelled short with its prepared inputs (a completed, running or evicted job is refused). Retry reuses the prepared snapshot, so relinking a source requires a new batch.

Sidecars are staged through project's StreamingStage before the MP4 and published only after render's MP4 publication. A prepublish failure or cancellation leaves no new artifact and every published file is complete. A publication failure after the MP4 is reported but cannot roll back the complete MP4, and an existing MP4 is preserved.

Each short keeps publication progress across retries. `retry` skips artifacts this short already published (checking they still exist), republishes only missing ones under the original overwrite policy, and re-renders the MP4 when it is gone. It never treats an artifact this batch did not publish as skippable, so a sidecar failure after the MP4 retries without deleting the MP4 or granting blanket overwrite. Sidecar-only retries seal cancellation through the same publication fence, so a late cancel cannot relabel committed output as Cancelled.

Missing or changed sources return render's typed `Unavailable` entries per job, and the batch does not degrade without `allow_degraded`. Storage exhaustion and the byte ceiling surface as typed IO or project failures from the bounded stages, leaving previous destinations and no staging orphan. Every artifact path is validated through render's shared output admission against the retained immutable original project, not only the selected source, so exporting into any original project source path or hard link is refused even with overwrite permission.
