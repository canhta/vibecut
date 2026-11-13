# Editor context

The one read-only context and the one write context every editor area receives.

`Context` carries the session (the only owner of the item selection, read through `Context::selection`), the nested selection scopes, locale and appearance, the shared transport position, the registered actions the editor refuses, the armed eyedropper and the decoded-thumbnail owner. `Thumbnails` uploads one decoded image on first use so several library records that share content share the GPU texture. `visuals` holds the cache-only clip display contract (`VisualQuery` with its strip, waveform and request states) the timeline reads. `single` is the shared one-selected-item read and `closest` the snapping rank the timeline and the preview canvas share.

`Desk` is lent to one area for one frame. Its entries apply typed engine commands, write the selection through the one `select` entry, report the failure, resync the preview once per committed change and record preference edits in the app's own history; `request` queues a cross-area intention the frame drains after the area returns. `View` is the read-only projection a workflow reads while it holds the desk: session (and its selection), preferences and playhead.

`bench` mounts one area alone in an offscreen harness for area tests. Layers and import direction: [structure](../../../../../docs/engineering/structure.md).

The write context can adopt a fully validated preference candidate after its related
project command succeeds, without making an independent preference-history entry.

`CacheUi` projects typed maintenance results and visual failures into Settings. `CacheAction`
travels through `Request` to the frame; cache custody and worker lifetime remain with Media.
These human desktop requests neither edit the session nor enter the agent command registry.
