# UI design

## Principles

- **Selective, never cloned.** For each screen and interaction, compare how the
  references solve it, take the best fit for creators editing short and long talking
  videos, and record why. Improve on a reference when ours serves users better.
- **Familiar first.** Creators arrive from mainstream editors: standard panel
  placement, shortcuts and gestures beat novelty.
- **Every state designed.** Each panel has its empty, loading, error and offline
  states; nothing appears blank or frozen.
- **Agent-drivable.** Every interactive widget has a stable automation id that
  survives locale and layout changes. Agents and tests send input through those ids
  into the real command path, read engine project state plus app-owned widget state,
  and capture a window or panel screenshot that waits for a presented frame and
  returns a typed unavailable or timeout failure.
- **Two languages from day one.** All strings come from the English and Vietnamese
  catalogs; layouts survive the longer string.

## Design language

### Screens

| Screen | Reference A (desktop) | Reference B (web) | Ours |
|---|---|---|---|
| Home | Rail: Home, Templates; "Create with AI" (generative studios); Library, Spaces. Content: `Create project` tile, agent prompt card ("Create and edit in one flow") beside AI tool cards, AI suite cards (incl. `Long video to shorts`), then `Projects` with search, view, trash | Projects page: breadcrumb, view toggle, search, `New project`, grid; no rail | Rail: Home, Library. Content: `Create project` tile, `Long video to shorts` card, agent prompt card once the agent is in scope, then `Projects` (search, sort, grid/list, multi-select). Generative studio, image/video generation and ad-maker cards are out of scope |
| Editor | Tool tabs, preview, properties, timeline; agent button bottom-right opens the agent chat docked bottom-right beside the timeline | Same four panels; header holds logo menu (back to projects), name, Export | Layout below; header back-to-Home and project name; agent chat docked bottom-right |
| Long video to shorts | Own window from its Home card: upload (10 GB / 180 min, minutes quota shown), then results | none | Own window from its Home card: pick a local video (no upload, no quota), review suggestions, export them or open one in the editor |

Evidence: `.spikes/ui-reference/screens-2026-11-04/`, `01-project-list/`,
`02-editor-shell/`.

### Measured desktop reference

[measured] Home values below use logical pixels (native screenshot pixels ÷ 2).
Source: `.spikes/ui-reference/live/02-home-settings/shots/02-home-overview-01.png`
(3456×2060 native pixels, 1728×1030 logical). Samples and bounds are recorded in
`.spikes/ui-reference/live/tokens.md`; unresolved measurements stay in Gaps.
Desktop measurements take precedence over earlier proposed values. Web-only
observations do not replace desktop sizes or colours.

- Rail: 240 px wide; selected item 176×32 px at x=20; adjacent row pitch 44 px.
- Create-project banner: bounds (240,44)–(1668,187.5), size 1428×143.5 px.
  Its gradient samples are `#83D4E1` at (250,60), `#80D3DF` at (250,110),
  `#7DD1DF` at (250,170), and `#53AECC` at (1650,110).
- Home project thumbnails: 124 px wide, with 22 px gaps in the nine-card capture.
  Card count varies between captures; nine columns are not a fixed layout rule.
- Home background `#090909` at (300,690); rail gradient `#06171A` at (10,100)
  to `#080F10` at (10,400); selected row `#162326` at (100,180).
- Heading `#FAFAFA`, card title `#CCD3DA`, metadata `#7A7A7A` are sampled from
  their text bounds in that shot. Text bounds establish rendered glyph size,
  not a font size; radii and avatar/icon bounds remain unmeasured.

### Layout grid

- Baseline window 1728×1030 logical px [measured desktop capture above]; minimum
  1280×800 remains [proposed], not verified by resizing this capture.
- Top bar 52 px; panel header 44 px; properties tab row 40 px; timeline toolbar
  40 px.
- Columns 25 / 50 / 25 percent: tools, preview, properties.
- Rows 60 / 40 percent: main content, timeline.
- Separators are 1 px `#263031` [measured desktop rail at (20,264) in the
  Home shot above]; application to editor panel separators remains [proposed].
  There is no gutter between panels.
- Radii: 0 px on editor panels, 4 px on fields, 12 px on modals.
- Panel dividers are draggable; the 25/50/25 split is the reset value.
- Export is a 120x26 px button at the top-right of the top bar.
- Home/Library uses the measured 240 px left rail. Home project thumbnails are
  124 px wide with a 22 px gap in the measured grid; columns fill available width.
  This does not establish asset-card geometry inside Library.

### Theme and tokens

- Dark and light both ship in v1. Dark window/Home background is `#090909`
  [measured desktop, Home shot above]. Editor chrome stays `#0F0F0F` and top bar
  `#131313`; the current live capture does not remeasure those editor surfaces.
- Light theme tokens are in [Light theme](#light-theme); same geometry, type and
  density as dark.
- Editor panels and editor chrome use `#0F0F0F`; Home has the measured
  `#090909` background above.
- Export modal `#262626`, fields `#303034`; settings modal `#141414`, controls
  `#262626`, active tab `#3B3B3B`.
- Text `#E0E0E0`; secondary text `#949494`; separators `#263031` [measured
  desktop, Home shot above: brightest pixels in Home R(54,175,88,184), Management
  R(20,438,88,445), divider sample (20,264)]. Borders remain `#383838`.
- Accent lime `#D1FE17` (brand choice, replacing CapCut's teal) for selection,
  checked boxes and the primary button; text and checkmarks on a lime fill are
  `#0F1113` (16.1:1), never white (1.17:1).
- Video clips use a [proposed] `#0A2A24` teal border around the filmstrip.

### Light theme

Same geometry, type and density as dark; only colour changes. Values sampled from
a real light reference screenshot are cited; every designed value is marked
`[proposed]`, including the whole neutral ramp (the references are the basis, not a
palette to copy). Ratios are WCAG 2.x contrast against the named surface.
Local evidence IDs E1–E6 map to the light screenshots and token notes under
`.spikes/ui-reference/`; the mapping stays in the ticket report so reference names
never enter tracked files.

| Token | Dark | Light | Ratios against surface (and label) | Evidence |
|---|---|---|---|---|
| Window (editor chrome stays `#0F0F0F`) | `#090909` | `#F2F3F5` | 1.11:1 vs top bar `#FFFFFF` | [proposed]; E6 light-pane samples `#EDF3F3`/`#FDFDFD` |
| Top bar | `#131313` | `#FFFFFF` | 1.11:1 vs window `#F2F3F5` | Observed in E1 and E2 |
| Editor panels | `#0F0F0F` | `#F9FAFB` | 1.06:1 vs window `#F2F3F5` | Observed panel fill in E2 |
| Export modal | `#262626` | `#FFFFFF` | 1.11:1 vs window `#F2F3F5` | Observed white popover surface in E3 |
| Fields | `#303034` | `#F2F3F5` | 1.11:1 fill vs modal `#FFFFFF`; 3.10:1 boundary via border `#858B91` | [proposed]; light row/field geometry observed in E3 and E4 |
| Settings modal | `#141414` | `#FFFFFF` | 1.11:1 vs window `#F2F3F5` | Observed white settings panel in E4 |
| Controls | `#262626` | `#EEEFF0` | 1.10:1 fill vs panel `#F9FAFB`; 3.10:1 boundary via border `#858B91` | Observed control fills in E2 |
| Active tab | `#3B3B3B` | `#DEDEDE` | 1.29:1 fill vs panel `#F9FAFB`; 4.71:1 accent text on fill | Observed in E2 |
| Text | `#E0E0E0` | `#1C1C1C` | 16.31:1 vs panel `#F9FAFB` | Observed text colour in E1 and E2 |
| Secondary text | `#949494` | `#5C6268` | 5.91:1 vs panel `#F9FAFB`; 4.59:1 on active tab `#DEDEDE` | [proposed]; reference muted 50% grey fails 4.5:1 on light |
| Separators | `#263031` | `#DEDEDE` | 1.29:1 vs panel `#F9FAFB` (decorative; WCAG 1.4.11 does not apply) | Observed 1 px lines in E2 |
| Borders | `#383838` | `#858B91` | 3.10:1 vs field `#F2F3F5`; 3.29:1 vs panel `#F9FAFB` | [proposed]; reference `#DEDEDE` borders are 1.3:1 |
| Accent: selection, checked boxes, primary button | `#D1FE17` (16.3:1 on `#0F0F0F`, 11.3:1 on `#303030`) | `#4D6600` | 6.25:1 vs panel `#F9FAFB`; 4.85:1 on active tab `#DEDEDE`; 6.53:1 for white label on fill | [proposed]; `#D1FE17` is 1.12:1 on the light panel |
| Video clip fill and border | `#0A2A24` | `#E3F7F2` fill, `#0B6B5B` border | 1.07:1 fill vs panel; 5.77:1 border on fill; 15.30:1 primary text on fill | [proposed]; no light timeline clip sample |
| Error | `#EA1515` [proposed dark] | `#B3261E` | 5.91:1 on error surface `#FFF0F0`; 6.25:1 vs panel `#F9FAFB`; 6.54:1 for white label on fill | [proposed]; E5 error toast uses `#E50000` on `#FFF0F0`, but that is only 4.38:1 |

- **Accent decision.** Lime `#D1FE17` is the brand accent. It fails on light
  surfaces, so light uses `#4D6600` (same lime hue, darkened) for selection,
  checked boxes, primary-button fill and active tab text, with white labels on
  that fill (6.53:1). Dark keeps `#D1FE17` with `#0F1113` labels on its fill.
- **Surfaces.** Adjacent surface separations are intentionally subtle (1.06–1.29:1);
  the 3:1 requirement applies to text and to control boundaries that identify a
  component, not to decorative 1 px panel separators. Light panels remain lighter
  than the light window; the measured dark Home
  background does not establish desktop editor-panel contrast.
- **Lines.** Panel separators keep the observed decorative `#DEDEDE` (1.29:1);
  WCAG 1.4.11 does not require 3:1 for them. Field and control borders that identify
  a component use `#858B91` (3.10:1 on fields, 3.29:1 on panels).
- **Error and clips.** Import/progress keeps the accent bar `#4D6600`; error toasts
  keep the observed pale surface `#FFF0F0` but use `#B3261E` text/icons because the
  reference `#E50000` is 4.38:1 there. Light video clips use a `#E3F7F2` fill,
  `#0B6B5B` 1 px border and primary text; the filmstrip media sits inside them.

### Type scale

- Four sizes: 14, 16, 18, 28 px; line heights 1.4, 1.35, 1.3, 1.2.
- 14 px metadata, labels, tabs and values; 16 px panel and section headings;
  18 px modal titles; 28 px page headings.
- Weights: 400 body, 500 labels, 600 headings, 700 modal titles.
- Timecode and numeric fields use tabular figures.
- Inter 400/500/600/700, bundled TTF, no runtime font request.

### Iconography

- Phosphor `regular` only, embedded SVG; no other weight or set. The selected
  state is the accent colour on icon and label. Every button carries an icon; text-only buttons are
  limited to modal footers and the properties tabs.
- Sizes are boxes (glyph is about 80% of the box): 16 px in dense rows and
  timeline headers, 20 px in toolbars, 24 px for empty states. At 20 px the
  regular stroke is 1.25 px, matching CapCut's ~1.2 px.
- Feature toolbar tab [measured desktop, 2x screenshot]: 16 px glyph, 3.5 px
  gap to a 12 px semibold label, 48 px minimum tab width, about 9 px between
  labels, 42 px bar on `#303030`; inactive `#E0E0E0`, selected tab in the accent.
- Properties tabs are text-only at 14 px; the active tab uses the accent.
- Icon-only buttons are 28x28 px with a 16 px icon and a tooltip naming the
  action and its shortcut.

### Motion

- Modals 200 ms; accordions 200 ms ease-out; tooltips 150 ms.
- Scrubbing, trimming, dragging and playhead movement run at 0 ms.
- Reduced-motion preference forces 0 ms for modal and accordion transitions.

### Density

- Row pitch 48 px; field height 26 px; settings tab bar 34 px; properties tab
  row 40 px; Home/Library nav pitch 44 px with 32 px selected-item height
  [measured desktop, Home shot above]; top bar 52 px.
- Top-bar primary buttons 26 px high; modal footer buttons 34 px high; icon
  buttons 28x28 px.
- Timeline: 22 px ruler, 65 px video track, 50 px audio track, 25 px text track,
  6 px gap, 112 px label column, 12 px scrollbar.
- Clip trim handles are 8 px wide with a 12 px capture area.

### Export and settings

- Export is a 900x820 px modal at 12 px radius: 227 px cover column plus a
  settings column, 48 px row pitch, 26 px fields.
- Footer: `Duration: Ns | Size: about N MB` at 14 px, then a 90x34 px `Export`
  button and a 90x34 px `Cancel` button.
- Video section: Resolution (480P, 720P, 1080P, 2K, 4K, 8K), Bit rate, Codec,
  Format (MP4), Frame rate, Colour space; Audio is a collapsed section.
- Settings is a 480 px modal: 34 px tab bar with Draft, Edit, Performance,
  General; 48 px rows; 26 px controls; 16 px checkboxes.
- The properties panel uses a 40 px text-tab row (`Project`, `Details`), not a
  vertical icon rail.

### Shortcuts

- Cmd/Ctrl+B splits at the playhead.
- I and O set in and out; Shift+X exports the in/out range.
- Space toggles playback; K pauses; J and L seek backwards/forwards one second; arrows step one frame; Shift plus arrows
  jump one second; Home and End go to the ends.
- Delete and Backspace remove; Cmd/Ctrl+Z undoes; Shift+Cmd/Ctrl+Z redoes;
  Cmd/Ctrl+C, V, D copy, paste, duplicate; N toggles snapping.
- Cmd/Ctrl+, opens settings; ? opens the shortcut list.
- Rebinding captures the next chord, rejects a key already bound to another
  action, and offers reset to defaults.

### States

- Import progress: 2 px accent bar, 14 px `Processing N%`, and a 320 px
  bottom-right toast.

### Gaps

A UI ticket that hits a gap captures a real desktop screenshot first.

- Desktop error/offline/import-failure states remain uncaptured. A web-only
  empty-project Export tooltip reads `Project is empty and cannot be exported`
  (`.spikes/ui-reference/live/09-export-errors/shots/web/09-web-export-empty-blocked.png`).
  It proves no desktop error UI, actual export, progress/done or file-failure handling.
- Audio and text clip colours: only a video clip has been sampled.
- Multi-track timeline geometry: only one video track has been sampled.
- Motion: no durations or easing have been measured.
- Desktop shortcut-dialog layout/search/rebinding/conflict remains unmeasured.
  Web-only stills show a read-only three-column, 45-row list (17 Globe, 11 Timeline,
  17 Canvas; directions counted separately):
  `.spikes/ui-reference/live/02-home-settings/shots/web/02-web-editor-shortcuts-1.png`
  and `02-web-editor-shortcuts-2.png`. This does not change the desktop shortcuts above.
- Panel min/max sizes: only one size per screenshot has been observed.
- The live capture has no desktop editor/timeline/player/properties/export frames
  for areas 03–09. Their web-only tokens cannot confirm or replace existing desktop
  geometry. Home card radii, type sizes and detailed icon bounds also remain gaps.

## Evidence

Reference screenshots and behaviour notes live in `.spikes/ui-reference/` (local).
A UI ticket cites the reference it took each decision from and verifies with an
offscreen screenshot of our app driven through automation ids
([verification](../engineering/verification.md#test-selection)).
