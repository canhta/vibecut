# Assets

## Effects panel

The Effects tab lists `project::effect_definitions()` as draggable tiles. Each tile previews through the engine `EffectThumbnails` job/cache: the CPU reference compositor headlessly, the playback GPU adapter and its byte budget in native startup. Pending, failed and ready states stay visible; a failed tile offers Retry. A tile drops onto a video, image, sticker or graphic clip, or `+` adds it to the selected clip.

## Transition catalog

The Transitions tab reads `project::TRANSITION_DEFINITIONS`; names and family navigation use the app catalog, and search filters the current family. Cards follow the measured 88 px grid, 15 px gaps, 116 px row pitch. Hover or keyboard focus plays a 48 px preview at 12 frames/s through render's transition evaluator; idle tiles keep a still frame, failed previews offer Retry, theme changes replace textures. The private `transition` module owns previews, `view/transition` the catalog, `workflow/transition` target resolution.

A card or its plus applies to two adjacent selected video clips, or to the cut under the playhead. A card can also be dropped on the visible shared edge (12 px capture each side), which highlights before release. Every application is `transition.add` with a centered 0.5 s duration clamped by project's source-handle rules. Missing adjacency, exhausted handles, locked tracks, duplicate cuts and overlapping transitions give localized refusals. The plus is disabled without a valid selection. Escape, leaving the tab, focus loss, a changed snapshot or a missed or reversed drop publishes nothing. One release is one history step.
