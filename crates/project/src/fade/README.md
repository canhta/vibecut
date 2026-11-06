# Clip fades

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
