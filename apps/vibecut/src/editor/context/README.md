# Editor context

The one read-only context and the one write context every editor area receives.

`Context` carries the session, selection and scopes, locale and appearance, the shared transport position, the registered actions the editor refuses, the armed eyedropper and the decoded-thumbnail owner. `Thumbnails` uploads one decoded image on first use so several library records that share content share the GPU texture. `visuals` holds the cache-only clip display contract (`VisualQuery` with its strip, waveform and request states) the timeline reads. `single` is the shared one-selected-item read.

`Desk` is lent to one area for one frame. Its entries push the selection, apply typed engine commands, report the failure, resync the preview once per committed change and record preference edits in the app's own history; `request` queues a cross-area intention the frame drains after the area returns. `View` is the read-only projection a workflow reads while it holds the desk: session, selection, preferences and playhead.

`bench` mounts one area alone in an offscreen harness for area tests. Layers and import direction: [structure](../../../../../docs/engineering/structure.md).

The write context can adopt a fully validated preference candidate after its related
project command succeeds, without making an independent preference-history entry.
