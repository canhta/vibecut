# UI design

## Principles

- **Selective, never cloned.** For each screen and interaction, compare how the references solve it, take the best fit for creators editing short and long talking videos, and cite each layout, interaction, shortcut and empty/loading/error state with the reason for choosing it. Compare code in `.spikes/repos/` and screenshots in `.spikes/ui-reference/`. A design without a reference basis is marked new and needs coordinator approval before implementation.
- **Familiar first.** Creators arrive from mainstream editors: standard panel placement, shortcuts and gestures beat novelty.
- **Every state designed.** Each panel has its empty, loading, error and offline states; nothing appears blank or frozen.
- **Agent-drivable.** Every interactive widget has a stable automation id that survives locale and layout changes. Agents and tests send input through those ids into the real command path, read engine project state plus app-owned widget state, and capture a window or panel screenshot that waits for a presented frame and returns a typed unavailable or timeout failure.
- **Decide visually, from source values.** Choose an icon set, colour or layout from a side-by-side render at matched size against the reference, never from a library's reputation; take colour values from the reference's source tokens (CSS or design tokens), not from screenshot pixels.
- **Two languages from day one.** All strings come from the English and Vietnamese catalogs; layouts survive the longer string.

## Design language

### Screens

| Screen | Reference A (desktop) | Reference B (web) | Ours |
|---|---|---|---|
| Home | Rail: Home, Templates; "Create with AI" (generative studios); Library, Spaces. Content: `Create project` tile, agent prompt card ("Create and edit in one flow") beside AI tool cards, AI suite cards (incl. `Long video to shorts`), then `Projects` with search, view, trash | Projects page: breadcrumb, view toggle, search, `New project`, grid; no rail | Rail: Home, Library. Content: `Create project` tile, `Long video to shorts` card, agent prompt card once the agent is in scope, then `Projects` (search, sort, grid/list, multi-select). Generative studio, image/video generation and ad-maker cards are out of scope |
| Editor | Tool tabs, preview, properties, timeline; agent button bottom-right opens the agent chat docked bottom-right beside the timeline | Same four panels; header holds logo menu (back to projects), name, Export | Layout below; header back-to-Home and project name; agent chat docked bottom-right |
| Long video to shorts | Own window from its Home card: upload (10 GB / 180 min, minutes quota shown), then results | none | Own window from its Home card: pick a local video (no upload, no quota), review suggestions, export them or open one in the editor |

Evidence: `.spikes/ui-reference/screens-2026-11-04/`, `01-project-list/`, `02-editor-shell/`.

### Measured desktop reference

[measured] Home values below use logical pixels (native screenshot pixels ÷ 2). Source: `.spikes/ui-reference/live/02-home-settings/shots/02-home-overview-01.png` (3456×2060 native pixels, 1728×1030 logical). Samples and bounds are recorded in `.spikes/ui-reference/live/tokens.md`; unresolved measurements stay in Gaps. Desktop measurements take precedence over earlier proposed values. Web-only observations do not replace desktop sizes or colours.

- Rail: 240 px wide; selected item 176×32 px at x=20; adjacent row pitch 44 px.
- Create-project banner: bounds (240,44)–(1668,187.5), size 1428×143.5 px. Its gradient samples are `#83D4E1` at (250,60), `#80D3DF` at (250,110), `#7DD1DF` at (250,170), and `#53AECC` at (1650,110).
- Home project thumbnails: 124 px wide, with 22 px gaps in the nine-card capture. Card count varies between captures; nine columns are not a fixed layout rule.
- Home background `#090909` at (300,690); rail gradient `#06171A` at (10,100) to `#080F10` at (10,400); selected row `#162326` at (100,180).
- Heading `#FAFAFA`, card title `#CCD3DA`, metadata `#7A7A7A` are sampled from their text bounds in that shot. Text bounds establish rendered glyph size, not a font size; radii and avatar/icon bounds remain unmeasured.

### Layout grid

- Baseline window 1728×1030 logical px [measured desktop capture above]; minimum 1280×800 remains [proposed], not verified by resizing this capture.
- Top bar 52 px; panel header 44 px; properties tab row 40 px; timeline toolbar 40 px.
- Default columns 25 / 50 / 25 percent: tools, preview, properties.
- Rows 50 / 50 percent: main content, timeline, matching the PNL-01 reference layout; the measured CapCut capture divides the same rows 60 / 40.
- Separators are 1 px `#263031` [measured desktop rail at (20,264) in the Home shot above]; application to editor panel separators remains [proposed]. There is no gutter between panels.
- Radii: 0 px on editor panels, 4 px on fields, 12 px on modals.
- Panel dividers are draggable; assets and properties stop at 15–40%, preview at 30–85%, timeline at 15–70%, so a drag never collapses a region. Reset restores the default columns above.
- Export is a 120x26 px button at the top-right of the top bar.
- Home columns fill available width using the [measured rail and thumbnails](#measured-desktop-reference); Library asset-card geometry remains unmeasured.

### Theme and tokens

Dark and light both ship in v1 with the same geometry, type and density; only colour changes. Dark follows the brand reference's tokens (named in the Source column); light is our own ramp built from the same reference greys and colour ramps. Ratios are WCAG 2.x against the named surface.

| Token | Dark | Light | Contrast | Source (dark) |
|---|---|---|---|---|
| Window, top bar, editor chrome | `#0F1113` | `#F4F4F4` | | surface-tertiary |
| Panels | `#18191C` | `#FFFFFF` | | surface-section |
| Modals, popovers | `#1C1E20` | `#FFFFFF` | | surface-primary |
| Fields, controls | `#23262A` | `#EDEDED` | | surface-secondary |
| Hover, active tab | `#2A2D32` | `#E4E4E4` | | surface-secondary-hover |
| Text | `#FFFFFF` | `#131517` | 17.6:1 dark panel; 18.3:1 light | font-primary |
| Secondary text | `#898A8B` | `#565656` | 5.08:1 dark panel (4.39:1 on fields: placeholders only); 6.27:1 light field | font-secondary |
| Disabled | `#737475` | `#7F7F7F` | not required | font-disabled |
| Separators (decorative) | `#26272A` | `#E4E4E4` | not required | separator-card, white 6% |
| Component borders | `#393C3F` | `#828282` | light 3.28:1 on fields; dark fields are identified by fill plus border [proposed] | white 10% |
| Accent: selection, checked, primary button, focus | `#D1FE17` | `#4F6109` | 15.0:1 dark panel; 6.89:1 light; labels on fill `#131517` (15.6:1) dark, white (6.89:1) light | lime-500 / lime-900 |
| Highlight: AI markers, badges | `#FF005B` | `#AD003E` | 4.52:1 dark panel; 7.38:1 light | pink-500 / pink-700 |
| Error | `#FF5462` text, `#E72930` fill | `#D60016` | 5.61:1 dark panel; 5.43:1 light | red-500, surface-error / red-700 |
| Success | `#53C546` | `#2E7D32` | 7.9:1; 5.13:1 | font-success |
| Warning | `#F3C977` | `#8A6100` | 11.2:1; 5.54:1 | font-warning |
| Video clip fill and border | `#0A2A24` | `#E3F7F2` fill, `#0B6B5B` border | 15.3:1 text on light fill | [proposed] track colour, not brand |

- **Accent.** Follow the table for fill/label pairs; white on dark-theme lime is 1.17:1, and dark-theme lime on white is 1.12:1.
- **Highlight.** Pink is used sparingly: at most one pink mark per view, never selection, a primary button or an error. As a fill it carries an icon, a dot or `#131517` text.
- **Surfaces.** Adjacent surface steps are subtle by design; 3:1 applies to text and to the boundary of a component, not to decorative separators.

### Type scale

- Four sizes: 14, 16, 18, 28 px; line heights 1.4, 1.35, 1.3, 1.2.
- 14 px metadata, labels, tabs and values; 16 px panel and section headings; 18 px modal titles; 28 px page headings.
- Weights: 400 body, 500 labels, 600 headings, 700 modal titles.
- Timecode and numeric fields use tabular figures.
- Inter 400/500/600/700, bundled TTF, no runtime font request.

### Iconography

- Phosphor `regular` only, embedded SVG; no other weight or set. The selected state is the accent colour on icon and label. Every button carries an icon; text-only buttons are limited to modal footers and the properties tabs.
- Sizes are boxes (glyph is about 80% of the box): 16 px in dense rows and timeline headers, 20 px in toolbars, 24 px for empty states. At 20 px the regular stroke is 1.25 px, matching CapCut's ~1.2 px.
- Feature toolbar tab [measured desktop, 2x screenshot]: 16 px glyph, 3.5 px gap to a 12 px semibold label, 48 px minimum tab width, about 9 px between labels, 42 px bar on the panel surface; inactive in text colour; selection follows the icon rule above.
- Properties tabs are text-only at 14 px; the active tab uses the accent.
- As in CapCut, the timeline toolbar, player controls and track-header controls are icon-only; labels appear only in the feature toolbar tabs, the top bar's primary actions and dialogs.
- Icon-only buttons are 28x28 px with a 16 px icon and a tooltip naming the action and its shortcut.

### Motion

- Modals 200 ms; accordions 200 ms ease-out; tooltips 150 ms.
- Scrubbing, trimming, dragging and playhead movement run at 0 ms.
- Reduced-motion preference forces 0 ms for modal and accordion transitions.

### Density

- Row pitch 48 px; field height 26 px. Home navigation follows [measurements](#measured-desktop-reference); bars follow [layout](#layout-grid) and [settings](#export-and-settings).
- Top-bar primary buttons 26 px high; modal footer buttons 34 px high; icon buttons follow [iconography](#iconography).
- Timeline: 22 px ruler, 65 px video track, 50 px audio track, 25 px text/sticker/effect/filter/adjustment track, 6 px gap, 112 px label column, 12 px scrollbar.
- Clip trim handles are 8 px wide with a 12 px capture area.

### Local models

The model manager reuses the export modal's 12 px radius, 24 px inner margin and
34 px footer buttons at an 880 px width. A scrollable model list shows each id and
its installed/download state; the selected model's detail shows provider, language
scope, exact size, licence and source links, per-file SHA-256 and the policy
restriction. Only the affirmative footer Download click grants transfer consent;
non-permissive licences add an explicit review checkbox first. Progress, Cancel,
Resume and typed storage/network/integrity errors stay in the same footer. The
layout is our own application of the [layout grid](#layout-grid) and
[density](#density); no external reference capture exists for it, so treat its
spacing as proposed until the next UI audit.

### Export and settings

- Export is a 900x820 px modal: 227 px cover column plus settings, using [layout radii](#layout-grid) and [density](#density).
- Footer: `Duration: Ns | Size: about N MB` at 14 px, then a 90x34 px `Export` button and a 90x34 px `Cancel` button.
- Video section: Resolution (480P, 720P, 1080P, 2K, 4K, 8K), Bit rate, Codec, Format (MP4), Frame rate, Colour space; Audio is a collapsed section.
- Settings is a 480 px modal: 34 px tab bar with Draft, Edit, Performance, General; [standard rows/fields](#density), 16 px checkboxes.
- The assets panel leads with a 36 px vertical icon bar: Media, Sounds, Text, Stickers, Effects, Transitions, Captions, Adjustment, Settings at 28 px icon buttons with right-side tooltips and top/bottom scroll fades. A selected element's properties view shows its type tabs as 14 px text at 40 px row height (active tab accent, 2 px underline); the empty and multi-select views keep the existing shared-field presentation, and `Project`/`Details` remain the inspector header switch. Geometry follows [layout](#layout-grid), styling follows [iconography](#iconography).
- Scrub number fields have a 28×26 px drag handle, a 26 px typed entry and optional unit suffix; the active value previews during the drag and commits once on release.

### Shortcuts

- Cmd/Ctrl+B splits at the playhead.
- I and O set in and out; Shift+X exports the in/out range.
- Space toggles playback; K pauses; J and L seek backwards/forwards one second; arrows step one frame; Shift plus arrows jump one second; Home and End go to the ends.
- Delete and Backspace remove; Cmd/Ctrl+Z undoes; Shift+Cmd/Ctrl+Z redoes; Cmd/Ctrl+C, V, D copy, paste, duplicate; N toggles snapping.
- Cmd/Ctrl+, opens settings; ? opens the shortcut list.
- Rebinding captures the next chord, rejects a key already bound to another action, and offers reset to defaults.

### States

- Import progress: 2 px accent bar, 14 px `Processing N%`, and a 320 px bottom-right toast.

### Gaps

A UI ticket that hits a gap captures a real desktop screenshot first.

- Desktop error/offline/import-failure states remain uncaptured. A web-only empty-project Export tooltip reads `Project is empty and cannot be exported` (`.spikes/ui-reference/live/09-export-errors/shots/web/09-web-export-empty-blocked.png`). It proves no desktop error UI, actual export, progress/done or file-failure handling.
- Audio and text clip colours: only a video clip has been sampled.
- Multi-track timeline geometry: only one video track has been sampled.
- Motion: no durations or easing have been measured.
- Desktop shortcut-dialog layout/search/rebinding/conflict remains unmeasured. Web-only stills show a read-only three-column, 45-row list (17 Globe, 11 Timeline, 17 Canvas; directions counted separately): `.spikes/ui-reference/live/02-home-settings/shots/web/02-web-editor-shortcuts-1.png` and `02-web-editor-shortcuts-2.png`. This does not change the desktop shortcuts above.
- Panel min/max sizes: only one size per screenshot has been observed.
- The live capture has no desktop editor/timeline/player/properties/export frames for areas 03–09. Their web-only tokens cannot confirm or replace existing desktop geometry. Home card radii, type sizes and detailed icon bounds also remain gaps.

## Evidence

Reference screenshots and behaviour notes live in `.spikes/ui-reference/` (local). UI behaviour tests and design-review captures follow [verification](../engineering/verification.md#test-selection).
