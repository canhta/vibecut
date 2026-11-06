# Baseline

A main-revision baseline is an explicit pinned file, never candidate metrics
relabelled. `--write-baseline` records the heuristic metrics plus the complete
metric identity: source, transcript, resolved pipeline settings, ordered picks
and priorities, `k`, language and energy mode. Run it on the pinned main
revision. `--baseline` loads and validates that file, rejecting a mismatched
label set or any identity mismatch as a typed refusal. A `baseline.json` beside
the labels is loaded automatically; the synthetic fixtures ship one. Use
`--no-baseline` to ignore it. When no baseline is present the report says the
main baseline was not measured. The persisted baseline schema is 2.
