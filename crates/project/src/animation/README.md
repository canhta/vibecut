# Animated parameters

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

Limits: 64 paths per item, 4096 keys per channel, 100,000 keys per project; JSON
budgets below also apply. Stored scalars/handle dv must be finite with absolute
value <= 10^12. Unknown fields/paths, wrong component kinds, unordered/duplicate
keys, invalid handles and over-budget input refuse. Raw channel deserialization
bounds its key sequence; map deserialization refuses duplicate paths. Candidate
mutations remain subject to whole-project validation. Numeric evaluation uses
allocation-free scalar/rgba arithmetic and binary channel lookup; no throughput
or renderer output claim is made.
