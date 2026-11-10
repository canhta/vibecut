# Editor presentation

Interface: `Editor`, `AppCommand`, `PreviewZoom`, `Inspection`, `WidgetState`,
`AssetInspection`, `AssetState`, `PreviewAxis`, `PreviewGuide`, `ShortcutRow`,
`ShortcutTab`, `ShortcutsInspection`, `TextCaret`, `Client`, `Sounds`, `SoundsMode`,
`SoundsProblem`, `SoundsView`, `CropAspect`, `CropInspection`.

`Editor` is the public facade over the areas. `Editor::ui` runs without a window or
device. `activate` maps stable widget IDs to engine `execute` for every project edit
and to app-owned transport, preferences and selection; `dispatch` takes one typed
`AppCommand` for the app-owned commands; `inspect` exposes
enabled/focus/rect/current view, playing, position, selection, zoom, snapping and the
active snap-indicator target. The project is the Session's immutable snapshot. Every
interactive control, including each timeline item and trim handle, exposes a
locale- and layout-independent `author_id` through AccessKit.

Each panel receives its full slot including its header strip; the frame supplies only
outer insets and draggable gutters. One write context is lent to one area for one
frame: its edit entry pushes the selection, applies the command, reports the failure
and resyncs the preview once per committed change, and cross-area intentions are
queued and drained by the frame after that area returns. A read-only context carries
session, selection, locale and theme into areas.

The assets, properties, timeline and captions areas and the player are folder
interfaces with private view and workflow implementations; the export and model
manager dialogs own their state and workflow. The player's Crop modal edits a
transient `project::Crop` through the shared `crate::services::crop` control, locked
to the selected ratio (Custom resizes freely, Original keeps the source shape), and
writes it with one `timeline.set_item_param` patch on Apply, so closing never mutates
the project.

`Playback::new` injects `Sources`, which creates render's `SourceProvider`;
production uses `FileSources` as export does. `AudioPort::fill` accepts a headless
device callback.

Responsibilities and layers: [structure](../../../../docs/engineering/structure.md).
