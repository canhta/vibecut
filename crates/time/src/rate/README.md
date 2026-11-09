# Frame rates

Admitted frame rates are positive rationals with `i64` parts. Ordering uses exact `i128` cross products, including at the maximum parts. The standard broadcast and NTSC rates divide the tick rate exactly. Audio sample rates are positive `i64` values, and 44 100, 48 000 and 96 000 Hz are exact. Floating seconds are accepted only at display and provider boundaries and must be finite.
