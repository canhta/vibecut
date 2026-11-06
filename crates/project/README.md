# project

Headless project/timeline snapshots, invariant validation, `.vibecut` persistence,
atomic replacement and recovery snapshots. Editing commands/history belong to
engine; media owns `SourceAsset`; captions owns cue content and language tags.
Project also owns item-relative parameter/keyframe data, validation, evaluation and
time propagation. No codecs, widgets, device state, scheduling or effect execution.

| Module | Responsibility | Contract |
|---|---|---|
| `model.rs` | Owns the project, timeline, scenes and item model. | [Model](src/model/README.md) |
| `bounds.rs` | Holds canvas, audio and volume bounds. | [Canvas bounds](src/bounds/README.md) |
| `typography.rs` | Defines text item typography. | [Text typography](src/typography/README.md) |
| `retime.rs` | Owns playback-rate mappings and split/trim edges. | [Retime](src/retime/README.md) |
| `transform.rs` | Owns transform units, defaults and clamps. | [Transform](src/transform/README.md) |
| `animation.rs` | Owns parameter overrides and keyframe evaluation. | [Animated parameters](src/animation/README.md) |
| `bezier.rs` | Defines scalar channels and Bezier segments. | [Bezier channels](src/bezier/README.md) |
| `fade.rs` | Owns clip fade windows and gains. | [Clip fades](src/fade/README.md) |
| `persistence.rs` | Encodes, decodes and bounds project JSON. | [Persistence and limits](src/persistence/README.md) |
| `atomic.rs` | Publishes files atomically and streams output. | [Streaming output](src/atomic/README.md) |
| `writer.rs` | Holds the writer lease and save receipts. | [Writer lease and save acknowledgement](src/writer/README.md) |
| `autosave.rs` | Writes rotating recovery snapshots. | [Autosave snapshots](src/autosave/README.md) |
| `metadata.rs` | Owns the optional project metadata sidecar. | [Optional project metadata](src/metadata/README.md) |
| `effects.rs` | Defines the item effect stack and parameters. | [Item effects](src/effects/README.md) |
| `mask.rs` | Defines item masks and their animation paths. | [Masks](src/mask/README.md) |
| `blend.rs` | Defines blend modes and opacity ownership. | [Blend selection and opacity](src/blend/README.md) |
| `transition.rs` | Defines video transition entities and bounds. | [Video transitions](src/transition/README.md) |

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
