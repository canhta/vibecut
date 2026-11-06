# Metrics

Each metric is reported per video and aggregated for the selected provider, the
`source: heuristic` baseline at the evaluated revision, and the pinned main
baseline when one is loaded:

- **Must hit rate**: share of `must` picks matched by any suggestion with summed
  source-segment IoU >= 0.5. Intersections and source durations are summed
  pairwise, gaps are excluded, and disjoint pieces are never matched by their
  enclosing envelope.
- **Precision@k**: share of the top `k` suggestions matching any labelled pick.
  The denominator is the number of available suggestions when fewer than `k`.
- **Boundary quality**: share of all suggestion segment edges that do not fall
  strictly inside a captions-owned speech unit or a sentence span.
- **Duration compliance**: share of suggestions within the configured total,
  piece and segment-count bounds.
- **Cost and wall time**: provider upper-bound estimate in USD micros and elapsed
  time per video. Billing is not measured.

Snapping is applied to every suggestion before metrics. A per-suggestion snapping
refusal is counted, keeps the unsnapped ranges for measurement, and is noted in
the report; the whole eval does not abort.
