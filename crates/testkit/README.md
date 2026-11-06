# testkit

Dev-only support for tests and examples: synthetic media fixtures, process-private
generated-fixture directories and external FFmpeg/FFprobe oracle access. Product
code never depends on this crate; it is admitted only as a `[dev-dependencies]`
entry and owns no runtime behaviour.

| Module | Responsibility | Interface |
|---|---|---|
| `source` | Builds synthetic source assets and streams. | `asset`, `stream`, `video`, `audio`, `AssetBuilder`, `StreamBuilder` |
| `fixture` | Owns process-private generated-fixture directories and atomic publication. | `dir`, `publish` |
| `oracle` | Discovers and drives external FFmpeg/FFprobe oracles. | `command`, `available`, `require`, `generate`, `ffmpeg`, `probe`, `rgba`, `stream_field`, `decoded_frames`, `duration`, `decodes`, `approx` |

Fixtures live under `target/fixtures/<area>/<pid>/`; a process locks its folder for
its lifetime and prunes unlocked siblings, so runs never share a partially written
source. `oracle::generate` and `fixture::publish` write a temporary sibling and
rename it into place, so readers never observe a partial file. Missing tools are
explicit skips unless `VIBECUT_REQUIRE_ORACLES=1`.

The crate is test support, so it allows the panic shortcuts the workspace denies
product code (`unwrap_used`, `expect_used`, `panic`, `indexing_slicing`).
