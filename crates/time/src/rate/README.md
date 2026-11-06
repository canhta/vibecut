# Frame rates

- `FrameRate` constants (`FPS_23_976` through `FPS_120`), `FrameRate::new`,
  `frame_at`, `tick_of`, `snap` and `snap_nearest`.
Admitted frame rates are positive rationals with `i64` parts; ordering uses exact
`i128` cross products, including at the maximum admitted parts; the standard
broadcast and NTSC rates divide the tick rate exactly. Audio sample rates are
positive `i64` values; 44 100, 48 000 and 96 000 Hz are exact. Floating seconds are
accepted only at display/provider boundaries and must be finite.
