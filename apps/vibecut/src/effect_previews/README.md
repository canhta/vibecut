# Effect previews

Interface: `EffectPreviews`, `EffectPreviewStatus`, `preview_texture`.

`EffectPreviews::new(cache, budget)` owns one job runner and one entry per effect kind:
`ensure` requests the first preview for a kind; `retry` cancels its old token and
requests one replacement. QueueFull alone leaves a Pending entry owning its bounded
EffectInstance. `tick` consumes completed outputs before admitting deferred requests
in FIFO order, stopping at queue saturation; permanent failures cannot block the next
kind. The runner keeps queue4, retained4 and one GPU-class worker. A missing retained
job becomes an explicit Failed tile. `settle` includes deferred requests in its existing
deadline, and Drop cancels and joins through the owned runner. Other refusals stay Failed.
`GpuEffects::thumbnails(budget)` builds the engine's bounded offscreen cache over the
playback GPU adapter without a window; `cpu()` uses the reference CPU compositor for
headless panels. Bounds, cadence, cancellation, cache keys and typed failure are the
engine's [thumbnail contract](../../../../crates/engine/src/artifacts/README.md#effect-thumbnails).
