# Edit commands

The kernel owns `BoundaryFailure`, the typed payload of `EditError::ClipBoundary`.
It carries refusal categories and zero-based indices without depending on a feature.
The [shorts crate](../../../../shorts/README.md) owns suggestion, snapping and batch export.

## Import policy

`import.add_asset` raises project FPS to the greatest video stream rate, compared
as exact rationals by `time::FrameRate`; it never lowers FPS. `timeline.insert`
applies the same ratchet for the inserted visual source, including preloaded
assets. Audio and image sources (including animated images) do not change FPS. A raised source rate
that cannot produce exact positive canvas ticks refuses the command atomically.
Invalid probe rational parts are unavailable at native probing and refused by
JSON ingestion, before policy execution.

When all project scene timelines are empty and `original_canvas_size` is null,
the first inserted visual source sets canvas width/height, the `original` preset,
the last-custom size and that original size. Subsequent inserts, including after
deleting all items or switching scenes, retain it. Existing
audio/text/overlay items prevent this automatic initialization.
Batches execute in supplied order, so simultaneous first inserts have a single
deterministic winner. All policy changes share the import/insert history step;
failed commands, cancelled gestures and failed batches publish none of them.
Ephemeral assets remain in project storage and available to insert/render, while
the desktop media library omits them and its selection uses only visible assets.

`media.remove { assets }` removes a nonempty, unique list of at most 100,000
existing asset IDs as one undo step. It refuses missing IDs with `NoAsset` and
any reference in any scene with `AssetsInUse(Vec<AssetUsage>)`, listing the asset
and its item IDs. Refusal preserves the entire project and history; successful
removal deletes records only, retaining source files and unrelated content.
Undo restores the exact records and their project order; redo removes them again.

## Timeline placement

`timeline.move_items { items, delta, placement? }` moves a nonempty unique
selection by one signed output-tick delta. Omitted/null placement retains each
member's track. An explicit placement uses the insertion placement owner; an
above/below drop allocates one destination per source track, retaining the
group's horizontal offsets. Empty-main origin correction shifts the whole
selection, including audio partners, by the same amount. The caller supplies
every linked partner in `items`; there is no implicit link model. Invalid IDs,
locked tracks, incompatible kinds, overlaps, negative times and arithmetic
overflow refuse the entire edit, including any new tracks. Snapping remains a
caller decision, outside this algebra.

`timeline.trim { items, edge, delta }` moves every selected start/end edge by
the same signed output-tick delta (`edge: start | end`). It delegates to the
existing trim owner for source ranges, playback rates, keyframes, cues and
fades. A refused member, overlap or edge that cannot represent the exact shared
delta at its playback rate refuses the entire group.

`timeline.preview { commands }` stages a bounded batch of edit commands in the
same transaction state used by explicit gestures. Each call replaces the whole
candidate from the entry snapshot, so deltas are absolute from drag start.
An empty explicit pointer gesture can enter replacement mode; a candidate that
already differs from its entry refuses promotion, retaining its staged edits.
`project()` remains the committed snapshot; `preview_project()` exposes the
candidate only while a gesture is active. This seam also serves transform and
graph previews. `timeline.commit_preview {}` publishes one undo step and one
change event; `timeline.discard_preview {}` restores the exact entry project,
history and pending change without a history entry. A net-zero commit preserves
redo. Failed replacement retains the previous valid candidate. Save, undo,
selection changes and nested gestures refuse while preview is active. Preview
IDs remain retired after discard. Preview commands are human gesture controls;
external agents use atomic batches and undo.

`timeline.insert { placement, item }` performs track choice, optional allocation
and insertion as one atomic undo step. `placement` is a tagged object with
`mode: auto` (the default), `track` with a track ID, or `above`/`below` with a
reference track ID. `Outcome.placements` reports the final track and item IDs
for allocated items still present after the command/batch; `created` continues
to report only new IDs in allocation order.

Auto video insertion uses the unlocked main track when magnet is enabled.
It picks the nearest clip edge at the requested start (ties before), inserts
without replacing content, then packs the main track from zero. This is an
engine placement policy, separate from same-track ripple deletion/trim.
With magnet disabled, or for other kinds, auto chooses the first unlocked
compatible track whose requested half-open range is free: main, overlays in
screen order, then audio in screen order. If none fits it creates a secondary
track at the top of its group. Locked main falls back to secondary tracks.
The first insertion or move into empty main clamps to zero even with magnet
disabled. Simultaneous moves shift the earliest main-targeted item to zero and
preserve relative spacing; other destination tracks keep their requested times.
Subsequent explicit placements onto occupied main retain their requested time. Explicit track
placement refuses overlaps and incompatible kinds. Negative starts and checked
arithmetic overflow refuse atomically, including track/ID allocation.

`timeline.add_track { name, kind, placement }` uses `mode: auto` by default,
or `above`/`below` with a reference ID. Relative placement respects the project
owner's groups: non-audio above main, audio below main; crossing these boundaries
refuses. Creating a track does not edit its reference, even when it is locked.
Compatibility is determined by `ItemParams::kind()` in project; empty reserved
track kinds remain available but cannot receive unsupported item kinds.

Command integration and fixed-seed random insertion tests cover compatibility,
ordering/non-overlap, first-main clamping, undo/redo and persistence round trips.

## Transient selection and shared fields

`timeline.set_item_param` also accepts the exact `transform.positionX`,
`transform.positionY`, `transform.scaleX`, `transform.scaleY` and
`transform.rotate` patch fields with numeric values. They set a constant override
on project's existing channel and refuse non-finite/out-of-bounds input atomically;
audio items are incompatible. `background_color` (text) and `graphic_stroke`
(solid overlay) set hex-text overrides on the existing `background.color` and
`params.strokeColor` channels; `graphic_fill` edits the overlay's static colour.
All colours reuse project's straight-alpha RGBA8 value. `project.set_background`
sets that same value on the canvas in one history step. `project.set_canvas`
writes a validated width/height and an optional fixed preset (the `original`
preset resolves the retained first-visual size); a custom width/height commits
its nearest even size on each odd axis (ties up) so the canvas can always
export, and custom sizes are recorded as the last custom size;
`project.set_fps` stores an exact positive rational. Each is one
history step. Masks retain `masks.update`.

`set_selection(BTreeSet<ItemId>)` validates existing IDs, breaks merge continuity
and changes no Project content, revision or history length. A change during an
active gesture/group refuses. `selection()` exposes the current item IDs.
Each bounded undo snapshot retains the selection that produced the edit; undo
and redo restore both. Deleted IDs are pruned from current selection. Scope
selections (mask points/keyframes) remain app-owned. Selection is absent from
project JSON and optional persisted metadata.

`timeline.set_item_param_many` accepts a nonempty, unique `items` list (at most
100,000 IDs) and one typed `patch`: `audio_gain` in −120..=24 dB, `audio_muted`, `text_content`,
`text_font` (1..=256 bytes), `text_size` in 1..=4096 px,
`text_color` as four RGBA bytes, `text_background` as an optional padded box,
`text_background_color` as RGBA bytes, `text_background_padding_x`/`_padding_y`/
`_corner_radius` in 0..=128 and `text_background_offset_x`/`_offset_y` in -1024..=1024
canvas pixels (each preserves the other box fields or creates a baseline box if absent),
`text_weight`, `text_italic`, `text_letter_spacing`, `text_line_height`, `text_align`
(`left`/`center`/`right`), `text_decoration` (`none`/`underline`/`line_through`), and
`text_position_x`/`text_position_y` as canvas percentages in 0..=100. All numbers
must be finite.
Text content/font patches copy at most the control owner's `MAX_TEXT_BYTES` budget
across compatible targets per command tree, including nested batches; excess
expansion refuses atomically.
The command preserves unrelated fields and skips incompatible item kinds;
missing IDs, duplicates, invalid values and locked compatible items refuse the
entire edit. An all-incompatible selection is a no-op. One successful changed
command is one undo step, including every item's previous field value.

## Edit semantics and limits

Commands build a private candidate, then validate through `Project::new` before
publishing one snapshot. Failure leaves project, history and allocation untouched.
Each batch child must also yield a valid project; a failed child rolls back the
whole batch. Typed commands admit at most 1024 commands and nesting depth 16.
JSON dispatch additionally caps depth at 32, nodes at 200,000, each collection at
100,000 entries and aggregate string/key bytes at 4 MiB before deserialization.
The caller already owns the Value; these caps bound added processing, not its
initial allocation. Project and source constructors retain their own limits.

- `timeline.add_track` inserts a secondary track at the highest position for its group;
  `track.remove` refuses the mandatory main track and removes secondary contents
  in the same undo step. `track.set_muted` accepts Video/Audio;
  `track.set_hidden` accepts all kinds except Audio. Hidden filters visual output;
  muted filters audio, independently, for preview and export.
  `track.set_magnet` persists main-track magnet state, undoable with the project;
  gap-closing execution belongs to the ripple edit policy, not this preference.
- `timeline.insert` places an item without shifting or overwriting its neighbors.
- `move_item` / `move_items` set absolute output positions and destination tracks.
  Multi-move removes all selected items first, permitting swaps. Duplicate IDs
  fail; wrong track kinds, overlap and missing references fail whole-candidate
  validation. Touching half-open intervals are valid.
- `split_at` uses an output tick strictly inside the identified item, retaining
  the left ID and allocating a right ID with continuous source timing. At a
  retimed item the source cut comes from project's mapping, so the pieces touch
  exactly at the requested tick, the right piece ends at the original end, and its keyframes move
  by the left piece's output length.
- `trim_start` / `trim_end` set absolute output edges, adjusting source edges at
  the item's speed with checked arithmetic (project's mapping keeps the other edge
  fixed). No implicit frame snapping occurs.
- `timeline.set_speed {items, speed, maintain_pitch}` retimes unique video/audio
  items atomically, keeping each source window: the duration follows, keyframes
  stretch by `old / new` and, on the magnetic main track, later items shift by the
  duration change; elsewhere growth that would overlap a neighbour refuses. Pass
  every linked video/audio item in one call to keep them in sync. Speed is the
  decimal 0.01-5 (rounded to 0.001); generated items refuse.
  Known media handles bound extension; generated content has no media endpoint.
  Caption mapping clips retained cues through captions' mapping owner, preserving
  source cue IDs, languages, text, translation references and edit flags. A trim
  commits retained cue content; a later independent extension cannot recover
  discarded cues. Gesture, closure-group, batch and equal-key merged trims rebuild
  each preview from retained entry cues rather than the previous clipped preview.
  Selected `retranslate` target edits update that retained content and remap the
  visible view, preserving hidden/unselected cues and unclipped source ranges.
  An explicit `set_item_param` replaces the full retained content; splitting
  establishes separate retained content for each piece. Refusal rolls back retained content
  with the candidate. Retention ends at commit/cancel or a merge boundary.
  Undo restores the original snapshot and all its cue content.
- Ripple policy is independent of app preferences: `Session::apply_with_ripple`
  and `execute_with_ripple` pass it explicitly for one atomic command, including
  replacement previews. Headless `timeline.edit { commands, ripple }` supplies
  the policy atomically for a whole batch; nested policy wrappers refuse.
  Headless `timeline.set_ripple` sets the session default
  (off initially, no project/history entry, refused during a transaction).
  One post-command diff, after the whole batch, closes same-track vacated tails
  from delete, tail trim or leftward movement. It subtracts every occupied span,
  closes overlapping group tails once and never shifts another track. Cross-track moves,
  splits and hide/mute leave followers unchanged. Edit and shifts publish as one
  undo step; previews rebuild from their baseline, including on reversal.
- `delete` lifts identified items leaving gaps when ripple policy is off. `ripple_delete` removes one item
  and shifts later items on that track left by its duration, preserving earlier
  gaps; other tracks and bookmarks stay fixed. No cross-track sync lock exists.
- `set_item_param` replaces the complete typed ItemParams. `track.set_flags`
  replaces all three flags and is permitted on locked tracks so they can unlock.
  All item edits, insertion, moving into/out of, and track removal refuse locks.
  `track.set_locked` toggles the same barrier for one track.
- `track.rename` changes a track's display name and `track.reorder` moves a
  secondary track to a zero-based index inside its own section (overlays top to
  bottom, audio top to bottom); the mandatory main track refuses. Both, together
  with `track.set_locked`, are project data with exact undo/redo and persistence;
  they remain permitted on a locked track, while all item structural edits and
  track removal return `EditError::Locked`. The add command is
  `timeline.add_track` (name, kind, optional above/below placement); secondary
  tracks start unlocked.
- `track.remove` removes its items together. `add_bookmark` accepts a nonnegative
  output position/duration and normalized note; `bookmark.move` changes its start
  while preserving duration and refuses missing, negative or overflowing targets.
  Asset, track and bookmark additions/moves are validated with the rest of the
  candidate.

Errors are typed `EditError` categories with safe IDs/static diagnostics and
retained project/time/caption sources. No raw JSON diagnostics or parameter text is
returned. Commands perform no IO or retries; processing lifecycle is described below.

## Keyframe commands

Project owns the [parameter model and evaluator](../../../../project/src/animation/README.md#animated-parameters-and-retime).
Engine adds the registered `keyframes.set_param`, `keyframes.add`,
`keyframes.remove`, `keyframes.retime`, `keyframes.set_handle`,
`keyframes.scale_time`, `keyframes.toggle_at`, `keyframes.delete_selection` and
`keyframes.set_extrapolation` commands. The registry owns their exact required
fields and wire types. `set_param` installs/replaces a validated static/animated
override; add/remove/move/handle edits select its path and an optional RGBA
component. For scalar/discrete params the component is null; RGBA requires r/g/b/a.
`toggle_at` adds the key at an exact time with the caller's evaluated value or
removes the key already there; `delete_selection` removes a nonempty list of
addressed keys atomically; `set_extrapolation` changes the before/after hold or
linear policy without changing keys or the base value.
`retime` moves one key between exact local ticks and refuses an occupied time;
`scale_time` stretches all item channels/handle times by a positive rational,
without changing media playback or item duration. Missing paths/keys, invalid
handles, locks and failed budgets preserve the entire candidate/history.

Commands share existing typed/JSON dispatch, batches, undo/redo and gestures.
Begin/commit one gesture for a drag; previews commit one history step and cancel
restores the original curves. Range edits call project's checked translation,
retaining out-of-window keys; no second interpolation or tangent algorithm.
`NewItem.animations` carries explicit animation data on insertion. Rendering and
CLI property/export wiring are separate consumers of the project evaluator.

## Effect commands

`effects.add { item, effect }`, `effects.remove { item, effect: instance_id }`,
`effects.reorder { item, order: [instance_id, ...] }`,
`effects.toggle { item, effect: instance_id, enabled }` and
`effects.update_param { item, effect: instance_id, key, value }` mutate the project
stack through normal atomic history. Locked/missing items, duplicate IDs, unknown
instances, wrong types and out-of-range intensity refuse. Add stores enabled state
and static parameters; scalar animation overrides use `keyframes.set_param` at
`effects.<id>.params.intensity`. Remove deletes those animation paths in the same
undo step. Reorder requires an exact permutation and preserves instance identity
and animation paths. Toggle preserves params and animation. Param edits replace
one static scalar; an existing animation still overrides it. Every command uses
gestures/merged edits for one undo step per continuous interaction. Availability
requires an unlocked visual item; audio refuses all effect edits. CLI `exec`
consumes these same registered commands before export.

Derived illustrations use the [effect thumbnail contract](../../artifacts/README.md#effect-thumbnails).

## Mask edits

`masks.add`, `masks.remove`, `masks.update` and `masks.reorder` operate on the
project-owned ordered mask vector through the same atomic candidate/history path.
Update selects by the supplied mask's stable ID; reorder requires an exact
permutation. Missing/duplicate identities, invalid parameters and locked tracks
refuse the entire command or batch. Removal also removes that mask's animation
channels; undo restores both. Registry schemas and CLI execution share these
commands and their examples. No app-only mask state or evaluator is introduced.

## Scene commands

`scene.create {name}` appends an empty secondary scene without switching and
returns its allocated `CreatedId::Scene`. `scene.rename {scene,name}` edits a
scene name. `scene.delete {scenes}` atomically deletes the selected non-main
scenes; any unknown or main ID refuses the whole edit. Deleting the active scene
selects main. Create, rename and multi-delete each use one ordinary undo step;
delete undo restores scene timelines, items and bookmarks together.

`scene.switch {scene}` selects an existing scene, clears item selection and
terminates edit coalescing without adding undo history or clearing redo. It is
refused during gestures/transactions and inside batches. Undo/redo preserves the
currently selected scene when that scene exists in the restored snapshot;
otherwise it uses the snapshot's valid scene choice. All timeline commands target
the active scene; allocation scans every scene on load and never reuses a live
scene, track, item or bookmark ID.

Switching retains zoom, horizontal scroll and playhead per scene in project's
optional metadata sidecar, outside undo. New scenes start with default views;
removed scenes' views are pruned. Canvas/fps/background remain shared. Rendering,
audio mixing and CLI export use the active scene by default; export settings can
resolve explicit scopes through project's `SceneScope` and `for_scene` contract.

## Visual item fields

`timeline.set_item_param` accepts `{ "item": 1, "patch": { "field": "blendMode",
"value": "multiply" } }` or an `opacity` field with a finite 0..=1 value.
The same typed patch works in `timeline.set_item_param_many`. Visual fields refuse
audio targets in the single-item command; the shared command skips incompatible
items. Locked/missing targets refuse atomically. Blend changes retain animation;
setting constant opacity replaces its animation channel, as `keyframes.set_param`
does. Keyframe commands still own animated editing. History and persistence retain
both fields through the existing snapshot path; no adapter owns duplicate values.

## Audio controls

`timeline.set_item_param` and `timeline.set_item_param_many` accept `volume`
(finite -60..20 dB, step 0.01) and `muted` (bool) patches on audio and video
items. The Rust variants remain `AudioGain`/`AudioMuted`; old transport field
names are refused. Constant edits replace the corresponding animation override;
keyframe commands edit scalar `volume` or held bool `muted` paths through the
same evaluator. Locked/missing targets and invalid grids refuse atomically.
`audio.set_master {volume, muted}` changes project-level master controls.
`audio.set_fade {item, edge, duration, curve}` sets one clip fade (`edge` is `in` or
`out`, `curve` `equal_power`, `linear` or `exponential`) on audio and video items.
A negative duration, a non-audible item or a locked track refuses atomically; a
duration beyond the clip is clamped to the clip minus the other edge, so the two
fades never overlap. `split_at` is non-destructive: both pieces keep the whole
shape of any fade reaching them and record the part the other piece holds
(`Fades::split`), so a cut inside a fade, at its boundary or off the sample grid
leaves the summed envelope unchanged. Moving a trim edge starts a fresh fade of the
audible length there. Trimming to its existing position preserves the fade phase.
Stored durations beyond the shortened clip stay, and render clamps the window.
All three edits use ordinary gestures, transactions, undo/redo and persistence;
render owns mastering, sample timing and latency compensation.

## Video transitions

`transition.add {track, from, to, kind, duration, alignment}` allocates a transition
ID and clamps duration through project's handle rules. `transition.move
{transition, start}` slides the existing span over its cut, clamping to available
handles and updating its alignment. `transition.set_duration {transition,
duration}` resizes following that alignment; `transition.remove {transition}`
removes only the transition. Locked tracks refuse all four commands. IDs use the
same nonreused session counter as items, tracks and bookmarks; `CreatedId::Transition`
reports additions. Every command supports batches, gesture cancellation, undo/redo
and persistence through the existing session snapshot path.

Clip edits revalidate the affected active timeline before publishing: transitions
follow moved cuts, clamp after shortening, and disappear when adjacency or handles
are lost. Splits extend the transition's ordered side pieces while keeping the
original span and progress. Explicit trim/move/delete edits detach pieces that
no longer form a contiguous chain reaching the cut, then revalidate the remaining
side extents. Splits retain the original retime processing window for both video
mapping and audio replay. Failed edits publish
neither item nor transition changes. Audio remains on its existing mixer path;
these transitions are video-only. [Project](../../../../project/src/visual/README.md#video-transitions)
owns the admitted kinds and invariant mathematics.
