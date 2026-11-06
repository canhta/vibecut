# Animated parameters and retime

Item-relative parameter overrides, keyed channels, Bezier evaluation, clip fades
and retime. Rendering and editing share this evaluator.

| Module | Responsibility | Contract |
|---|---|---|
| `params` | Owns animation paths, typed parameter values and the item animation map. | [Parameters](#parameters) |
| `keyframes` | Owns keyed channels, segments and evaluation. | [Keyframes](#keyframes) |
| `bezier` | Inverts cubic time curves for scalar segments. | [Bezier segments](#bezier-segments) |
| `fade` | Owns clip fade windows and per-sample gain. | [Clip fades](#clip-fades) |
| `retime` | Owns playback-rate mappings and split/trim edges. | [Retime](#retime) |

## Parameters

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

`ParamValue` holds scalar, bool, NFC text or linear-light unpremultiplied RGBA.
RGBA uses four independently keyed scalar `Channel`s, selected by `ColorComponent`;
there is no shared-key-time restriction. Stored RGBA values are in 0..=1 and
composite evaluation clamps overshoot/extrapolation to that range. Input/output is
already linear RGBA; existing byte-color fields are not silently reinterpreted or
converted. Discrete color text stays a held string. `AnimationPath::scale_group()`
identifies the scaleX/Y editor group; both scalar curves remain independent.

Limits: 64 paths per item, 4096 keys per channel, 100,000 keys per project; JSON
budgets below also apply. Stored scalars/handle dv must be finite with absolute
value <= 10^12. Unknown fields/paths, wrong component kinds, unordered/duplicate
keys, invalid handles and over-budget input refuse. Raw channel deserialization
bounds its key sequence; map deserialization refuses duplicate paths. Candidate
mutations remain subject to whole-project validation. Numeric evaluation uses
allocation-free scalar/rgba arithmetic and binary channel lookup; no throughput
or renderer output claim is made.

## Keyframes

`Channel::new(base, keys, before, after)` validates strictly increasing unique
key times and homogeneous values. Each scalar key's `Segment` governs the segment
leaving it: Step, Linear or Bezier. Exact endpoints return their stored values.

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
round trip. Key times are item-relative output ticks: a speed change stretches them
by `old / new`, trims shift them by the start edge's output move, and splits move the
right piece's keys by the left piece's output length.
Moving one key leaves explicit handles unchanged and clamps them when evaluated.

## Bezier segments

Bezier handles contain relative signed `dt` ticks and `dv` in scalar value units;
incoming dt <= 0, outgoing dt >= 0. At evaluation dt is clamped to the adjacent
span. Missing handles use (span/3, delta/3) secants. The cubic time curve is inverted
before evaluating its absolute value curve, including nonzero handles between
equal endpoint values. Scalar extrapolation before/after is independently Hold
or the edge/neighbor Linear secant; a single key holds.

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
reproduce the original gain at every sample. A fade without `offset` loads as uncut. Curves: `sin(x*pi/2)` (-3 dB midpoint), `x`, and `(e^(4x)-1)/(e^4-1)`.

## Retime

Video and audio items carry `retime { speed, maintain_pitch, window? }` (default
1x, pitch preserved); every other kind must keep the default. An unsplit item's
output duration is `floor(source / speed)`; a rate change keeps its source range.
Items shorter than one output tick refuse. `Item::source_time_at` (floor) and
`output_time_of` (ceiling) own the mapping used by render and edits.

`Retime.window`, when present, retains the original source processing window,
output offset and slice duration. `source_time_at` evaluates that original mapping
at offset plus local output time, retaining fractional-tick phase. Split pieces
partition output time exactly, including the original final tick, while
`source_range` records their source cut. The original window also lets audio
continue the same interpolation/WSOLA processing across cuts. Validation checks
positive contained slices, source mapping consistency and the original source
window against the asset duration. A new speed establishes a new mapping; a
pitch-only change retains it. Shortening a retained slice preserves its mapping.

`split_at` keeps both original output edges and places the new cut exactly at the
requested tick. A piece with an empty source range refuses. `trim_start` and
`trim_end` keep the opposite edge fixed; unsplit trims may displace the requested
edge inward by less than one source tick's output duration (up to 100 ticks at
0.01x), while shortening a retained slice keeps the exact output tick. Pitch
policy is data only; render applies it. Frame interpolation uses the nearest
earlier source frame; the project holds no blend policy.
