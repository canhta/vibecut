# Bezier channels

`Channel::new(base, keys, before, after)` validates strictly increasing unique
key times and homogeneous values. Each scalar key's `Segment` governs the segment
leaving it: Step, Linear or Bezier. Exact endpoints return their stored values.
Bezier handles contain relative signed `dt` ticks and `dv` in scalar value units;
incoming dt <= 0, outgoing dt >= 0. At evaluation dt is clamped to the adjacent
span. Missing handles use (span/3, delta/3) secants. The cubic time curve is inverted
before evaluating its absolute value curve, including nonzero handles between
equal endpoint values. Scalar extrapolation before/after is independently Hold
or the edge/neighbor Linear secant; a single key holds.
