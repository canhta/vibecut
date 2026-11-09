# Media commands

`normalize <project> [--clip] [--master] [--target-lufs -14] [--true-peak-db -1]` sets those [loudness targets](../../../../../crates/engine/src/analysis/README.md#loudness-normalization) (master by default), runs the measurement job with progress on stderr, stores the gains and prints the measured clips and master with their gains. `export` applies them and refuses stale gains after an audible edit until `normalize` runs again.

`scenes` composes the engine's shared scene detector and ordinary jobs/cache and returns candidates without a project edit. Syntax and output belong to the [CLI contract](../../../../../docs/product/agent-control/cli.md#scene-detection); thresholds, limits and reviewed marker application belong to [engine](../../../../../crates/engine/src/analysis/README.md#scene-detection).
