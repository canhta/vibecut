# Motion edits

Interface: `animation` and `keyframes`, crate-private to the edit module.

The edits that write an item's keyed channels.

| Module | Holds |
|---|---|
| `keyframes` | Keyframe commands: set an animated parameter, add, toggle, remove, retime and delete keys, easing, handles, extrapolation and time scaling. |
| `animation` | Built-in In, Out and Combo clip animations expanded into ordinary keys on the channels they own, and their refit when a clip is trimmed. |
