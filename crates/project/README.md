# project

Headless project and timeline snapshots, invariant validation, `.vibecut` persistence, atomic replacement and recovery snapshots. Also owns item-relative parameter and keyframe data, validation, evaluation and time propagation.

Engine owns editing commands and history. Media owns `SourceAsset`. Captions owns cue content and language tags. No codecs, widgets, device state, scheduling or effect execution.

| Module | Owns | Contract |
|---|---|---|
| `model/` | Project, scenes, tracks, items, settings | [Project model](src/model/README.md) |
| `animation/` | Parameters, keyframes, curves, fades, retime | [Animated parameters and retime](src/animation/README.md) |
| `visual/` | Persisted visual item properties | [Visual properties](src/visual/README.md) |
| `preset.rs`, `preset/` | Built-in text and caption looks: shelves, our names and the style values they copy; data only, applied by `engine` | `src/preset/text.rs`, `src/preset/caption.rs` |
| `visual/effects/` | Effect parameter model, effect catalogue, filter looks | [Effects](src/visual/effects/README.md) |
| `persistence/` | JSON envelope, writer lease, atomic files, autosave, metadata, bundles, streaming output | [Persistence and publication](src/persistence/README.md) |
| `persistence/bundle/` | Bundle directory export and import, with media and LUT copies | [Project bundles](src/persistence/bundle/README.md) |

The crate root re-exports the public names; module paths are private.

Verification is in `cargo xtask ci`. Writer and atomic-file tests establish application-level behaviour on the tested filesystem, not survival of physical power loss. No performance claim is made.
