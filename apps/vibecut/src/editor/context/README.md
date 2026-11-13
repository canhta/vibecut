# Editor context

The one read-only context and the one write context every editor area receives.

`Context` carries the session (the only owner of the item selection, read through `Context::selection`), the nested selection scopes, locale and appearance, the shared transport position, the registered actions the editor refuses, the armed eyedropper and the decoded-thumbnail owner. `Thumbnails` uploads one decoded image on first use so several library records that share content share the GPU texture. `visuals` holds the cache-only clip display contract (`VisualQuery` with its strip, waveform and request states) the timeline reads. `VisualQuery::availability` reports whether an asset's source file is still on disk, as the engine's media status answers it, so the timeline draws a missing-media clip without probing a path. `single` is the shared one-selected-item read and `closest` the snapping rank the timeline and the preview canvas share.

`Port` is the session-only door an area edits through (slice plan: [structure](../../../../../docs/engineering/structure.md#editor-area-ports)). It holds the open session, a `Read` snapshot (byte budget, preferences, playhead, clip rectangles, clock, the frame's insertion, caption-bank and edit-ready answers, the selected transition) and the `Note`s the area leaves for the frame: error line, toast, preview resync, selection reset or single selection, leaving the transition scope, a staged draft frame, preference write or adoption, widget record and cross-area `Request`. Its entries apply typed engine commands and transactions, write the selection through the one `select` entry, report the failure and resync the preview once per committed change; a preference write that must refuse in place validates against the snapshot before it leaves its note. `View` is the read-only projection a workflow step reads through the port: session (and its selection), preferences, playhead, edit readiness and the selected transition. The frame builds it from disjoint `Editor` fields, so an area that takes a `Port` never names `Editor`; the frame applies the notes in order after the area returns.

Layers and import direction: [structure](../../../../../docs/engineering/structure.md).

The port can adopt a fully validated preference candidate after its related project
command succeeds, without making an independent preference-history entry.

`CacheUi` projects typed maintenance results and visual failures into Settings. `CacheAction`
travels through `Request` to the frame; cache custody and worker lifetime remain with Media.
These human desktop requests neither edit the session nor enter the agent command registry.
