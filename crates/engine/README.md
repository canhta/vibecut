# engine

Headless project editing, bounded background execution, persistent sessions and media workflows.
Domain crates own processing algorithms, decoded/raster storage and persisted project formats.
Widgets, device state, app preferences and authenticated transports live in consumers.

`submit_scene_exports_with_sources` admits portable `render::OutputOptions`, resolves all scene
snapshots and runs them as one cancellable job. Every scene file is finalized in private
temporary storage before user output publication begins; cancellation or rendering failure
removes private files and preserves destinations. Publication uses the job fence and per-file
atomic publication. A filesystem failure while publishing several outputs can leave earlier
complete outputs; it never publishes a partial file. The caller owns project-file protection and
the output grant authorizes every generated path. Control API version 2 requires
`export.settings` with this typed contract.

| Folder | Responsibility | Contract |
|---|---|---|
| `session/` | Owns editable project state and the wire contract. | [Session](src/session/README.md) |
| `session/edit/` | Applies validated editing commands to project candidates. | [Edit commands](src/session/edit/README.md) |
| `session/wire/` | Defines the transport-neutral wire contract. | [Wire contract](src/session/wire/README.md) |
| `runtime/` | Executes bounded background work. | [Runtime](src/runtime/README.md) |
| `runtime/cache_storage/` | Retains content-addressed execution artifacts. | [Artifact cache](src/runtime/cache_storage/README.md) |
| `runtime/staged_publication/` | Publishes admitted output artifacts. | [Completed export publication](src/runtime/staged_publication/README.md) |
| `store/` | Persists managed project sessions. | [Store](src/store/README.md) |
| `store/recovery_workflow/` | Owns recoverable persistent sessions. | [Session recovery and save acknowledgement](src/store/recovery_workflow/README.md) |
| `subtitles/` | Produces source-timed subtitle output. | [Subtitles](src/subtitles/README.md) |
| `transcript/` | Acquires source-timed cues from the user's chosen providers. | [Transcript](src/transcript/README.md) |
| `analysis/` | Measures source media. | [Analysis](src/analysis/README.md) |
| `analysis/shot_analysis/` | Produces conservative source-time shot analysis. | [Per-shot analysis](src/analysis/shot_analysis/README.md) |
| `artifacts/` | Produces derived project artifacts. | [Artifacts](src/artifacts/README.md) |
| `providers/` | Resolves explicit AI provider choices, their consent and the local models. | [Providers](src/providers/README.md) |
| `control/` | Admits transport-neutral commands; the top layer, the only one that calls every feature. | [Control](src/control/README.md) |

The dependency table is owned by [structure](../../docs/engineering/structure.md#engine-cratesenginesrc)
and `xtask/modules.toml`.

Entry points: use [Session commands](src/session/README.md#public-entry-points) for edits,
[control admission](src/control/README.md#control-admission-revisions-and-receipts) for transport adapters,
[the job runner](src/runtime/README.md#background-jobs-and-byte-policy) for bounded work and
[the subtitle workflow](src/subtitles/README.md#subtitle-workflow) for speech-to-caption composition.
