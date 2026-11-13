# Audio edits

Interface: `dubbing`, crate-private to the edit module; its fit and ducking constants and
`FitInput` are re-exported by the edit module.

The edits that change what an item sounds like. `audio.rs` holds the fades, the master, the
cleanup chain and loudness.

| Module | Holds |
|---|---|
| `dubbing` | Narration takes: the duration-fit tiers, the overflow rule and the ducking automation. The rules are in [Dubbing](../README.md#dubbing). |
