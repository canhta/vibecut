# Item effects

## Item effects

`effect_definitions()` exposes the admitted effect names and scalar parameter bounds,
defaults and animation capability for UI consumers. `Item.effects` is an ordered stack of at most 64 visual `EffectInstance`s. Instance
IDs are item-local ASCII animation identifiers (1–64 bytes); duplicates refuse.
`EffectParams::Blur` stores finite intensity 0–100 (default 15); `enabled` bypasses
an instance. `EffectParams::ColorGrade` stores finite brightness, contrast,
saturation and hue, each -50..=50 with 0 identity (controls and ranges are new, not
measured parity; `GRADE_MIN`/`GRADE_MAX`); it has no animation paths. Audio items refuse visual effects. Evaluation/device work belongs to
render/the desktop app. Animated overrides use the existing `Animations` map and
`Param` storage at `effects.<instance-id>.params.intensity`, with scalar channels
only. No second keyframe store is introduced. Removing an effect removes its
animation paths; undo restores both. Schema 4 adds the required stack and refuses
older envelopes through the existing typed version errors.

`EffectParams::set_scalar` validates a registered scalar key and value before
replacing the static parameter; unsupported keys and invalid values retain the
previous parameter. Definition metadata and typed parameter validation share the
project owner.
