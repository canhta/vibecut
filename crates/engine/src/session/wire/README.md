# Wire contract

`session/wire` is the transport-neutral contract every transport shares; `control` re-exports its consumer-facing items. It owns `Limits` and the `MAX_*` budgets, `Revision`, the `Request`/`Response`/`Event` envelopes and their error codes, the typed `ControlResult`, bounded JSON, schema closing, command decoding and the bounded receipt/event ledger. The feature result views a response carries (font entries, snapshot receipts, preview frames, job views and the job id/kind/stage/state vocabulary) are defined here because the registry embeds `ControlResult`'s schema in every command and the kernel stamps events with the catalog digest; `fonts`, `snapshot` and `jobs` re-export them. The ledger fields are `pub(crate)` so the control layer can admit, replay and resynchronize while the kernel only records revision changes.

`parse_frame` accepts at most 16 MiB of UTF-8 with duplicate-key refusal, 32 nested levels, 200,000 nodes, 100,000 members per collection and 4 MiB aggregate text. Validators compile only generated local schemas with network and file resolution disabled. Safe wire error codes omit paths, user text, provider bodies and Rust Debug. `bounded_json` is the adapter's response serializer, capped at 16 MiB. Preview jobs retain their separate 4 MiB result reservation and serialization ceiling.
Caption workflow result bounds belong to [subtitles](../../subtitles/README.md#caption-workflow-commands).

API version 18 (7 added the `blur.intensity` animation channel and the Blur In/Out animation presets, 8 the `matte_strokes` item field, 9 the clip candidate `label`, 10 the `remove_flicker` item field and `footage.analyze` kind `flicker`, 11 the 0.1-100x speed range, 12 replaced the text stroke `width` and the shadow `offset_x`, `offset_y` and `blur` with `thickness`, `opacity`, `blurriness`, `distance` and `angle`, and the caption outline `width` with `thickness`, 13 added `media.relink`, 14 moved the `dub.generate` consent into its job, so a denial ends the job as a permission failure instead of refusing the command, 15 the speed-curve control points and the Montage, Hero, Bullet, Jump cut, Flash in and Flash out speed presets, 16 the `plan_required` error code for a voice behind a paid plan, 17 the `auth`, `quota`, `rate_limit` and `network` codes for every provider failure that used to report `ai`, 18 the chroma key `feather_edge` and `clean_up_edge` effect parameters and its Intensity default of 20) is independent of the project file schema. The BLAKE3 catalog digest covers schemas, static effects, examples and limits, excluding enabled and session state. [Generated commands](../../../../../docs/product/agent-control/commands.md) and `catalog.digest` regenerate with `cargo xtask catalog`.

Every Session starts with an OS-random epoch. `revision()` is an opaque string; its generation and `JobTarget` share the same Arc invalidation boundary. UI edits, undo/redo, preview lifetimes, transaction rollback or net-zero end, view changes and successful save advance it; ordinary no-ops and refusals do not. One typed `Payload` set covers command result/error, committed project change, revision change and job snapshots; envelopes include API, catalog and session identity, revision and a string sequence number.

Before publishing an admitted JSON/control edit, Session validates the candidate project's serialized integers and created IDs, including values produced by allocation or tick/keyframe arithmetic. Undo/redo validate their destination snapshot before moving history. An invalid candidate returns InvalidParams without consuming IDs, advancing revision or emitting a project change. Typed in-process editing keeps the owner types' full integer domain.

`ControlError` owns source-free protocol refusals, including unknown commands, invalid
parameters and command limits, as well as schema/session/revision, permission, receipt
and resynchronization failures. JSON budgeting, wire-integer validation, frame parsing,
bounded serialization and command-reference generation return this narrow owner error;
command decoding retains its mixed result because edit-budget checks also refuse domain
operations. `control` re-exports it; the top Control error composes it transparently.
This Rust boundary does not change serialized error codes or the catalog.

Pure editing declares `EditError` at the Kernel edit owner, including lower project/time/
caption classification and static source wrappers. Mixed `engine::control::Error::Edit` delegates;
wire categories and schemas do not encode either Rust enum namespace.

`Failure` states code and retry eligibility at the error's declaration owner.
Its nonserialized `JobFailurePolicy` preserves direct terminal cancellation and direct
AI retry-advisory seconds for the scheduler. Ordinary owners default to neither;
Runtime Failed and recovery persistence wrappers do not inherit either policy.
The transparent root Runtime composition forwards its owner's policy. Engine enums
classify exhaustively; engine wrapping variants classify foreign errors. Runtime `Failed` delegates
its code recursively, as does recovery's erased persistence cause. Delegating a code does not
make a wrapper transient: only direct network and selected AI failures qualify, subject to
runtime's existing retry/publication gates. Refined failure classes change envelopes and the
catalog digest within the current API; consumers use the current digest.

`ControlResult::Clipboard` returns the nullable bounded keyframe snapshot only on read
responses. Static catalog limits include clipboard keys, paths, items and compact bytes;
the aggregate explicit payload bytes in an edit tree share the keyframe or item cap
before candidate cloning.
Copy revision and lifetime rules: [clipboard](../README.md#clipboard).

Job admission carries bounded `word_highlight_warnings` from the captured project and selected caption languages. Preview and burn use render’s shared eligibility and shaped layout; warning entries expose only numeric item/cue positions, a language tag and safe reason, never caption text. Disabled or repaired state recomputes to an empty list. Project persistence and caption correspondence belong to their lower owners.
