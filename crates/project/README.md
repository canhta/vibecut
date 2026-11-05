# project

Headless project/timeline snapshots, invariant validation, `.vibecut` persistence,
atomic replacement and recovery snapshots. Editing commands/history belong to
engine; media owns `SourceAsset`; captions owns cue content and language tags.
Project also owns item-relative parameter/keyframe data, validation, evaluation and
time propagation. No codecs, widgets, device state, scheduling or effect execution.

## Entry points and invariants

`Project::empty` creates a 1920×1080, 30 fps, black-canvas project with 48 kHz
output audio. `Project::new(ProjectData)` validates a complete candidate;
`data()` borrows an immutable snapshot and `into_data()` yields an editable
candidate for an engine command. Revalidate before committing. Serde
construction also validates; use `decode`/`load` for untrusted JSON ingestion so
allocation and nesting budgets apply before typed deserialization.

Project, scene, track, item and bookmark IDs are separate `u64` newtypes; media owns
asset IDs. IDs are unique within their type across a project. Zero is permitted;
engine owns allocation. `Timeline` owns `overlays` (topmost first), one mandatory
`main` Video track named `Main Track`, `audio` (topmost first), and persisted
`magnet` state (default on). `iter` / `iter_mut` traverse bottom-to-top render
order; UI reverses this traversal. Secondary insertion uses `insert_track` at the
highest position of its group. Main is never removed by an engine edit. Track kinds
are Video, Audio, Text, Sticker, Effect, Filter and Adjustment. Captions share Text
tracks; generated solid-colour `ItemParams::Overlay` uses Video tracks. Sticker,
Effect, Filter and Adjustment accept empty tracks until their item owners land.
Groups, main kind and unique IDs are validated on every candidate. The main track's
name is editable like secondary track names. Items
within each track are sorted by output start, nonnegative and nonoverlapping.
Touching half-open intervals are valid. Item end uses checked arithmetic. There
is no retiming: output duration equals `source_range.duration()`. An active scene's
output duration is the maximum item end, excluding bookmarks.

Each item's typed `ItemParams` determines its track kind. Video/audio require an
asset with a corresponding stream and cannot exceed its known presentation
duration. Caption items retain source asset identity and captions' `CueTrack`,
including source ranges, languages, edited flags and translation references.
Cues must fit the item's source range. Text/solid-overlay items have no media
reference; their source range uses local content ticks. Missing referenced asset
IDs are invalid; missing files are valid. After loading, use
`SourceAsset::availability()` to show relink state; explicit relinking verifies
content before retaining the asset ID. Source paths are absolute native paths;
working directory changes never reinterpret them.

Audio items carry `AudioParams::source`. The default `upload` source references a project asset; `library` records the provider identity, attribution, licence, source page and exact preview URL that produced the cached bytes. The record is validated (positive provider id, HTTPS URLs, bounded text) and round-trips in the project file, so export can write attribution and licence metadata without guessing.

Names, notes and text use captions' NFC text owner. Canvas frames must have exact,
positive integer tick durations. Canvas area is at most 2^26 pixels; output audio
admits 8–192 kHz. Audio/video source volume and master volume admit -60..20 dB on a 0.01 dB grid; font sizes admit 1..4096 pixels.
Text items also carry a 1..256-byte font-family name, an optional padded RGBA
background box, and x/y canvas percentages in 0..=100 (default 50/84.5 reproduces
the original lower-third box). Typed invalid/unsupported/limit errors refuse a whole candidate.

`ProjectData.original_canvas_size` is the optional first visual media size, not a
second user canvas setting. It starts null and retains the first inserted media's
width/height for reset-to-original. Its dimensions obey the canvas area bound.
[Engine import policy](../engine/README.md#import-policy) owns automatic
initialization and FPS ratcheting in import/insert commands.

## Scenes

`ProjectData.scenes` is a nonempty JSON array, represented by `Scenes` with one
protected main scene and up to 255 secondary scenes. `current_scene_id` identifies
an existing scene. Scene IDs are unique; track, item and bookmark IDs remain
unique across the entire project, including inactive scenes. All existing item,
track, animation and bookmark validation and aggregate budgets span every scene.
A candidate with no main, multiple mains, duplicate IDs or a missing active scene
is refused; loading never repairs it.

`Scene` owns its NFC name, `is_main`, timeline and bookmarks. Assets, canvas
(width, height, fps, background) and output audio settings remain project-level.
`ProjectData::scene` / `scene_mut` access the current scene; incomplete candidates
read main when their current ID is invalid, but `Project::new` still refuses those
candidates. `Scene::duration` derives content duration. `Project::duration` uses
the active scene. `Scenes::main().duration()` supplies main-scene duration to
project cards without changing the active scene.

`Project::scene_ids(SceneScope)` resolves Active, All or an explicit nonempty,
unique Selected list before any export scheduling. Unknown/duplicate IDs and
more than 256 selections refuse. `for_scene` creates a validated immutable
snapshot for the selected scene; preview, mixing and export consume that same
active-scene snapshot. Export settings UI owns output scope controls and paths.

## Animated parameters

`TransformField` owns the editor's units, defaults and finite clamps for the five
existing transform channels: position X/Y in project-canvas pixels
(−100,000..=100,000, default 0), independent X/Y scale ratios
(0.01..=10, default 1), and clockwise rotation in degrees
(−360..=360, default 0). The command adapter refuses out-of-bounds input;
animated evaluation clamps overshooting curves to these bounds. These channels
reuse the existing persisted animation representation without a schema change.

`Item.animations` is the single `Animations` map for parameter overrides. A missing
path leaves the consumer's existing static property in use. A present `Param`
owns its explicit base value and keys, including when no keys remain; editing
static item content does not rewrite this independent override. `Param::new(value)`
creates a static override. Validate edited candidate data through `Project::new`.
`Animations::value_at(path, local_tick)` is the shared evaluation entry point for
engine, rendering, audio automation and graph consumers. None means absent,
not zero. No render/CLI execution is introduced by this data contract.

`AnimationPath::new` admits `transform.positionX/Y`, `transform.scaleX/Y`,
`transform.rotate`, `opacity`, `volume`, `muted`, `color`, `background.color`,
`background.paddingX/Y`, `background.offsetX/Y`, `background.cornerRadius`,
`params.<name>` and `effects.<id>.params.<name>`. Extension ids/names are 1..=64 ASCII
letters/digits/underscore/hyphen; complete paths are at most 160 bytes. These
extension namespaces reserve animation data, not an effect/graphic implementation
or catalog entry. Future owners resolve ids against their admitted implementations.
`volume` is scalar dB (consumers clamp overshoot to -60..20); `muted` requires a held bool. Built-in numeric paths require scalars; color paths accept linear RGBA composites
or discrete text values. Extension paths admit all parameter kinds.

## Clip fades

Audio and video items carry `fades {fade_in, fade_out}` (default none), each a
`Fade {duration, offset, curve}`: the whole fade length in ticks from the edge it was
set on, the ticks of it a split cut away beyond this clip's edge (`0..=duration`,
default 0), and a `FadeCurve` (`equal_power` default, `linear`, `exponential`).
Only the window `duration - offset` is audible on the clip; that window, not the
stored `duration`, is what the clip length clamps. Fades are persisted data, not
animation: they multiply the volume curve and are never keyframed. Validation
refuses negative durations and an offset outside `0..=duration`; durations beyond
the clip are legal because trims shorten clips without rewriting them. `Fades::set`
clamps one edge to the clip duration minus the other edge's window and starts a
fresh fade (offset 0). `Fades::split(start, at, end)` returns the two pieces' fades:
each keeps the whole shape of any fade reaching it and records the part the other
piece holds (an empty window is no fade). `Fades::spans(start, end, rate)` converts
ticks to samples once, rounding both absolute endpoints with nearest rounding, so
a piece lands on exactly the samples the whole clip had; windows clamp to the clip
and overlapping stored windows multiply. `FadeSpans::gain(sample)` is the single
per-sample gain used by preview and export: exactly 0 at the clip edge
where a fade starts, exactly 1 in the body. Both pieces of a split together
reproduce the original gain at every sample. A fade without `offset` loads as uncut. Curves: `sin(x·π/2)` (-3 dB midpoint), `x`, and `(e^{4x}-1)/(e^4-1)`.

`ParamValue` holds scalar, bool, NFC text or linear-light unpremultiplied RGBA.
RGBA uses four independently keyed scalar `Channel`s, selected by `ColorComponent`;
there is no shared-key-time restriction. Stored RGBA values are in 0..=1 and
composite evaluation clamps overshoot/extrapolation to that range. Input/output is
already linear RGBA; existing byte-color fields are not silently reinterpreted or
converted. Discrete color text stays a held string. `AnimationPath::scale_group()`
identifies the scaleX/Y editor group; both scalar curves remain independent.

`Channel::new(base, keys, before, after)` validates strictly increasing unique
key times and homogeneous values. Each scalar key's `Segment` governs the segment
leaving it: Step, Linear or Bezier. Exact endpoints return their stored values.
Bezier handles contain relative signed `dt` ticks and `dv` in scalar value units;
incoming dt <= 0, outgoing dt >= 0. At evaluation dt is clamped to the adjacent
span. Missing handles use (span/3, delta/3) secants. The cubic time curve is inverted
before evaluating its absolute value curve, including nonzero handles between
equal endpoint values. Scalar extrapolation before/after is independently Hold
or the edge/neighbor Linear secant; a single key holds.

`TangentMode::{Auto, Aligned, Broken, Flat}` are persisted source labels only.
They do not calculate, flatten or mirror handles and never change evaluation.
This is an explicit source gap: no UI may advertise them as easing choices until
a separate ticket defines their mathematics. Explicit dt/dv handles are authoritative.
Bool/text keys require Step and Hold extrapolation; before their first key the
explicit base applies, then each key holds until the next. A scalar channel before
its first key follows its configured edge extrapolation.

Key times are signed i64 ticks relative to item start. Negative and out-of-window
keys are retained: trims/splits translate anchors rather than discarding or
resampling curves. `Animations::shift` checks overflow, preserving exact tick
differences and handle offsets; engine's range changes call this once. Trim-end
and item moves retain local times. `Animations::retime(num, den)` stretches key and
handle times by a positive rational through `Tick::checked_scale` with nearest-tick,
ties-away-from-zero rounding. It refuses collisions/overflow atomically. Integer
stretches of frame-aligned 29.97-fps keys remain exact. General rational stretches
may round each timestamp by at most half a tick; they do not promise an invertible
round trip. Media playback rate/output duration remain the retime consumer's job.
Moving one key leaves explicit handles unchanged and clamps them when evaluated.

Limits: 64 paths per item, 4096 keys per channel, 100,000 keys per project; JSON
budgets below also apply. Stored scalars/handle dv must be finite with absolute
value <= 10^12. Unknown fields/paths, wrong component kinds, unordered/duplicate
keys, invalid handles and over-budget input refuse. Raw channel deserialization
bounds its key sequence; map deserialization refuses duplicate paths. Candidate
mutations remain subject to whole-project validation. Numeric evaluation uses
allocation-free scalar/rgba arithmetic and binary channel lookup; no throughput
or renderer output claim is made.

## Persistence and limits

`encode`/`decode` use one JSON envelope: `format`, `schema_version`, `generator`,
`project`. Current schema 16 adds timeline `transitions` to schema 15 audio/video clip `fades`, following schema 14, which added optional imported cue style to schema 13 text weight, italic, letter spacing and line height (also on text masks), which followed schema 12 audio `source` (upload or library provenance), volume/mute and master controls.
ProjectId is stored as a canonical decimal string, preserving
all u64 identity bits for control clients; other IDs remain numeric. Schemas 1-15
are refused rather than migrated or silently defaulted. Older schemas return `TooOld`, newer schemas `TooNew`; there are no
migration or compatibility paths. Unknown current-schema project/envelope fields
are refused. `WriterLease::save` serializes completely before touching the destination;
read-only `load` reads at most 16 MiB + 1 byte, including when a file grows while reading.

Ingestion and serialization cap JSON at 16 MiB, nesting at 64, individual raw
JSON strings at 1 MiB and combined comma/colon separators at 200,000, including
ignored fields. Those aggregate budgets can be reached before individual model
caps: 10,000 assets, 256 tracks, 100,000 total items/bookmarks, 100,000 cues per
caption item, and 1 MiB per stored text. Exceeding a budget returns an error;
no truncation or silent repair occurs. JSON diagnostics expose line/column only,
without user content. Raw serde use does not apply the ingestion budgets.

`atomic_write` is the shared replacement primitive for later preferences/bundles.
It stages privately in the destination directory, writes, restores the previous
file's permissions, syncs, then atomically replaces. Failed staging/publication
cleans up and preserves prior bytes. Destination symlinks are refused; parent
sync after publication is best effort. It does not preserve ownership, ACLs,
extended attributes or hard links. Hardware power-loss durability and Windows
runtime behavior remain unverified. Callers serialize writes to one target.

## Writer lease and save acknowledgement

`WriterLease::acquire(path)` takes an exclusive OS lock on the stable adjacent
`.FILENAME.lock` before reading project bytes. Retain the lease from load through
editing and saving. Dropping the lease explicitly unlocks before closing its
handle, so descriptors inherited during concurrent subprocess creation cannot
extend its lock lifetime. `WriterLease::load` refuses a changed baseline; use standalone
`load` only for read-only ingestion. `acquire_wait(path, duration, cancelled)` polls
contention and returns `Busy` on timeout/cancellation. Native filesystem calls are
not interruptible, so cancellation is checked between them. Unsupported lock paths,
permissions or OS locking return typed errors and never authorize an unlocked save.

Project data and permanent writer control files have disjoint namespaces. All
atomic publication and writer acquire/save-as paths refuse a basename beginning
with `.` and ending with `.lock` (ASCII case-insensitive), including reusable or
not-yet-created sidecars. Admission checks both supplied and canonical names, so
symlinks and directory/case aliases cannot publish project bytes over a lease.
Windows also refuses trailing spaces/dots and alternate-stream names before
creation, preventing different spellings from acquiring separate sidecars.
Existing project files with reserved names remain readable; save them to an
admitted destination rather than rewriting the control namespace.

A lease retains its original lock object and verifies the current sidecar identity
at acquisition and before/after bounded reads. Unix compares device/inode with
metadata from the held descriptor. Windows opens without delete sharing and uses
full 128-bit file IDs; unavailable identity support refuses the lease instead of
falling back to ambiguous 64-bit IDs. A missing, replaced, symlinked or differently
identified sidecar returns `Conflict`, including when both objects are regular
files with identical bytes. Invalidated leases cannot load, save or save-as.

`save(&mut self, project)` compares the current bounded file's full BLAKE3 content
with the acquired/last-saved baseline, then stages through the existing atomic
writer. After staging/sync it rechecks both content and lease identity immediately before publication; a refused final
precondition cleans staging and returns no receipt.
A changed, removed or unexpectedly created file returns `Conflict`; failed saves
retain the baseline and issue no receipt. A successful `SaveReceipt` contains the
canonical native path and hash of the exact envelope bytes published. Engine binds
its own saved revision and still-current path to that receipt before acknowledging
clean state; revision tracking/journal scheduling remain engine-owned. Schema and
JSON envelope are unchanged; leases, hashes and receipts are not persisted in it.

`save_as` acquires the destination while retaining the source lease, publishes,
then replaces the held lease only on success. `save_to(&self, &mut destination,
project)` publishes to an already acquired destination while retaining the source;
engine uses it to defer lease adoption until revision/receipt acknowledgement. Both
paths refuse an invalidated source lease. Busy/failure preserves source path
and both files. Existing symlinks resolve to the target, including symlinked parent
directories; relative paths resolve once. Case/Unicode aliases follow the native
filesystem's name rules, including for adjacent lock files. Native `OsStr` names
are retained without lossy conversion. Filesystem-rejected names return typed
errors; the lock name adds six native characters, so near-limit filenames can be
refused rather than truncated into colliding locks.

The lock file is private on Unix, is never truncated or removed, and contains no
project data. Dropping the lease or killing its process releases the OS lock; the
empty file remains safe to reuse. The shared atomic publisher enforces the control
namespace too. Locks are advisory: content checks detect external edits present
before publication, but do not provide atomic compare-and-swap against another
application racing the final check and rename. Lock-file/directory replacement
observed at a validation boundary invalidates the lease. Hard-link preservation
remains outside the atomic writer contract.
Local-filesystem writer behavior is exercised by macOS and Windows tests;
remote filesystem locking remains unverified.

Managed-directory moves opt in with `WriterLease::enable_directory_moves(root)`
before projects are published. This creates the permanent private control file
`<root>/.vibecut-directory-moves.lock`, outside every movable child. All ordinary
writer acquisitions beneath a child take shared admission on that file through
adjacent-lock acquisition, then release admission; independent open projects do
not block each other. Acquisitions begun before barrier installation recheck
admission after locking the sidecar, before reading or exposing a writer.
`WriterLease::move_directory(destination)` consumes a held
project lease, takes exclusive admission, validates source/content and an absent
sibling destination, closes the adjacent handle, and renames while still holding
exclusive admission. New writers at either path receive Busy throughout that
interval; existing writers remain excluded by the adjacent project lease. This
preserves Windows sidecar identity protection without delete sharing or an
unprotected unlock-before-rename interval. Admission files share the reserved
writer-control namespace and must never be deleted/replaced. External-file
projects without this opt-in retain the ordinary adjacent-lock protocol.

A refused move leaves its source and the caller's history unchanged. A process
interruption before/after the atomic sibling rename leaves the complete directory
at its source/destination; OS locks release on process death. The library owns
hidden-directory retention/recovery and permanent deletion. Physical power-loss
durability and noncooperating directory/control-file replacement races remain
outside this advisory protocol. Windows library integration and deterministic
close/rename interruption tests exercise this protocol; macOS review and remote
filesystem behavior require separate receipts.

`autosave::save(path, project, keep)` writes into `<path>.autosave`, with increasing
20-digit snapshot numbers and no clock dependency. Retention admits 1..=1000.
Only canonical regular snapshot files participate; unrelated files, symlinks
and directories are preserved. Enumeration caps at 10,000 entries. Publication
never overwrites a snapshot and happens before pruning; pruning errors are
returned while the complete new snapshot remains. Autosave directories cannot
be symlinks. A crash can leave a temporary file, an extra complete snapshot, or
an incomplete pruning pass; no partial snapshot is promoted.

## Streaming output

`atomic::StreamingStage::new(destination, overwrite, max_bytes)` creates a private
same-directory `.vibecut-export-*` temporary file. `writer()` yields the sole
bounded seekable `StreamingWriter`, including media's mux-sink interface. Writes
and seeks reject positions beyond the authorized maximum before touching bytes.
`finalize()` syncs and restores prior permissions, yielding `FinalizedStage`;
only that type exposes `publish()`. `Overwrite::Refuse` uses atomic no-clobber
publication, including a competing destination created after admission.
`Replace` is explicit authorization to replace the target at publication.

Dropping or failing a stage removes its temporary file and leaves existing bytes
untouched. `AtomicFs` injects sync/publication failure for boundary tests. A crash
may leave an orphan temporary file; it is never promoted, reused or automatically
swept. The caller may present cleanup after restart. Destination symlinks and
nonregular files are refused. Callers serialize writes and close their mux writer
before finalizing; cancellation arbitration belongs to their execution owner.
Parent-directory sync remains best effort after commit: there is no fallible
post-publication result. Hardware power loss, racing parent replacement and
Windows runtime behavior remain unverified.

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
A unit test injects failure after a partial staging write to verify original
bytes and cleanup. This establishes application-level behavior on the tested
filesystem, not survival of physical power loss. No performance claim is made.

## Optional project metadata

`WriterLease::metadata` reads `<canonical project filename>.meta.json`, bounded to
512 KiB, with metadata schema version 3. Missing, unreadable, corrupt, oversized or
unsupported metadata defaults without blocking the project. `Metadata` contains
created/modified Unix milliseconds, the current `ViewState` and up to 256 scene-ID-keyed views (positive zoom at most 1000, horizontal
scroll 0–1e9 logical pixels, nonnegative integer-tick playhead). Nonfinite values
are refused. This sidecar is outside the editable project envelope and does not
change its schema. Thumbnail is Missing, Unavailable or Ready with 1..=128-pixel dimensions and
exactly width × height × 4 sRGB RGBA bytes (at most 64 KiB). Invalid dimensions,
byte counts and oversized sidecars default on read and refuse on write. No image
decoder runs on metadata. The 8-character display ID is derived from the project
ID by engine, never duplicated here. No project index is stored here.
`read_metadata(path)` supports bounded read-only catalog display; mutations remain
under WriterLease. `WriterLease::validate` checks its baseline and stable lock
identity without requiring a project file to exist.

`WriterLease::save_metadata` validates then atomically replaces the sidecar under
the existing canonical project lease and content precondition. Sidecar symlinks
are refused. Project writer destinations reserve the `.meta.json` suffix, including
case variants and canonical aliases, so saving a project cannot replace another
project's metadata. A sidecar write error is returned; it does not roll back a
previously successful project publication. Callers acknowledge a save only after
both writes succeed. `tests/it/metadata.rs` covers bounds, fallback, native paths,
symlink refusal, external project replacement and round-trip independence.

## Item effects

`effect_definitions()` exposes the admitted effect names and scalar parameter bounds,
defaults and animation capability for UI consumers. `Item.effects` is an ordered stack of at most 64 visual `EffectInstance`s. Instance
IDs are item-local ASCII animation identifiers (1–64 bytes); duplicates refuse.
`EffectParams::Blur` stores finite intensity 0–100 (default 15); `enabled` bypasses
an instance. Audio items refuse visual effects. Evaluation/device work belongs to
render/the desktop app. Animated overrides use the existing `Animations` map and
`Param` storage at `effects.<instance-id>.params.intensity`, with scalar channels
only. No second keyframe store is introduced. Removing an effect removes its
animation paths; undo restores both. Schema 4 adds the required stack and refuses
older envelopes through the existing typed version errors.

`EffectParams::set_scalar` validates a registered scalar key and value before
replacing the static parameter; unsupported keys and invalid values retain the
previous parameter. Definition metadata and typed parameter validation share the
project owner.

## Masks

`Item.masks` stores an ordered vector of project-owned `Mask` values. IDs are
unique within the item and use the animation-path identifier grammar. Empty or
whitespace-only text masks are inactive while retaining their editable data. The vector
admits 1,024 entries for resource safety; presentation limits do not define the
model. Audio items refuse masks. Modes are None/Add/Subtract/Intersect/Lighten/
Darken/Difference; render owns their coverage math. Commands/history belong to
engine. Schema 16 requires timeline transitions, scene arrays, per-item mask vectors, `blendMode`, text style and typography, audio control fields (audio source is optional, default upload) optional imported cue style and optional clip `fades` (`offset` defaults to 0); schema 15 and earlier refuse with `TooOld`.

Shapes include split, cinematic band, rectangle, ellipse, heart, diamond, star,
text and a cubic `MaskPath`. Text masks currently persist content and font size
and render with bundled Inter Regular; richer typography uses the shared text
contract when available. Normalized centers are offsets from the placed item
center, width/height are item fractions, rotation is degrees and uniform scale
multiplies shape size. Path positions/tangent offsets use normalized item space;
fill closes open paths. Feather, expansion, stroke width and text size use
project-canvas pixels. Color is the existing sRGB RGBA8 `Color`; opacity is 0..1.

IDs, finite ranges, scalar animation types and duplicate IDs validate with the
whole candidate. Paths admit 4,096 vertices and bounded normalized coordinates;
text admits 4 KiB and 1..4096-pixel font sizes. `masks.ID.params.KEY` uses the
existing `Animations` owner for center_x/center_y, width/height, rotation, scale,
feather, expansion, opacity and stroke_width. Values evaluate in item-local time;
`Mask::at` clamps animated overshoot to the static admissible range. No second
keyframe store or path interpolator is introduced.

## Blend selection and opacity

`Item.blend_mode` persists as required `blendMode` using the 17 kebab-case IDs in
`BlendMode::ALL`. Normal is the creation default; audio items refuse a non-normal
blend. The render owner documents the linear-light formulas. Opacity has one
owner: the existing scalar `opacity` animation channel. Missing channels mean
unity; constant channels and keyframed channels share persistence and trim/retime.

`atomic::admit_destination` exposes the existing reserved writer-control/native
filename admission to background publication consumers before they stage data.
Source and active-project identity checks remain with the execution owner.
`metadata_path` supplies that owner with the native sidecar name.

## Video transitions

`Timeline.transitions` owns first-class `Transition` entities with project-wide
unique `TransitionId`, track, outgoing/incoming item IDs, kind, output start,
duration and `TransitionAlignment` (start, center, end, custom). Both items must
be adjacent and touching on the same video track and contain video or generated
solid overlays. `TRANSITION_DEFINITIONS` is the admitted catalog: cross dissolve,
dip to black and dip to white in the Dissolve family. Unknown kinds return
`UnsupportedTransition` with a bounded kind name, including during file ingestion.

`Transition::bounds` intersects the two clip extents with real source handles:
the incoming source in-point bounds the time before the cut and the outgoing
source's remaining known duration bounds the time after it. Unknown outgoing
source duration permits no extension after the cut. Generated overlays have no
media-handle constraint. `set_duration` clamps positive ticks to those bounds,
following alignment; an odd centered duration puts floor(duration/2) before the
cut. Custom duration changes keep the span's centre, clamped to stay over the cut.
Zero duration/handles and invalid spans refuse. Transitions on one track cannot
overlap, including multiple transitions on the same cut. Validation never repairs
untrusted persisted data. Edit-time revalidation belongs to [engine](../engine/README.md#video-transitions).
