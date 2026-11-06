# Wire contract

## Wire contract

`session/wire` is the transport-neutral contract every transport shares; `control`
re-exports its consumer-facing items, so adapters import one `engine::control`
namespace. It owns `Limits` and the `MAX_*` budgets, `Revision`, the
`Request`/`Response`/`Event` envelopes and their error codes, the typed `ControlResult`,
bounded JSON, schema closing, command decoding and the bounded receipt/event ledger.
The feature result views a response carries (font entries, snapshot receipts, preview
frames, job views and the job id/kind/stage/state vocabulary) are defined here because
the registry embeds `ControlResult`'s schema in every command and the kernel stamps
events with the catalog digest; `fonts`, `snapshot` and `jobs` re-export them at their
existing paths. The ledger fields are `pub(crate)` so the control layer can admit, replay
and resynchronize while the kernel only records revision changes.

`parse_frame` accepts at most 8 MiB of UTF-8 with duplicate-key refusal, 32 nested
levels, 200,000 nodes, 100,000 members per collection and 4 MiB aggregate text.
Validators compile only generated local schemas with network/file resolution disabled.
Safe wire error codes omit paths, user text, provider bodies and Rust Debug;
`bounded_json` is the adapter's bounded response serializer.
API version 1 is independent of the project file schema. The BLAKE3 catalog digest
covers schemas, static effects/examples and limits, excluding enabled/session state.
[Generated commands](../../../../../docs/product/agent-control/commands.md) and `catalog.digest` regenerate
with `cargo xtask catalog`; the conformance test checks
both against current registry output in CI.

Every Session starts with an OS-random epoch. `revision()` is an opaque string;
its generation and `JobTarget` share the same Arc invalidation boundary. UI edits,
undo/redo, preview lifetimes, transaction rollback/net-zero end, view changes and
successful save advance it; ordinary no-ops/refusals do not. One typed `Payload` set
covers command result/error, committed project change, revision change and job
snapshots; envelopes include API/catalog/session identity, revision and string
sequence number.
Before publishing an admitted JSON/control edit, Session validates the candidate project's actual serialized
integers and created IDs, including values produced by allocation or tick/keyframe
arithmetic. Undo/redo validate their destination snapshot before moving history.
An invalid candidate returns InvalidParams without consuming IDs, advancing
revision or emitting a project change. This also applies to JSON execution; typed
in-process editing keeps the owner types' full integer domain.
