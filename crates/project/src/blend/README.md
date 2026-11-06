# Blend selection and opacity

## Blend selection and opacity

`Item.blend_mode` persists as required `blendMode` using the 17 kebab-case IDs in
`BlendMode::ALL`. Normal is the creation default; audio items refuse a non-normal
blend. The render owner documents the linear-light formulas. Opacity has one
owner: the existing scalar `opacity` animation channel. Missing channels mean
unity; constant channels and keyframed channels share persistence and trim/retime.

`atomic::admit_destination` exposes the existing reserved writer-control/native
filename admission to background publication consumers before they stage data.
Source and active-project identity checks remain with the execution owner.
`metadata_path` supplies that owner with the native sidecar name.
