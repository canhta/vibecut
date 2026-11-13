# Attribute clipboard

Interface: `AttributeGroup` (re-exported by `session`). `attributes` is private; `clipboard.rs` owns the snapshots and re-exports `paste_attributes` to the edit owner. Transfer and custody rules: [clipboard](../README.md#clipboard).

`timeline.paste_attributes {items, source}` copies attribute groups from one copied item onto each target. Copy attributes is `clipboard.copy` of that one item: it shares the single item slot, so a copy of two or more items, or of mask points, refuses as an attribute source. `source` is `stored` or a one-item `payload`; a stored-empty paste is an exact no-op.

| Group | Carries | Held by |
|---|---|---|
| `Transform` | position, scale, rotation and reflection channels | video, text, overlay, graphic, sticker |
| `Blend` | the blend mode and the opacity channel | every kind except audio |
| `Effects` | the effect stack and its channels | every kind except audio |
| `Masks` | the masks and their channels | video, text, overlay, graphic |
| `Speed` | the whole `Retime` as one value | video, audio |
| `Audio` | volume, mute and fades, with their channels | video, audio |
| `Text` | the text style (never the words) and its colour, outline and background channels | text |

A group the copied item holds is replaced on the target as a unit: the target's channels of that group are removed first, so a group with no keys resets the target. Channels paste as model values with their keys unchanged; key times are item-local, so they stay relative to the target's start. Clip animations (In, Out, Combo) are ordinary keys on the transform and opacity channels and travel with those groups. Crop and item-parameter channels belong to no group.

A group the copied item holds and the target kind cannot hold is skipped, not refused; `Session::attribute_paste_skips` names them per target for the status line. Speed applies first through the timeline's speed rules (ripple on a magnetic main track, key stretch), so a changed duration that would overlap a neighbour refuses the whole paste. A locked target track, a missing target or a repeated target refuses before any change. One paste is one undo step.
