# Ticks

- `TICKS_PER_SECOND`, `Tick`, `Tick::from_seconds_f64`, `Tick::from_units`,
  `Tick::from_rational` and their inverse conversions.
- `Tick::checked_scale(num, den, rounding)` scales by a positive rational with
  checked i128 intermediate arithmetic and the same explicit rounding policy.
