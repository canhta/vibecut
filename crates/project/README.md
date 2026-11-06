# project

Headless project/timeline snapshots, invariant validation, `.vibecut` persistence,
atomic replacement and recovery snapshots. Editing commands/history belong to
engine; media owns `SourceAsset`; captions owns cue content and language tags.
Project also owns item-relative parameter/keyframe data, validation, evaluation and
time propagation. No codecs, widgets, device state, scheduling or effect execution.

| Module | Responsibility | Interface | Contract |
|---|---|---|---|
| `model/` | Owns the project, scenes, tracks, items and project settings. | `Project`, `ProjectData`, `Scene`, `Track`, `Item`, `Settings` | [Project model](src/model/README.md) |
| `animation/` | Owns item-relative parameters, keyframes, curves, fades and retime. | `Animations`, `Param`, `Keyframe`, `Channel`, `Fade`, `Retime` | [Animated parameters and retime](src/animation/README.md) |
| `visual/` | Defines persisted visual item properties and their bounds. | `TransformField`, `BlendMode`, `Mask`, `EffectInstance`, `Transition`, typography | [Visual properties](src/visual/README.md) |
| `persistence/` | Encodes, bounds, publishes and recovers project files. | `encode`, `decode`, `load`, `WriterLease`, `atomic_write`, `autosave` | [Persistence and publication](src/persistence/README.md) |

The crate root re-exports the same public names in the same order as before the
folder split; module paths inside the crate are private.

## Contracts

- [model](src/model/README.md) owns project, scene, track, item and settings data and
  whole-project validation.
- [animation](src/animation/README.md) owns animation paths, channels, keyframe
  evaluation, Bezier segments, clip fades and retime.
- [visual](src/visual/README.md) owns transform, blend, mask, effects, transition,
  bounds and typography contracts.
- [persistence](src/persistence/README.md) owns the JSON envelope, writer lease,
  atomic files, autosave snapshots, metadata and streaming output.

## Verification

`cargo xtask ci` runs model/invariant tables, serde and file round trips across
all item kinds and NTSC rates, hostile/deep/oversized JSON, NFC/Unicode paths,
missing-media state, hash-verified relink, save failure/cleanup, permissions,
symlinks and autosave retention. Public writer tests cover concurrent threads and
real processes, abrupt process death, stale-content refusal, save-as acquisition,
wait/cancel, receipts, relative/symlink/case aliases and native path refusal.
Namespace regressions retain the public Save As lost-edit reproduction, all ASCII
suffix case variants, aliased publication targets, same-byte sidecar replacement,
parent replacement, stale save-as refusal and hard-link alias noninterference.
Property tests generate adjacent integer-tick items and rational common rates and check exact persistence without drift.
`tests/it/retime.rs` pins rate, duration and source range at 0.01, 0.5, 1, 2 and 5x,
split/trim edge placement (table and random speeds/cuts), the unity-speed exact
tick behaviour, and persistence and refusal of retime fields.
A unit test injects failure after a partial staging write to verify original
bytes and cleanup. This establishes application-level behavior on the tested
filesystem, not survival of physical power loss. No performance claim is made.
