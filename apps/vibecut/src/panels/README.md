# Panels

Two resizable columns and a horizontal divider start at the UI design's 25/50/25 and 50/50 proportions. Drags clamp assets/properties to 15–40%, preview to 30–85% and timeline to 15–70%.

The assets panel has an icon-over-label strip for Media, Audio, Text, Effects and Captions, plus a per-tab sub-navigation column. Unsupported tabs and categories stay hidden. Media owns the live library, Audio the saved/provider sounds library, Text inserts text, Effects lists the engine catalogue, Captions mounts the subtitle editor.

A single selection shows that element type's properties tabs; the last tab is remembered per type. Text, video with/without source audio, image, sticker, graphic, audio and effect have the tab sets in `src/panels.rs`. Image currently means a source video stream reporting one frame; sticker and effect sets are unreachable from a real project until standalone item contracts exist.
