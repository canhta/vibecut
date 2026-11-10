# Effect previews

Interface: `EffectPreviews`, `EffectPreviewStatus`, `preview_texture`.

`EffectPreviews::new(cache, budget)` owns one job runner and one entry per effect kind:
`ensure` admits the first preview for a kind, `retry` cancels and resubmits, and `tick`
retains only terminal status. A refused submission is a Failed tile, not a blank.
`GpuEffects::thumbnails(budget)` builds the engine's bounded offscreen cache over the
playback GPU adapter without a window; `cpu()` uses the reference CPU compositor for
headless panels. Bounds, cadence, cancellation, cache keys and typed failure are the
engine's [thumbnail contract](../../../../crates/engine/src/artifacts/README.md#effect-thumbnails).
