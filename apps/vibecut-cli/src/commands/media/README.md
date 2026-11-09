# Media commands

`normalize <project> [--clip] [--master] [--target-lufs -14] [--true-peak-db -1]` sets those [loudness targets](../../../../../crates/engine/src/analysis/README.md#loudness-normalization) (master by default), runs the measurement job with progress on stderr, stores the gains and prints the measured clips and master with their gains. `export` applies them and refuses stale gains after an audible edit until `normalize` runs again.

`scenes` composes the engine's shared scene detector and ordinary jobs/cache and returns candidates without a project edit. Syntax and output belong to the [CLI contract](../../../../../docs/product/agent-control/cli.md#scene-detection); thresholds, limits and reviewed marker application belong to [engine](../../../../../crates/engine/src/analysis/README.md#scene-detection).

`media waveform <project> --asset <id>` and `media thumbnails <project> --item <id> --pixels-per-second <zoom> [--start <tick> --end <tick>]` warm the engine's `media.generate_waveform` and `media.generate_clip_thumbnails` jobs and print the cached summary layout or the cached source positions. They load the project under its writer lease, never edit it and open only the `waveform` and `thumbnails` caches under the default VibeCut cache root.
