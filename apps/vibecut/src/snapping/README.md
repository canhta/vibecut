# Snapping

Interface: `SNAP_PIXELS`, `PREVIEW_SNAP_PIXELS`, `ROTATE_SNAP_STEP`,
`ROTATE_SNAP_DEGREES`, `SnapSource`, `SnapCandidate`, `SnapMatch`, `SnapOutcome`,
`threshold_ticks`, `snap_point`, `snap_delta`, `closest`.

Ranks timeline snap candidates and converts the 10 logical-pixel capture threshold into
ticks. The app owns this policy; engine commands receive only the resulting tick
position. Candidates are ranked by the smallest pixel distance, then by a stable source
order that is deliberately independent of scene iteration order, so a tie cannot change
between frames or project reloads. The preview canvas uses the same ranking rule with its
own measured 5 px threshold.
