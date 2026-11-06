# Media commands

`normalize <project> [--clip] [--master] [--target-lufs -14] [--true-peak-db -1]`
sets those [loudness targets](../../../../../crates/engine/README.md#loudness-normalization)
(master by default), runs the measurement job with progress on stderr, stores the gains
and prints the measured clips and master with their gains. `export` then applies them;
it refuses stale gains after an audible edit until `normalize` runs again.

`scenes` composes the engine's shared scene detector and ordinary jobs/cache,
returning candidates without a project edit. Syntax and output are owned by the
[CLI contract](../../../../../docs/product/agent-control/cli.md#scene-detection); thresholds, limits and
reviewed marker application belong to [engine](../../../../../crates/engine/README.md#scene-detection).
