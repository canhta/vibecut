# Edit commands

Interface: the registered `Command` variants with `Effect::Edit` and their parameter types
`Placement`, `TrackPlacement`, `TrimEdge`, `ItemParamPatch`, `Move` and `NewItem`; `apply` and
`check_budget` are crate-private.

Per-command fields, effects, availability and examples are the declaration in
[`session/commands.rs`](../commands.rs) and the generated
[command reference](../../../../../docs/product/agent-control/commands.md). This page holds
only the rules that span commands.

## Import policy

`color.import_lut` is a Human source-path CPU job through the process media access object. It publishes a durable descriptor, never a Session edit. The caller checks its captured session/revision/item before `color.register_lut` plus grade/reference/strength edits in one Batch; `Session::apply_lut` composes that consumed batch for desktop and CLI. Registration validates structure, deduplicates exact descriptors and refuses conflicting first metadata. Render verifies bytes when resolving a reference. Invalid registration/application, stale revision, locked/missing item or an open transaction leaves the project/history unchanged; a complete unregistered bundle can remain within the store quota. Ordinary edits may select already registered references without granting arbitrary file access.

`import.add_asset` and every inserted visual item raise project FPS to the greatest video-stream
rate, compared as exact rationals, and never lower it. Audio and image sources, including
animated images, do not change FPS. A raised rate that cannot produce exact positive canvas
ticks refuses the command atomically ([`ratchet_fps`](media_edits.rs)).

The first visual insertion while every scene timeline is empty and `original_canvas_size` is
null sets canvas width and height, the `original` preset, the last custom size and that original
size; later inserts keep it, including after deleting all items or switching scenes. Batches
execute in caller order, so simultaneous first inserts have one deterministic winner. FPS,
canvas and insertion share one history step ([`execute_insert_item`](clips.rs)).

## Timeline placement

`Placement::Auto` uses the unlocked main track for video when magnet is enabled or main is
empty, choosing the nearest clip edge at the requested start (ties before) and inserting
without replacing content; otherwise it takes the first unlocked compatible track whose
requested half-open range is free, in main, overlay then audio order, and creates a secondary
track at the top of its group when none fits. An explicit `track` refuses overlaps and
incompatible kinds. A first insertion onto empty main clamps to zero. The caller supplies every
linked partner explicitly: there is no implicit link model, and snapping is a caller decision.
Negative starts and checked arithmetic overflow refuse the whole edit, including track and ID
allocation ([`placement_track`](tracks.rs), [`placement_start`](clips.rs)).

## Transient selection and shared fields

`timeline.set_item_param` and `timeline.set_item_param_many` apply one typed `ItemParamPatch`.
Text content and font patches copy at most the control owner's `MAX_TEXT_BYTES` budget per
command tree, including nested batches. The single-item command refuses an incompatible target;
the many-item command skips incompatible kinds. Missing IDs, duplicates, invalid values and
locked compatible items refuse the whole edit ([`parameters.rs`](parameters.rs)).

`timeline.set_flipped` sets `transform.flipX` or `transform.flipY` on every listed active-scene item as one candidate. Every target is resolved before any write: an empty/duplicate list, missing or other-scene ID, locked track, audio/caption item or an axis carrying keys refuses the whole edit without a partial change. Video, image, text, overlay and graphic items are admitted. Absent and equal-false states are exact no-ops; the held Boolean is an ordinary zero-key parameter, so duplicates and splits clone it, trims/splits translate its keys and save/reopen preserves it.

`timeline.align_items {items,alignment,at}` and `timeline.distribute_items {items,axis,at}` translate `transform.positionX/Y` on every listed active-scene item as one candidate. `at` is the shared output clock; each item samples its real transform at `clamp(at - start, 0, duration)` so a non-overlapping selection still has one reference time. The envelope is the four transformed destination corners after positive scale, reflection, clockwise rotation and translation in project canvas pixels; captions' saved cue layout and audio have no box, and a transient render placement refuses the edit rather than aligning a different rectangle. Six alignments move each item individually against the canvas. Distribution requires three or more items, sorts by leading edge then ID, pins the first leading edge and last trailing edge, and refuses a negative gap. Every position base and scalar key is shifted by the same delta with key times, segments, handles and extrapolation preserved; absent channels become constant overrides, and raw or shifted values outside the admitted range refuse instead of clamping. A near-zero move is an exact no-op; all other changes are one undo step.

`Session::set_selection` validates existing IDs, breaks merge continuity and changes no project
content, revision or history length; a change during a gesture or group refuses. Undo and redo
restore the selection that produced each snapshot, and deleted IDs are pruned. Scope selections
(mask points, keyframes) are app-owned.

## Edit semantics and limits

Every command builds a private candidate, validates it through `Project::new` and publishes one
snapshot. A refusal or unwind restores project, history, selection, merge state, ID counter and
pending change ([`apply`](../edit.rs), [rollback](../state/README.md#rollback)). One command is one
undo step; a batch or group is one step, and a failed child rolls the whole batch back.

Typed commands admit at most 1024 commands and nesting depth 16; each collection is capped at
100,000 entries and text at 4 MiB. JSON dispatch additionally caps depth at 32 and nodes at
200,000 before deserialization ([`check_budget`](../edit.rs), [limits](../wire/limits.rs)).

Locks protect item structure and item properties: item edits, insertion, moves into or out of a
track and track removal refuse a locked track; `track.set_flags`, `track.rename`,
`track.reorder` and `track.set_locked` remain permitted on one, so it can be unlocked
([`tracks.rs`](tracks.rs)).

Ripple policy is independent of app preferences and is a command, not a method:
`timeline.edit {commands, ripple}` supplies it for a whole batch (a typed caller wraps one atomic
command the same way). A typed entry may also wrap exactly one `timeline.preview` to give a
replacement preview the policy; the wire refuses that shape like any session control inside a
batch. Nested policy wrappers refuse, and `timeline.set_ripple` sets the session default (off
initially, no project or history entry, refused during a transaction). One post-command diff
closes same-track tails vacated by delete, tail trim or leftward movement; it never shifts
another track, and cross-track moves, splits and hide/mute leave followers unchanged.

The main-track magnet (`timeline.magnet`, `track.set_magnet`; default on) runs the same pass
without the session flag for a command whose leaves are only `delete`, `timeline.split_left` and
`timeline.split_right` (a `Batch` of them included) and whose listed items include one on the
main track ([`magnet_scope`](clips.rs)). The pass covers exactly the tracks those listed items
sit on: a removed main item ripples main, and a listed partner on another track closes its own
tail; unlisted tracks never shift. Magnet off, a command with no main-track target, or any other
command leaves the gap as before. A split that keeps both pieces vacates nothing, so it is never
closed, whatever the tracks: `split_at` and any split over every track keep their pieces in place
with the magnet on. Splitting left drops the left piece and the right piece takes the cut start;
splitting right keeps the left piece and the vacated right span closes. The edit and its shifts
publish as one undo step ([`close_vacated_tails`](clips.rs), [session](../README.md#transactions-gestures-and-changes)).

`EditError` and `AssetUsage` are declared in this owner and re-exported through Session
and the crate. Its Result covers pure editing: safe-ID/static refusals and actual lower
ControlError, SessionError, project, time and caption sources. Project IO/busy/conflict/media
refinements are classified here, as are the static Project/Time/Captions wrapper diagnostics;
higher engine workflows delegate through `engine::control::Error::Edit`. Commands perform no IO or retries.

## Text animation

`text.apply_animation` expands one built-in look once into ordinary item-relative
keys on one unlocked active-scene Text item. Names, not source parity: Fade animates
`opacity`; Slide travels 25 % of the project canvas width along `transform.positionX`;
Scale shrinks to half the item's current `transform.scale` on both axes. In/out/both
directions share one 300 ms edge, capped at floor(duration/2), measured in integer
ticks. Scalar bases come from the item's existing channels or their documented
defaults and are preserved; linear keys have no handles and Hold extrapolation.
Both coalesces only equal-time middle keys. A title must last at least two canvas
frames. Any keyed preset channel is accepted only when the complete requested Param
is already identical; otherwise undo or remove keys before applying another look.
Applying a preset whose family owns no keys leaves unrelated transform keys alone.
Locks refuse even an exact repeat. Trim, split, move and key-time scaling retain
ordinary key semantics, without edge binding or a saved preset identity. Text
media-speed editing remains unsupported.

## Text presets

`text.save_preset` copies the selected Text item's appearance, never its words or
keys, into the project-wide 64-entry bank under a unique NFC name.
`text.apply_preset` copies one saved style onto one or more unlocked active-scene
Text items as one atomic edit; an empty, duplicate, missing, non-text or locked
selection refuses every item and leaves history unchanged. The app names its
save action `Style N` (the first free N); command callers pass any valid name.
Preset names and values are new product behaviour, not reference parity. See
[model](../../../../project/src/model/README.md#saved-text-styles) for validation.

## Caption styles

`captions.apply_style` chooses the whole appearance of one active-scene caption item. A built-in
caption preset or a complete `CueStyle` materializes values into the existing cue-style table,
including cues temporarily hidden by a trim, and clears any live selection. A saved preset instead
stores `{preset, overrides}` on the item: the project bank stays the single source of its full
style and a sparse patch overrides individual fields. Both forms preserve text, timing, provenance
and animation. Explicit materialized apply replaces imported per-cue differences; undo restores
them. Locks refuse even an equal request, and an empty item refuses a materialized value. An
unchanged selection or an all-equal materialized table leaves history unchanged. Input types and
looks live in [`commands.rs`](../commands.rs); text serialization and rendering retain their
existing owners.

## Video transitions

Clip edits revalidate the affected timeline before publishing: transitions follow moved cuts,
clamp after shortening and disappear when adjacency or handles are lost. A split extends the
ordered side pieces while keeping the span; explicit trim, move and delete detach pieces that no
longer form a contiguous chain reaching the cut. Failed edits publish neither item nor
transition changes. Transitions are video-only; [project](../../../../project/src/visual/README.md#video-transitions)
owns the admitted kinds, parameter definitions and the invariant mathematics. `transition.set_param` replaces one parameter in a single undo step and refuses an unknown id, a value of another type or an out-of-range value without publishing.

`captions.save_preset` saves one complete full style. A selected item saves its live preset value with
its sparse overrides folded in; otherwise the authoritative source cues, including hidden trim
sources, must be uniform and their imported value is completed with presentation defaults. Empty,
unstyled or mixed sources refuse. Save does not style any item. `captions.rename_preset` and
`captions.delete_preset` edit project metadata, including with no caption target or all tracks
locked. Same-name rename is an exact no-op; duplicate names refuse. Deletion refuses while any
caption item in any scene selects the preset, so no live reference ever dangles; materialize a
different style first. Rename never changes appearance.

## Reframe

`reframe.set_manual {item,at}` finds the shot containing `at` — the span from the last `crop.auto` key at or before it — and replaces that key's value with false while keeping every `crop.x`/`crop.y` key. `reframe.clear {item}` removes the three crop paths and leaves the static crop. Both are one undo step and refuse a locked track, a missing item or a shot time outside the item. `reframe.apply {job,replace_manual}` is executed by control dispatch because it needs the runner and the media cache: it merges the job's emitted keys shot by shot, keeps a shot whose `crop.auto` value is false unless `replace_manual`, then commits the static crop and the three channels as one `Command::Batch`, one undo step. The [reframe analysis](../../analysis/reframe/README.md) owns key emission and that shot merge.

## Graphic appearance

Generated rectangles and ellipses insert through `timeline.insert` and ordinary Video placement, without initializing source canvas/FPS. Graphic fill/stroke/width/alignment/radius patches update stored fallbacks only; they preserve any authoritative animation Param. Ellipses refuse radius. Ordinary parameter commands validate every graphic base and key through the project candidate before publication. Generic masks, effects, trim/split/move/duplicate and key-time scaling retain their normal history and refusal rules; media speed remains unsupported.

`masks.set_path` replaces one existing mask's shape with a closed 3..=4096-anchor path and preserves every other field and its animations; an equal path is an exact no-op. `masks.add_point` inserts one de Casteljau anchor on a closed segment at `2^-20..=1-2^-20`, appending on the closing edge, and `masks.remove_point` removes unique current indices in one undo step while refusing fewer than three survivors. These commands refuse a non-path target, an open saved path or out-of-range input without publishing.

Keyframe Paste uses this same candidate/validation/one-undo path. Item Paste resolves
relative tracks, clears overlapped ranges with owner-side split/trim edits and inserts
fresh ids in the same candidate. Transfer and refusal
rules belong to the [clipboard contract](../README.md#clipboard).

The private edit allocator returns `SessionError` directly. Callers compose it into
mixed edit results; it neither wraps the counter nor changes allocation admission.
ID retirement and checkpoint rules belong to [Session](../README.md#public-entry-points).

`captions.set_word_highlight {item,enabled}` changes only an active-scene caption item’s opt-in. The ordinary locked-item guard runs even on equal requests; no-op, undo/redo, atomic batch and receipt/grant admission remain the shared command rules. It never changes styles, source text, verified correspondence or authoritative hidden trim cues.

`captions.select_emphasis {item,cue_ids,mode}` stores up to three keyword word ids per named cue in `CaptionParams.emphasis`; `mode` is `lexicon` (default), the deterministic local selection of `captions::select_lexicon`. `captions.set_emphasis {item,cue_id,word_ids}` replaces one cue's ids; an empty list clears it. Ids are indices into the cue's verified `CueWordTiming::Words` records, ascending, unique and in range, so a selection is always a subset of timed words. A cue without verified word timing (or whose source text no longer matches it) refuses the whole command with `captions::Error::NoWordTiming` and changes nothing; at render, such a cue with word highlight on already reports `missing_timing`. `retranslate` drops the emphasis of every cue it rewrites and leaves cue text and timing as they were. Each command is one undo step; the selection persists in the project file. The LLM mode is not an edit: `captions.suggest_emphasis {item,cue_ids?}` is a consented network job (`JobControl`) whose output lists `{item,cue,word_ids}` suggestions, which `captions.set_emphasis` then applies. A refused consent or a provider failure is a typed failure; neither falls back to the lexicon.
