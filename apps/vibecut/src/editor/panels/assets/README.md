# Assets

Interface: `State`, `show`, `Action`, `prepare`, `media_panel`, `import`, `sounds`
(with its `music` section), `captions`.

The area owns its navigation drafts, the media selection and thumbnail caches, the
import batch and its last report, and the prepared workflow projection; `show` consumes
the editor's read-only context and returns `Action` values; `State::apply` handles them
through the `Port` the frame lends, so the area never names `Editor` and owns Music state
next to Sounds. Only cross-area intentions leave as `Request`s. `prepare`
builds the read-only projection the view draws. Toast and clock deadlines stay in the
editor core and are passed to the overlay renderers.

## Effects panel

The Effects tab lists the library-shelf entries of `project::effect_definitions()` as draggable tiles. Each tile
previews through the engine `EffectThumbnails` job and cache: the CPU reference
compositor headlessly, the playback GPU adapter and its byte budget in native
startup. Pending, failed and ready states stay visible; a failed tile offers Retry. A
tile drops onto a video, image, sticker or graphic clip, or `+` adds it to the
selected clip. The tab's preview jobs and drag state live in [`effects`](effects/README.md).
The sub-navigation lists Favorites, then one collapsible `Video effects` group holding All and the library shelves; the group header is 104 x 22 (measured, [editor UI](../../../../../../docs/product/ui/editor.md#effects)). The reference's `Body effects` group has no catalogue content here, so it is not built. The grid header names the selected sidebar entry (All, a shelf or Favorites). Threshold's tile previews a 120-200 band (proposed; the reference has no measured tile).

## Filters panel

The Filters tab sits between Captions and Adjustment (the reference strip order; the icon is the existing half-circle, proposed). Its sub-navigation lists the shelves Featured, Movies, Retro, Night, Mono, Tools and Life from `project::filter_definitions()`; Portrait and Landscape are not built (how the reference tunes them is unverified) and the Favorites entry and tile star wait for 222 (hook: `view/filters.rs`). Tiles use the Effects grid metrics and show the look graded by our own pipeline on a procedural dusk landscape (`EffectPreviews::filters`, one render at time zero on the CPU compositor; no photograph). A tile or its click inserts one filter clip through `filters.insert` at the playhead: an effect item on its own Effect lane, 3 s long, Intensity 100, selected afterwards, one undo step. The clip's Intensity is its opacity (0..=100 percent), edited in Properties with the other effect-item controls.

## Adjustment presets

The Adjustment tab opens with `Add adjustment layer` (`adjustment.insert`): a label and a plus button in one row. The plus inserts an effect item (`ItemParams::Effect`, no media) at the playhead through `InsertItem` with auto placement, which finds or creates an Effect track. The layer is five seconds long, as text and graphics are, and is selected afterwards in one undo step. It is refused while insertion is unavailable and changes nothing then. The layer has no picture; its stack takes effects from the Effects tab (`+` or a tile drop), see the [properties owner](../properties/README.md#effects-tab). The label, row geometry and five-second length are proposed: the desktop Adjustment capture has no measured entry for this (ui-reference 07 G7), so only the tab's place in the feature strip is reference-backed.

The tab then lists the app-owned `adjustment_presets` bank from preferences as named rows with an apply and a delete icon. Apply targets the selected video or image item and writes the saved `color_grade` parameter map through `effects.update_param`, or adds one instance when the clip has none, inside one `batch` command (one undo step); a locked track, non-visual selection or missing selection changes nothing. Delete rewrites the bounded preference bank and never touches project data. Empty is an icon and `No presets`.

## Transition catalog

The Transitions tab reads `project::TRANSITION_DEFINITIONS`; names and shelf navigation use the app catalog, and search filters the current shelf. Cards follow the measured 88 px grid, 15 px gaps, 116 px row pitch. Visible hover or keyboard focus plays a 48 px preview at 12 frames/s through render's procedural thumbnail evaluator; idle tiles retain their 176 px midpoint raster. Only an active, visible tile requests timed repaints; window focus loss restores its idle frame. Failed previews offer Retry, theme changes replace textures, and the preview's accessible value reports the progress of its rendered frame. The all-shelves view keeps the measured heading space without repeating the tab name; a selected shelf names its results. The private `transition` module owns previews, `view/transition` the catalog, `workflow/transition` target resolution.

A card or its plus applies to two adjacent selected video clips, or to the cut under the playhead. A card can also be dropped on the visible shared edge (12 px capture each side), which highlights before release. Every application is `transition.add` with a centered 0.5 s duration clamped by project's source-handle rules. Missing adjacency, exhausted handles, locked tracks, duplicate cuts and overlapping transitions give localized refusals. The plus is disabled without a valid selection. Escape, leaving the tab, focus loss, a changed snapshot or a missed or reversed drop publishes nothing. One release is one history step.

Media rows, import feedback and marquee selection: [assets workflow](workflow/README.md).
Sounds rows: [sounds](sounds/README.md).

## Media sub-navigation

Import heads the Media group; Media and Timeline are its two pages, with one active row. Timeline lists the project's scenes in project order, the main scene marked and the open scene selected in the sub-navigation's style (accent label on the shared row fill, not egui's selection fill). A click switches scene through `Request::SwitchScene`, the scene row's switch without its select mode. The page shows while the sub-navigation is visible, as Music does. Create and rename stay in the scene sheet until the reference shows them here. Scene commands and the sheet: [timeline scene switcher](../../timeline/README.md#scene-switcher).

## Stickers

The Stickers tab searches with the search field and the All, Emoji, Icons, Flags and Shapes tabs, with a result count and See all. All lists the four shape cards (Rectangle, Ellipse, Polygon, Star) and then the catalogue: bundled flags, plus the downloaded emoji and icon collections. Emoji and Icons search only their category. Flags is the flag catalog in [libraries](../../../../../../crates/libraries/src/sticker_library/README.md); Shapes is the four graphic cards. A collection of the Emoji or Icons tab that is not downloaded (or not verified) shows a download card with its name, licence credit, source and size; its Download action is the explicit download job, and the card shows progress with Cancel, or the offline or retry state. Cancel leaves nothing in the cache. The job state is kept by `StickerDownloads` in the tab's state; `rows` reads the store, so a finished download shows its tiles on the next frame. Each shape card inserts through its `graphic.insert.*` command, and each flag through `libraries::stickers::insert_sticker`, the edit `stickers.insert` applies: one undo step at the playhead, five seconds long, the item selected afterwards. The preview shows the first 24 results; See all widens the page to the catalog's default limit of 100. Flag thumbnails come from `render::rasterize_sticker`; shape thumbnails come from the production compositor. Each thumbnail is one texture per id and pixel scale, held by the tab for the session.

Recent lists up to 50 ids of both kinds, newest first and deduplicated, from `Preferences::sticker_recents`. Stale ids drop out when the list is read. It stays outside projects, shows only while the search is empty, follows the tab, and Clear empties the saved list. The shared insertion predicate gates every insert, so drafts, transactions and modal work refuse it. Shape cards use their command ids (`graphic.insert.star`); other tiles use `stickers.insert.<id>` and Recent tiles `stickers.recent.insert.<id>`.

The tile grid uses the measured Stickers grid: 85 px square tiles, 17 px column gaps and 13 px row gaps ([editor UI](../../../../../../docs/product/ui/editor.md#stickers-grid)). The name and `+` action under each tile are proposed. The preview count of 24 and the five-second default length are open.

Music section: [sounds](sounds/README.md#music-section).
