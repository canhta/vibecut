# Assets

Interface: `State`, `show`, `Action`, `prepare`, `media_panel`, `import`, `sounds`,
`music`.

The area owns its navigation drafts, the media selection and thumbnail caches, the
import batch and its last report, and the prepared workflow projection; `show` consumes
the editor's read-only context and returns `Action` values the frame routes. `prepare`
builds the read-only projection the view draws. Toast and clock deadlines stay in the
editor core and are passed to the overlay renderers.

## Effects panel

The Effects tab lists the library-shelf entries of `project::effect_definitions()` as draggable tiles. Each tile
previews through the engine `EffectThumbnails` job and cache: the CPU reference
compositor headlessly, the playback GPU adapter and its byte budget in native
startup. Pending, failed and ready states stay visible; a failed tile offers Retry. A
tile drops onto a video, image, sticker or graphic clip, or `+` adds it to the
selected clip. The tab's preview jobs and drag state live in [`effects`](effects/README.md).

## Transition catalog

The Transitions tab reads `project::TRANSITION_DEFINITIONS`; names and shelf navigation use the app catalog, and search filters the current shelf. Cards follow the measured 88 px grid, 15 px gaps, 116 px row pitch. Visible hover or keyboard focus plays a 48 px preview at 12 frames/s through render's procedural thumbnail evaluator; idle tiles retain their 176 px midpoint raster. Only an active, visible tile requests timed repaints; window focus loss restores its idle frame. Failed previews offer Retry, theme changes replace textures, and the preview's accessible value reports the progress of its rendered frame. The all-shelves view keeps the measured heading space without repeating the tab name; a selected shelf names its results. The private `transition` module owns previews, `view/transition` the catalog, `workflow/transition` target resolution.

A card or its plus applies to two adjacent selected video clips, or to the cut under the playhead. A card can also be dropped on the visible shared edge (12 px capture each side), which highlights before release. Every application is `transition.add` with a centered 0.5 s duration clamped by project's source-handle rules. Missing adjacency, exhausted handles, locked tracks, duplicate cuts and overlapping transitions give localized refusals. The plus is disabled without a valid selection. Escape, leaving the tab, focus loss, a changed snapshot or a missed or reversed drop publishes nothing. One release is one history step.

Media rows, import feedback and marquee selection: [assets workflow](workflow/README.md).
Sounds rows: [sounds](sounds/README.md).

## Shapes

The persisted Stickers tab exposes only locally generated Rectangle and Ellipse cards, using their real silhouettes and existing kit controls. `graphic.insert.rectangle` and `graphic.insert.ellipse` insert five seconds at the playhead through ordinary engine placement and select the returned item. The prepared readiness projection, direct activation and preflush admission share the same predicate, including drafts, transactions and modal work. No unavailable shape variants or network source exist.

## Music section

The Audio tab's Music page consumes the libraries music library. A section switch at
the top of the 120 px navigation column chooses Music or Sound effects; Music holds
Saved, then one chip per source tag. Discover answers anonymously from the open
source, Saved is the offline local library, and the free-text field sends `q` while a
chip sends `tags`. Rows are 48 px on the existing audio pitch: a 32 px cover with a
24 px play/pause overlay, the title, `artist · duration · licence`, the full
attribution, and 24 px bookmark (save) and add buttons. Every row's accessible label
and hover text also carry the source page. Add downloads into the local cache when
needed and applies `music.add_to_timeline` as one undo step; unsave and clear-all only
reach the cache. Provider, cache, publication, no-audio-device and cancelled failures
are typed states with creator copy; a failed download changes no project bytes.

A header disclosure lists the project's complete used-track credits from
`Project::music_credits` and copies that exact block; unused saved tracks are excluded.
The settings sheet applies or removes the optional source key for this process only,
using a masked field and a direct source-terms link; the key never enters project,
preferences, automation, undo or logs.
