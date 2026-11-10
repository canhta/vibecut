# Panels

Interface: `AssetsTab`, `ElementKind`, `PropertiesTab`.

Presentation registries for the assets and properties panels: the tab order, labels,
icons and persisted ids live here once so tests and the preference store agree without
a second list.

Two resizable columns and a horizontal divider start at the measured editor defaults
(582, 486 and 447 px of the frame's content). Drags clamp assets and properties to
15–40%, preview to 30–85% and timeline to 15–70%; Reset restores the defaults.

The assets panel has an icon-over-label strip for Media, Audio, Text, Effects and
Captions, plus a per-tab sub-navigation column. Unsupported tabs and categories stay
hidden. Media owns the live library, Audio the saved and provider sounds library, Text
inserts text, Effects lists the engine catalogue, Captions mounts the subtitle editor.

A single selection shows that element type's properties tabs; the last tab is remembered
per type. Text, video with and without source audio, image, sticker, graphic, audio and
effect have their own tab sets. Image currently means a source video stream reporting
one frame; sticker and effect sets are unreachable from a real project until standalone
item contracts exist.

Clamp values: `preferences::PANEL_MIN`, `PANEL_MAX`, `PREVIEW_MIN`, `PREVIEW_MAX`,
`TIMELINE_MIN`, `TIMELINE_MAX`. Design: [measured frame](../../../../docs/product/ui/editor.md#measured-frame).
