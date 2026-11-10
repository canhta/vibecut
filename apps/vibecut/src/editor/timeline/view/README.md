# Timeline view

Interface: `show`, `show_scenes`, `Ink`, `item_label`, `track_height`,
`add_track_command`, `TOOLBAR_ACTIONS`.

Renders the timeline surface: clips, keyframe lanes, the ruler, scenes, toolbar and the
marker editor. Clip bands come from the cache-only display query in
[visuals](../visuals.rs): thumbnail strip tiles, waveform bars, wait placeholders, typed
markers and the `> 0 dBFS` tip. The view returns actions and writes no project.
