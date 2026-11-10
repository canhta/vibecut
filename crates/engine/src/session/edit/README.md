# Edit commands

Interface: the registered `Command` variants with `Effect::Edit` and their parameter types
`Placement`, `TrackPlacement`, `TrimEdge`, `ItemParamPatch`, `Move` and `NewItem`; `apply` and
`check_budget` are crate-private.

Per-command fields, effects, availability and examples are the declaration in
[`session/commands.rs`](../commands.rs) and the generated
[command reference](../../../../../docs/product/agent-control/commands.md). This page holds
only the rules that span commands.

## Import policy

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

Ripple policy is independent of app preferences. `Session::apply_with_ripple` and
`execute_with_ripple` pass it for one atomic command, `timeline.edit` supplies it for a whole
batch, nested policy wrappers refuse, and `timeline.set_ripple` sets the session default (off
initially, no project or history entry, refused during a transaction). One post-command diff
closes same-track tails vacated by delete, tail trim or leftward movement; it never shifts
another track, and cross-track moves, splits and hide/mute leave followers unchanged. The edit
and its shifts publish as one undo step ([`close_vacated_tails`](clips.rs),
[session](../README.md#transactions-gestures-and-changes)).

Errors are typed `EditError` categories with safe IDs and static diagnostics; commands perform
no IO and no retries.

## Video transitions

Clip edits revalidate the affected timeline before publishing: transitions follow moved cuts,
clamp after shortening and disappear when adjacency or handles are lost. A split extends the
ordered side pieces while keeping the span; explicit trim, move and delete detach pieces that no
longer form a contiguous chain reaching the cut. Failed edits publish neither item nor
transition changes. Transitions are video-only; [project](../../../../project/src/visual/README.md#video-transitions)
owns the admitted kinds and the invariant mathematics.
