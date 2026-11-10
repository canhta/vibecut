# Effect previews

Interface: `EffectPreviews`, `EffectPreviewStatus`.

`EffectPreviews::new(cache, budget)` owns one job runner and one entry per effect kind:
`update(visible, Editor.clock.now())` advances a two-second30fps integer-frame demo,
independent of project/playhead/transport. The first visible frame is0; hidden elapsed
time is excluded. Hiding cancels submitted work and clears deferred requests; returning
starts fresh generations at the paused coordinate. Existing app frames/repaint drive it.
Each kind owns its bounded effect/animations, one job, one coalesced latest sequence and
one last good image. Time never cancels slow work; current-generation completions present
in increasing unwrapped sequence order. Changed inputs/retry/visibility invalidate old IDs.
FIFO admission preserves older deferred kinds while replacing their desired time in place.
QueueFull alone defers; other failures and missing retained IDs stay explicit Failed.
Ready images remain visible during refresh. The owner updates the existing texture once
per presented image; stationary paints reuse it. `settle` freezes its current target,
including refresh/deferred work behind Ready, and observes the existing deadline without
sampling a clock. Drop cancels and joins. Queue4/retained4, one GPU worker,320×180 demo,
16MiB default CPU budget and engine cache bounds are unchanged.
`GpuEffects::thumbnails(budget)` builds the engine's bounded offscreen cache over the
playback GPU adapter without a window; `cpu()` uses the reference CPU compositor for
headless panels. Bounds, cadence, cancellation, cache keys and typed failure are the
engine's [thumbnail contract](../../../../crates/engine/src/artifacts/README.md#effect-thumbnails).
