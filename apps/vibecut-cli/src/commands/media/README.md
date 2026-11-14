# Media commands

`normalize <project> [--clip] [--master] [--target-lufs -23] [--true-peak-db -1]` sets those [loudness targets](../../../../../crates/engine/src/analysis/README.md#loudness-normalization) (master by default) with `Command::SetLoudness`, runs `audio.measure_loudness` through the registry with progress on stderr, stores the gains with `Command::ApplyLoudness` and prints the measured clips and master with their gains. `export` applies them and refuses stale gains after an audible edit until `normalize` runs again.

`scenes` composes the engine's shared scene detector and ordinary jobs/cache and returns candidates without a project edit. Syntax and output belong to the [CLI contract](../../../../../docs/product/agent-control/cli.md#scene-detection); thresholds, limits and reviewed marker application belong to [engine](../../../../../crates/engine/src/analysis/README.md#scene-detection).

`media waveform <project> --asset <id>` and `media thumbnails <project> --item <id> --pixels-per-second <zoom> [--start <tick> --end <tick>]` warm the engine's `media.generate_waveform` and `media.generate_clip_thumbnails` jobs and print the cached summary layout or the cached source positions. They load the project under its writer lease, never edit it and open only the `waveform` and `thumbnails` caches under the default VibeCut cache root.

`export-clip <project> --clip <id> --mode <clip|duration> --out <path> [--overwrite]` takes the engine's `clip_export_range` for the clip's span, submits `submit_clip_export` and waits for the job; it never edits the project.

`export-clips <project> --asset <id> [--asset <id>...] --dir <dir> [--overwrite]` plans one full-source export per asset with the engine's `plan_asset_exports`, submits them as one `submit_asset_exports` job and waits for it. It reads the project and writes only the named files; the output rules are in the [CLI contract](../../../../../docs/product/agent-control/cli.md).
