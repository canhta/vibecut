# Source cleanup

`CleanupChain` runs one item's `project::AudioCleanup` over its source samples inside the mix graph. The chain holds one stage per enabled effect with a nonzero mix; each stage keeps its biquad state per channel and continues it across contiguous blocks of the same item. A block that does not start where the previous one ended restarts the state.

The RBJ section designs and their zero-latency guarantees are in [audio](../README.md#source-cleanup). `biquad.rs` is private to this folder.
