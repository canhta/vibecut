# Assets

Interface: `State`, `show`, `Action`, `prepare`, `sounds`.

The area owns its navigation drafts and the prepared workflow projection; `show`
consumes the editor's read-only context and returns `Action` values the frame routes.
`prepare` builds the read-only projection the view draws.

## Effects panel

The Effects tab lists the library-shelf entries of `project::effect_definitions()` as draggable tiles. Each tile
previews through the engine `EffectThumbnails` job and cache: the CPU reference
compositor headlessly, the playback GPU adapter and its byte budget in native
startup. Pending, failed and ready states stay visible; a failed tile offers Retry. A
tile drops onto a video, image, sticker or graphic clip, or `+` adds it to the
selected clip.

## Transition catalog

The Transitions tab reads `project::TRANSITION_DEFINITIONS`; names and shelf navigation use the app catalog, and search filters the current shelf. Cards follow the measured 88 px grid, 15 px gaps, 116 px row pitch. Visible hover or keyboard focus plays a 48 px preview at 12 frames/s through render's procedural thumbnail evaluator; idle tiles retain their 176 px midpoint raster. Only an active, visible tile requests timed repaints; window focus loss restores its idle frame. Failed previews offer Retry, theme changes replace textures, and the preview's accessible value reports the progress of its rendered frame. The all-shelves view keeps the measured heading space without repeating the tab name; a selected shelf names its results. The private `transition` module owns previews, `view/transition` the catalog, `workflow/transition` target resolution.

A card or its plus applies to two adjacent selected video clips, or to the cut under the playhead. A card can also be dropped on the visible shared edge (12 px capture each side), which highlights before release. Every application is `transition.add` with a centered 0.5 s duration clamped by project's source-handle rules. Missing adjacency, exhausted handles, locked tracks, duplicate cuts and overlapping transitions give localized refusals. The plus is disabled without a valid selection. Escape, leaving the tab, focus loss, a changed snapshot or a missed or reversed drop publishes nothing. One release is one history step.

Media rows, import feedback and marquee selection: [assets workflow](workflow/README.md).
Sounds rows: [sounds](sounds/README.md).

## Shapes

The persisted Stickers tab exposes only locally generated Rectangle and Ellipse cards, using their real silhouettes and existing kit controls. `graphic.insert.rectangle` and `graphic.insert.ellipse` insert five seconds at the playhead through ordinary engine placement and select the returned item. The prepared readiness projection, direct activation and preflush admission share the same predicate, including drafts, transactions and modal work. No unavailable shape variants or network source exist.
