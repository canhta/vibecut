# Label format

One file per video. Paths resolve relative to the label file; absolute paths are used as-is.

```json
{
  "version": 1,
  "source": "interview.mp4",
  "sha256": "<lowercase sha256 of the source bytes>",
  "language": "vi",
  "transcript": "interview.transcript.json",
  "settings": {
    "count": 5,
    "min_duration_seconds": 20.0,
    "max_duration_seconds": 60.0,
    "min_segment_duration_seconds": 2.0,
    "max_segments": 4,
    "overlap_tolerance": 0.5,
    "frame_rate": 30.0,
    "snap_tolerance_seconds": 1.0,
    "silence_rms": 0.01,
    "keywords": []
  },
  "clips": [
    {
      "id": "human-pick-1",
      "priority": "must",
      "segments": [
        {"start": 2540160000000, "end": 5080320000000},
        {"start": 7620480000000, "end": 10160640000000}
      ]
    }
  ]
}
```

`segments` are ordered source-segment lists in play order. Gaps and reordering are allowed. `priority` is `must` or `nice`. Synthetic fixtures replace `source` with `"synthetic": {"duration_seconds": 41.0}`; the harness writes a deterministic silence WAV under `target/clip-eval/synthetic/` and still verifies `sha256`.
