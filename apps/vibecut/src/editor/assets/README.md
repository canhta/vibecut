# Assets

## Effects panel

The assets Effects tab lists `project::effect_definitions()` as draggable tiles;
each tile shows a live preview scheduled through the engine `EffectThumbnails`
job/cache with the CPU reference compositor headlessly; native startup attaches
the playback GPU adapter and shares playback's byte budget. Pending, failed and ready
states stay visible, and a failed tile offers Retry. A tile can be dragged onto a
video, image, sticker or graphic clip, or added with `+` to the selected clip.

## Transition catalog

The assets-panel Transitions tab reads `project::TRANSITION_DEFINITIONS`; localized
names and family navigation share the app catalog. Search filters the current
family. Cards follow the measured 88 px asset grid with 15 px gaps and a 116 px
row pitch. Hover or keyboard focus animates a bounded 48 px illustrative preview
at 12 frames/s through render's real transition evaluator; idle tiles retain a
still frame, failed previews offer Retry, and theme changes replace their textures.
The private assets `transition` module owns these previews; `view/transition` owns
the catalog presentation and `workflow/transition` resolves targets.

Click a card or its plus with two adjacent video clips selected, or with the
playhead exactly at their cut. Drag a card to the visible shared edge (12 px
capture on either side); the edge highlights before release. Every application
uses `transition.add` with a centered 0.5 s duration, clamped by project's source
handle rules. Missing adjacency, exhausted handles, locked tracks, duplicate cuts
and overlapping transitions carry localized refusal reasons. The plus is disabled
without a valid selection, while dragging remains available when adjacent clips
exist. Escape, leaving the tab, focus loss, a changed project snapshot or a
missed/reversed drop publishes no edit. One successful release is one history step.

`tests/it/transitions.rs` exercises catalog application, drop/cancel boundaries,
localized search, refusal states, undo/redo and persistence using real widgets.
`cargo xtask ui-audit assets-transitions` captures the empty-target catalog at
the reference window size in both themes and densities. Timeline selection and
the transition inspector belong to their respective area owners.
