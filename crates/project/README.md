# project

Headless project and timeline snapshots, invariant validation, `.vibecut` persistence, atomic replacement and recovery snapshots. Also owns item-relative parameter and keyframe data, validation, evaluation and time propagation.

Engine owns editing commands and history. Media owns `SourceAsset`. Captions owns cue content and language tags. No codecs, widgets, device state, scheduling or effect execution.

| Module | Owns | Contract |
|---|---|---|
| `model/` | Project, scenes, tracks, items, settings | [Project model](src/model/README.md) |
| `animation/` | Parameters, keyframes, curves, fades, retime | [Animated parameters and retime](src/animation/README.md) |
| `visual/` | Persisted visual item properties and bounds | [Visual properties](src/visual/README.md) |
| `persistence/` | JSON envelope, writer lease, atomic files, autosave, metadata, streaming output | [Persistence and publication](src/persistence/README.md) |

The crate root re-exports the public names; module paths are private.

Verification is in `cargo xtask ci`. Writer and atomic-file tests establish application-level behaviour on the tested filesystem, not survival of physical power loss. No performance claim is made.
