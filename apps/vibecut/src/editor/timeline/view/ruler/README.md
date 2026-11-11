# Timeline ruler

Interface: `State::paint_ruler`, `State::paint_playhead`, `RulerConfig`, `ruler_config`,
`ruler_text`, `ruler_label`, `snapped_seconds`, `optimal_ruler_interval`,
`tick_dividing_label` and `divides`.

Draws the adaptive timecode ruler and its tick labels. `ruler_config` picks frame or second
major and minor intervals from zoom and project fps, with labels at least
`RULER_LABEL_SPACING` (160 px) and ticks at least `RULER_TICK_SPACING` (18 px) apart.
Whole-second labels split into ten ticks when they fit; finer labels use frame ticks that
divide the label interval. Major labels show `MM:SS` (or `H:MM:SS`) while labels fall on
whole seconds, and time's shared DF/NDF frame timecode once labels are finer than one
second. Right-clicking the ruler opens the next/previous bookmark entries, enabled and
dispatched through the
[action registry's bookmark lookup](../../../../actions/README.md#bookmark-navigation).
