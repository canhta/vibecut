# Effect previews

Effect thumbnail consumers use `Arc<GpuEffects>::thumbnails(shared_budget)` to
construct the engine's bounded offscreen preview cache. It schedules the same GPU
adapter without a window; source selection and Effects panel presentation belong
to their UI consumer. Engine's [thumbnail contract](../../../../crates/engine/src/artifacts/README.md#effect-thumbnails)
owns bounds, cadence, cancellation, cache keys and typed failure.
