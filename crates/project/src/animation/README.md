# Animated parameters and retime

Item-relative parameter overrides, keyed channels, Bezier evaluation, clip fades and retime. Rendering and editing share this evaluator.

## Parameters

`Item.animations` is the single `Animations` map of parameter overrides. A missing path leaves the consumer's static property in use. A present `Param` owns its explicit base value and keys, even when no keys remain; editing static item content does not rewrite it. `Param::new(value)` creates a static override. Validate edited candidates through `Project::new`.

`Animations::value_at(path, local_tick)` is the shared evaluation entry point. `None` means absent, not zero.

`AnimationPath::new` admits `transform.positionX/Y`, `transform.scaleX/Y`, `transform.rotate`, `transform.flipX`, `transform.flipY`, `opacity`, `volume`, `muted`, `color`, `stroke.color`, `background.color`, `background.paddingX/Y`, `background.offsetX/Y`, `background.cornerRadius`, `crop.x`, `crop.y`, `crop.auto`, `params.<name>`, `effects.<id>.params.<name>` and `masks.<id>.params.<name>`. The fixed vocabulary is `BuiltinPath::ALL`: `BuiltinPath::as_str` is its persisted spelling and `BuiltinPath::parse` / `AnimationPath::builtin` round-trip that spelling unchanged. `AnimationPath::from(BuiltinPath)` is the typed construction used outside this crate. Extension ids and names are 1..=64 ASCII letters, digits, underscore or hyphen; complete paths are at most 160 bytes. Extension namespaces reserve animation data only; later owners resolve ids against their admitted implementations.

`volume` is scalar dB (consumers clamp overshoot to -60..20). `muted` requires a held bool.

`crop.x` and `crop.y` are scalars in `project::Crop` units (0..=1) that override the static crop position of a video or image item; width and height stay static and are not keyable. `crop.auto` is a held bool (`Step` only) marking where auto reframe owns the position; it never renders. Any of the three is refused on an item without a static crop, and every `crop.x` / `crop.y` base and key must lie in `[0, 1 - width]` / `[0, 1 - height]`. Render evaluates both channels with `Crop::at` and clamps extrapolation into the same bound. Built-in numeric paths require scalars. Color paths accept linear RGBA composites or discrete text. Extension paths admit all parameter kinds.

`ParamValue` holds scalar, bool, NFC text or linear-light unpremultiplied RGBA. RGBA uses four independently keyed scalar `Channel`s selected by `ColorComponent`, with no shared-key-time restriction. Stored RGBA values are in 0..=1 and composite evaluation clamps overshoot and extrapolation to that range. Discrete color text stays a held string. `AnimationPath::scale_group()` identifies the scaleX/Y editor group; both curves stay independent.

Limits: 64 paths per item, 4096 keys per channel, 100,000 keys per project, plus the JSON budgets in persistence. Stored scalars and handle `dv` are finite with absolute value <= 10^12. Unknown fields or paths, wrong component kinds, unordered or duplicate keys, invalid handles and over-budget input refuse. Raw channel deserialization bounds its key sequence; map deserialization refuses duplicate paths. Scalar evaluation is allocation-free; a discrete text value clones its stored text.

## Keyframes

`Channel::new(base, keys, before, after)` validates strictly increasing unique key times and homogeneous values. Each scalar key's `Segment` governs the segment leaving it: Step, Linear or Bezier. Exact endpoints return their stored values.

`TangentMode::{Auto, Aligned, Broken, Flat}` are persisted labels only. They do not calculate, flatten or mirror handles and never change evaluation; no UI may offer them as easing choices. Explicit dt/dv handles are authoritative.

Bool and text keys require Step and Hold extrapolation. Before the first key the explicit base applies, then each key holds until the next. A scalar channel before its first key follows its configured edge extrapolation.

Key times are signed i64 ticks relative to item start. Negative and out-of-window keys are retained: trims and splits translate anchors rather than discarding or resampling curves. `Animations::shift` checks overflow and preserves exact tick differences and handle offsets; engine's range changes call it once. Trim-end and item moves retain local times.

`Animations::retime(num, den)` stretches key and handle times by a positive rational through `Tick::checked_scale`, rounding to the nearest tick with ties away from zero. It refuses collisions and overflow atomically. Integer stretches of frame-aligned 29.97-fps keys stay exact. General rational stretches may round each timestamp by up to half a tick and are not invertible. Key times are item-relative output ticks: a speed change stretches them by `old / new`, trims shift them by the start edge's output move, and splits move the right piece's keys by the left piece's output length.

Moving one key leaves explicit handles unchanged and clamps them at evaluation.

## Bezier segments

Handles hold relative signed `dt` ticks and `dv` in scalar value units. Incoming dt <= 0, outgoing dt >= 0. At evaluation dt is clamped to the adjacent span. Missing handles use (span/3, delta/3) secants. The cubic time curve is inverted before evaluating the value curve, including nonzero handles between equal endpoint values. Scalar extrapolation before and after is independently Hold or the edge or neighbor Linear secant. A single key holds.

## Easing

`EasingCurve([x1, y1, x2, y2])` is a normalized cubic Bezier over one segment: x is the fraction of the segment time (`0..=1`), y the fraction of its value change (overshoot allowed). `Channel::set_easing(time, curve)` expands it into the explicit outgoing handle of the key at `time` and incoming handle of the next key, `dt = x * span` rounded to whole ticks and `dv = y * delta`. A flat segment borrows the nearest non-flat segment's span (else 1) as `delta`. The straight line stores `Segment::Linear` and no handles; every other curve stores `Segment::Bezier`. Both keys' tangent label becomes `Broken` while they hold a handle, else `Flat`. A last key, a non-scalar channel or an invalid curve refuses and leaves the channel unchanged. `Channel::segment_curve(time)` reads the segment back as a normalized curve (`None` for hold or the last key).

`EasingPreset` names the six built-ins (smooth, ease_out, ease_in, in_out, pop, linear). The control points are the permitted implementation reference's table (`graph-editor/easing-presets.ts` lines 13-18) stored as data in `EasingPreset::curve`.

## Clip fades

Audio and video items carry `fades {fade_in, fade_out}` (default none). Each is a `Fade {duration, offset, curve}`: the whole fade length in ticks from the edge it was set on, the ticks of it a split cut away beyond this clip's edge (`0..=duration`, default 0), and a `FadeCurve` (`equal_power` default, `linear`, `exponential`). Only the window `duration - offset` is audible; that window, not the stored `duration`, is what the clip length clamps. A fade without `offset` loads as uncut.

Fades are persisted data, not animation. They multiply the volume curve and are never keyframed. Validation refuses negative durations and an offset outside `0..=duration`. Durations beyond the clip are legal because trims shorten clips without rewriting them.

`Fades::set` clamps one edge to the clip duration minus the other edge's window and starts a fresh fade (offset 0). `Fades::split(start, at, end)` returns the two pieces' fades: each keeps the whole shape of any fade reaching it and records the part the other piece holds; an empty window is no fade.

`Fades::spans(start, end, rate)` converts ticks to samples once, rounding both absolute endpoints to nearest, so a piece lands on exactly the samples the whole clip had. Windows clamp to the clip and overlapping stored windows multiply. `FadeSpans::gain(sample)` is the single per-sample gain for preview and export: exactly 0 at the clip edge where a fade starts, exactly 1 in the body. Both pieces of a split reproduce the original gain at every sample. Curves: `sin(x*pi/2)` (-3 dB midpoint), `x`, and `(e^(4x)-1)/(e^4-1)`.

## Retime

Video and audio items carry `retime { speed, maintain_pitch, reverse, window? }` (default 1x, pitch preserved, forward). Every other kind must keep the default. An unsplit item's output duration is `floor(source / speed)`; a rate change keeps its source range. Items shorter than one output tick refuse. `Item::source_time_at` (floor) and `output_time_of` (ceiling) own the mapping used by render and edits.

`Retime.window`, when present, retains the original source processing window, output offset and slice duration. `source_time_at` evaluates that original mapping at offset plus local output time, keeping fractional-tick phase. Split pieces partition output time exactly, including the original final tick, while `source_range` records their source cut. The original window lets audio continue the same interpolation and WSOLA processing across cuts. Validation checks positive contained slices, source mapping consistency and the original source window against the asset duration. A new speed establishes a new mapping; a pitch-only change keeps it. Shortening a retained slice preserves its mapping.

`reverse` plays the visible source range backwards: output tick `p` of the window shows source tick `we - 1 - floor(p * speed)`, so the first output tick shows the last source tick. A reversed piece's visible range is derived from its window and output slice (`reverse_visible`), so splits and trims keep the mapping without a second copy of the range. `Item::set_reversed` flips a windowed piece in place: its window stays and its output slice mirrors inside it (`[o, o + d)` becomes `[T - o - d, T - o)`), so the same source span plays backwards with the same span and duration, within the floored rounding of the mapping. Flipping back restores the slice exactly.

`split_at` keeps both original output edges and places the cut exactly at the requested tick. A piece with an empty source range refuses. `trim_start` and `trim_end` keep the opposite edge fixed. Unsplit trims may displace the requested edge inward by less than one source tick's output duration (up to 100 ticks at 0.01x); shortening a retained slice keeps the exact output tick. Pitch policy is data only; render applies it. Frame interpolation uses the nearest earlier source frame; the project holds no blend policy.

## Speed curves

`Retime.curve` (`SpeedCurve`) replaces the constant rate with a piecewise-linear source/output time map: 2 to 8 keys `(output, source)` in ticks from the start of the retime window, the first `(0, 0)`, both coordinates strictly increasing, every stretch between 0.01x and 5x. A curve item keeps `speed` at 1x, is never reversed and its window source duration equals the curve's source span; validation refuses anything else. The curve is stored inline (a fixed array, so `Retime` stays `Copy`) and serializes as the key list; a constant rate omits the field. `Retime::output_duration`, `source_offset` and `output_offset` are the single dispatch between the constant ratio and the curve, used by `source_time_at`, `output_time_of`, `duration`, the window validation and the visible-range derivation, so video lookup, audio and tooling share one mapping. Lookups are exact integer arithmetic (floor forward, ceiling inverse); the end stretches extend linearly outside the curve for handle reads. A split or in-range trim keeps the whole curve and slices the output window (`window.offset`, `window.duration`), so each half reproduces the original mapping tick for tick; a trim that would extend past the curve and a reverse refuse. `SpeedCurve::from_segments` builds a curve from editable stretches (`SpeedSegment { span, speed }`, spans in thousandths of the source summing to 1000, each stretch playing for the ceiling of its source over its speed); `SpeedPreset` names four ramps (`slow_in_fast_out`, `fast_in_slow_out`, `slow_middle`, `fast_middle`). The preset shapes and ids are ours; the reference editor's preset names and curve shape are not measured.
