# Panels

Two resizable columns and a horizontal
divider start at the UI design's 25/50/25 and 50/50 proportions. Drags clamp
assets/properties to 15–40%, preview to 30–85% and the timeline to 15–70%, so no
region reaches zero. The assets panel has a horizontal icon-over-label strip
for Media, Audio, Text, Effects and Captions, followed by a per-tab sub-navigation
column. Unsupported tabs and categories remain hidden. Media owns the live
library, Audio owns the saved/provider sounds library, Text inserts text and
Effects lists the engine catalogue. A single
selection shows that element type's properties tabs, and the last tab is
remembered per type. Text, video with/without source audio, image, sticker,
graphic, audio and effect have the reference tab sets in `src/panels.rs`; image
currently means a source video stream reporting one frame, and sticker/effect
items await standalone item contracts before those two sets are reachable from a
real project. The effect-stack contract is attached to existing visual items,
whose Effects tabs are already available; the stack controls and the Effects
asset view belong to their feature. Captions remains a separate asset-view
mount point for the subtitle editor. English/Vietnamese, theme, panel proportions,
