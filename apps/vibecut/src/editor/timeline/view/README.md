# Timeline view

Renders the timeline surface: clips, keyframe lanes, the ruler, scenes,
toolbar and the marker editor. Clip bands are painted from the cache-only
display query (`visuals.rs`): thumbnail strip tiles, real waveform bars, wait
placeholders, typed markers and the `> 0 dBFS` tip.
