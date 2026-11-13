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

`timeline.set_item_param` and `timeline.set_item_param_many` apply one typed `ItemParamPatch`, declared once in [`commands/item_params.rs`](../commands/README.md). `reduce_noise` (Boolean) sets `VideoParams.reduce_noise` on video items only; other kinds are left unchanged in a shared edit and refused in a single one.
Text content and font patches copy at most the control owner's `MAX_TEXT_BYTES` budget per
command tree, including nested batches. The single-item command refuses an incompatible target;
the many-item command skips incompatible kinds. Missing IDs, duplicates, invalid values and
locked compatible items refuse the whole edit ([`parameters.rs`](parameters.rs)).

`timeline.set_flipped` sets `transform.flipX` or `transform.flipY` on every listed active-scene item as one candidate. Every target is resolved before any write: an empty/duplicate list, missing or other-scene ID, locked track, audio/caption item or an axis carrying keys refuses the whole edit without a partial change. Video, image, text, overlay and graphic items are admitted. Absent and equal-false states are exact no-ops; the held Boolean is an ordinary zero-key parameter, so duplicates and splits clone it, trims/splits translate its keys and save/reopen preserves it.

`timeline.align_items {items,alignment,at}` and `timeline.distribute_items {items,axis,at}` translate `transform.positionX/Y` on every listed active-scene item as one candidate. `at` is the shared output clock; each item samples its real transform at `clamp(at - start, 0, duration)` so a non-overlapping selection still has one reference time. The envelope is the four transformed destination corners after positive scale, reflection, clockwise rotation and translation in project canvas pixels; captions' saved cue layout and audio have no box, and a transient render placement refuses the edit rather than aligning a different rectangle. Six alignments move each item individually against the canvas. Distribution requires three or more items, sorts by leading edge then ID, pins the first leading edge and last trailing edge, and refuses a negative gap. Every position base and scalar key is shifted by the same delta with key times, segments, handles and extrapolation preserved; absent channels become constant overrides, and raw or shifted values outside the admitted range refuse instead of clamping. A near-zero move is an exact no-op; all other changes are one undo step.

`timeline.replace_media {items,asset}` points each listed active-scene Video or Audio item at a source already in the project's asset list, changing only `source_asset`: trims, start, retime, keys and params stay. The asset must carry a stream of the item's kind (a Video item accepts a video or still-image asset) and cover the retained source and retime windows when its duration is known; otherwise it refuses with `Invalid("replacement media kind mismatch")` or `Invalid("replacement source too short")`. Caption, text, overlay, graphic and sticker items refuse as unsupported. Targets are checked in caller order on one candidate, so a missing or duplicate target or a locked track refuses the whole edit. Linked partners are replaced only when listed, and the edit neither raises canvas FPS nor ripples. One undo step.

`timeline.freeze_frame {item,at,duration,dir}` is a file-publishing control command, not a pure edit: its capture and PNG publication belong to [artifacts](../../artifacts/README.md). Its timeline part is one `Session::transaction` of ordinary commands: an optional `AddAsset` for a new still, `SplitAt` at the playhead, `timeline.move_items` shifting every item of the host track starting at or after the playhead right by `duration`, and an explicit-track `InsertItem` of a Video item with the still asset at the playhead. One undo step. Insert-and-split is the rule: the hold lengthens the timeline on that track only. Transitions stay on the outer edges; the move detaches a transition whose cut was the split, and none is created at the new cuts. Refusals: a relative directory, a non-positive duration, a missing, non-Video or locked host, or a playhead not strictly inside the item.

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
([`tracks.rs`](tracks.rs)). Transcript word cuts (`transcript.delete_range`, `remove_fillers`)
refuse `Locked` when any item a span overlaps, on any track, is locked; `transcript.merge_cues`
refuses when its caption item's track is locked. Both run one shared check in [`text.rs`](text.rs)
before any change.

`timeline.split_all {at}` cuts every unlocked item whose span contains `at`, on every track of the active scene, as one undo step. Locked tracks are skipped as the single-item split of a selection skips them; a playhead with nothing unlocked under it refuses the edit ([`execute_split_all`](clips.rs)).

The session owns the whole gap policy (ripple setting, magnet, exclusions); a caller sends plain
commands and the UI, MCP and CLI behave alike. `timeline.set_ripple` sets the session setting (off
initially, no project or history entry, refused during a transaction; the app sets it when its
preference changes), and every edit and replacement preview follows it. `timeline.edit {commands,
ripple}` is the explicit batch form for a wire caller; nested policy wrappers refuse. The
exclusions live in one table, `gap_ripple` in [state.rs](../state.rs): library audio never
ripples, a reviewed span cut always does, a batch follows its children. One post-command diff
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
`Outcome.closed_span` reports the decision after the edit (session ripple or magnet), so a caller that moves its playhead to a kept piece reads the result rather than repeating the magnet test.

`EditError` and `AssetUsage` are declared in this owner and re-exported through Session
and the crate. Its Result covers pure editing: safe-ID/static refusals and actual lower
ControlError, SessionError, project, time and caption sources. Project IO/busy/conflict/media
refinements are classified here, as are the static Project/Time/Captions wrapper diagnostics;
higher engine workflows delegate through `engine::control::Error::Edit`. Commands perform no IO or retries.

## Clip animation

`clip.set_animation { item, slot, preset, duration }` expands one built-in look once into
ordinary item-relative keys on one unlocked active-scene Text, Video, Image, Overlay,
Graphic or Sticker item ([`motion/animation.rs`](motion/animation.rs)). `slot` is `in` (the first edge),
`out` (the last edge) or `combo` (the whole clip); `preset` null removes the slot. Looks are
named for creators, not source parity. The fourteen In/Out looks are Fade, Slide left, right,
up and down (a quarter of the canvas width or height; the name is the direction the picture
travels), Zoom in and out (half the scale), Spin (180 degrees with a fade), Drop and Rise
(a slide with a fade), Pop (a 0.2 scale with a fade), Stretch (narrow and tall as it arrives,
with a fade), Swing (20 degrees with a fade) and Shake (a shake waveform compressed into the
edge, on X only). Out plays the same motion towards the edge. The ten
Combo looks are Pulse, Heartbeat, Sway, Float, Drift left and right, Push in, Pull out,
Flicker and Turn, each a table of waypoints in permille of the clip. Blur In is not yet a
look: it needs a blur channel the owned set lacks. The panel groups the looks into the shelves
Basic, Vibrate and Zoom (`AnimationPreset::shelf`). They write only `opacity`, `transform.position`,
`transform.scale` and `transform.rotate` (the owned channels); positions are canvas pixels,
Y grows downward and rotation is clockwise. Scalar bases come from the item's existing
channels or their documented defaults and are kept; linear keys have no handles and Hold
extrapolation. Values clamp to the transform bounds (so Turn tops out at 360 degrees).

Durations are integer ticks, 0.5 s by default; a repeat of the slot's current look keeps its
duration, so a duration change sends the same look with the new value. Rules, in order:

- Each duration is clamped to at least one canvas frame and at most the clip length, and the
  clip must last two canvas frames.
- In and Out never overlap: the later one (Out) shrinks to the room In leaves, and In is held
  one frame short of the clip while an Out exists. Out never grows back by itself. Where they
  meet exactly they share one resting key.
- Combo excludes In and Out: setting one clears the other side. A Combo takes no duration;
  an In or Out look is refused in the Combo slot and the reverse; a duration without a look
  is refused.
- There is no stored animation identity. `ClipAnimations::of` reads the slots back from the
  keys: a slot is found only while the keys on the channels it owns equal exactly what the
  look writes, so hand-edited keys read as no animation. A change is accepted only when every
  channel it touches holds no keys or exactly the current look's keys; otherwise undo or
  remove those keys first. Keys on other channels are never touched.
- Trimming either edge re-fits the animations found before the trim to the new length with the
  same clamp; a clip left under two frames loses them. Split, move and key-time scaling keep
  ordinary key semantics, and a speed change stretches the keys with the media.

Locks refuse even an exact repeat; an exact repeat records no history. One command is one
undo step.

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

`masks.set_path` replaces one existing mask's shape with a closed 3..=4096-anchor path and preserves every other field and its animations; an equal path is an exact no-op. `masks.add_point` inserts one de Casteljau anchor on a closed segment at `2^-20..=1-2^-20`, appending on the closing edge, and `masks.remove_point` removes unique current indices in one undo step while refusing fewer than three survivors. `clipboard.paste_mask_points` inserts the clipboard's copied anchors in order directly after anchor `after` with their tangents unchanged (`MaskPath::with_inserted`), one undo step; its snapshot is the item clipboard payload with `maskPoints` (see the [session clipboard](../README.md#mask-point-clipboard)). These commands refuse a non-path target, an open saved path or out-of-range input without publishing.

Keyframe Paste uses this same candidate/validation/one-undo path. Item Paste resolves
relative tracks, clears overlapped ranges with owner-side split/trim edits and inserts
fresh ids in the same candidate. Transfer and refusal
rules belong to the [clipboard contract](../README.md#clipboard).

The private edit allocator returns `SessionError` directly. Callers compose it into
mixed edit results; it neither wraps the counter nor changes allocation admission.
ID retirement and checkpoint rules belong to [Session](../README.md#public-entry-points).

`captions.set_word_highlight {item,enabled}` changes only an active-scene caption item’s opt-in. The ordinary locked-item guard runs even on equal requests; no-op, undo/redo, atomic batch and receipt/grant admission remain the shared command rules. It never changes styles, source text, verified correspondence or authoritative hidden trim cues.

`captions.select_emphasis {item,cue_ids,mode}` stores up to three keyword word ids per named cue in `CaptionParams.emphasis`; `mode` is `lexicon` (default), the deterministic local selection of `captions::select_lexicon`. `captions.set_emphasis {item,cue_id,word_ids}` replaces one cue's ids; an empty list clears it. Ids are indices into the cue's verified `CueWordTiming::Words` records, ascending, unique and in range, so a selection is always a subset of timed words. A cue without verified word timing (or whose source text no longer matches it) refuses the whole command with `captions::Error::NoWordTiming` and changes nothing; at render, such a cue with word highlight on already reports `missing_timing`. `retranslate` drops the emphasis of every cue it rewrites and leaves cue text and timing as they were. Each command is one undo step; the selection persists in the project file. The LLM mode is not an edit: `captions.suggest_emphasis {item,cue_ids?}` is a consented network job (`JobControl`) whose output lists `{item,cue,word_ids}` suggestions, which `captions.set_emphasis` then applies. A refused consent or a provider failure is a typed failure; neither falls back to the lexicon.
