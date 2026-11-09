# Effect previews

`Arc<GpuEffects>::thumbnails(shared_budget)` builds the engine's bounded offscreen preview cache over the same GPU adapter, without a window. Bounds, cadence, cancellation, cache keys and typed failure: engine's [thumbnail contract](../../../../crates/engine/src/artifacts/README.md#effect-thumbnails).
