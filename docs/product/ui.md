# UI design

## Principles

- **Layout cloned, palette and icons ours.** The editor matches the reference captures in layout: panel structure and proportions, control sizes, spacing, radii, type scale, row and tab composition, what sits where. Colour comes from our own tokens and icons from our own set ([iconography](#iconography)); a reference colour or glyph is never copied, and a colour or icon difference is not a mismatch. Where a reference solves an interaction poorly for creators editing talking videos, say so and cite the reason; a design without a reference basis is marked new and needs coordinator approval. Compare code in `.spikes/repos/` and screenshots in `.spikes/ui-reference/`.
- **Three independent windows.** Home, Editor and Long video to shorts are separate windows, each with its own size, title/top bar and chrome, matched against its own captures; a component or token is shared only through the common kit, never a frame.
- **Creator words, few of them.** Write every string from the view of a creator editing a video, not of the engineer who built the feature. A control that an icon makes clear carries the icon and a tooltip, no visible label; visible text is limited to feature tabs, the top bar's primary actions, dialogs and values the creator reads or types. No section headings that repeat the panel or tab name, no instructional sentences in empty panels (an icon and at most four words), no internal names (provider, endpoint, payload, codec, session, snapshot, ASR, ticket or milestone ids), no "this panel arrives later" placeholders (hide what does not work yet). An error says what happened to the creator's work and the one thing to do, in plain words and at most one short sentence; the technical cause goes to the log or a details disclosure. Number formats follow the reference (whole percents, no trailing zeros, no ranges like "(0-1)"). A reviewer returns `changes` for a string that fails this and for an added string without a reason the icon could not carry it.
- **Familiar first.** Creators arrive from mainstream editors: standard panel placement, shortcuts and gestures beat novelty.
- **Every state designed.** Each panel has its empty, loading, error and offline states; nothing appears blank or frozen.
- **Agent-drivable.** Every interactive widget has a stable automation id that survives locale and layout changes. Agents and tests send input through those ids into the real command path, read engine project state plus app-owned widget state, and capture a window or panel screenshot that waits for a presented frame and returns a typed unavailable or timeout failure.
- **Decide visually, from source values.** Choose an icon set, colour or layout from a side-by-side render at matched size against the reference, never from a library's reputation; take geometry from measured reference shots and colour values from our own tokens, not from screenshot pixels.
- **Two languages from day one.** All strings come from the English and Vietnamese catalogs; layouts survive the longer string.

## Design language

### Screens

| Screen | Reference A (desktop) | Reference B (web) | Ours |
|---|---|---|---|
| Home | Rail: Home, Templates; "Create with AI" (generative studios); Library, Spaces. Content: `Create project` tile, agent prompt card ("Create and edit in one flow") beside AI tool cards, AI suite cards (incl. `Long video to shorts`), then `Projects` with search, view, trash | Projects page: breadcrumb, view toggle, search, `New project`, grid; no rail | Rail: Home, Library. Content: `Create project` tile, `Long video to shorts` card, agent prompt card once the agent is in scope, then `Projects` (search, sort, grid/list, multi-select). Generative studio, image/video generation and ad-maker cards are out of scope |
| Editor | Tool tabs, preview, properties, timeline; agent button bottom-right opens the agent chat docked bottom-right beside the timeline | Same four panels; header holds logo menu (back to projects), name, Export | Layout below; header back-to-Home and project name; agent chat docked bottom-right |
| Long video to shorts | Own window from its Home card: upload (10 GB / 180 min, minutes quota shown), then results | none | Own window from its Home card: pick a local video (no upload, no quota), review suggestions, export them or open one in the editor |

Evidence: `.spikes/ui-reference/screens-2026-11-04/`, `01-project-list/`, `02-editor-shell/`, and the 2026-11-05 desktop capture set (index in its `notes.md`).

### Measured desktop reference

[measured] The 2026-11-05 desktop capture set (36 full-window shots, 2× Retina) replaces all earlier editor geometry. Values are logical px (native ÷ 2); the capture canvas is 1840×1142 and the app window is 1728×1030 at (56,38). Citations name a shot by its capture time (HHMMSS); every sample point and region is recorded in `.spikes/ui-reference/live/tokens.md`. Desktop measurements take precedence over earlier proposed values; web-only observations do not replace desktop sizes or colours.

**Editor frame** (shots 221513 empty, 221643 populated):

- Window 1728×1030. Top bar 36 px; a single 42 px strip below it carries the feature tabs (left), the player title (centre) and the properties tabs (right); content row 490 px; 6 px gutter; timeline 447 px; 8 px bottom inset. Content:timeline = 52:48 (221643: column x=300, y 38–1067; row y=200 and y=700).
- Columns left 582 / player 628 / properties 486 px = 34 / 37 / 28 % of the 1710 px inner width. Panels are separated by a 7 px `#1C1C1C` gutter, not a 1 px divider; the 8–9 px outer inset is the same colour. Panel surfaces `#262626`, the 42 px strip `#303030` (221643: x 647–653 and 1282–1288; x 65–1774 at y 120).
- Top bar: autosave `#949494` at the left, project name `#FAFAFA` centred, both 14 px; right group ends 11 px from the window edge: two 18 px layout icons, a 50×22 `Pro` pill and a 62×22 `Share` button (both `#4F4F4F`), a 67×22 `Export` button (`#57BECB`, white label). Buttons are 22 px high and start 7 px below the bar top (221643: x 1511–1772, y 45–67; text y 50–61).
- Feature tabs: 16 px icon over a 12 px/600 label, 5 px apart, 7 px inset top and bottom, content-width with about 12 px horizontal padding (min 48 px), pitch 49–61 px. Active icon and label `#57BECB`; inactive `#E0E0E0`; no underline (221643: icon x 81–97, label x 75–102, icon y 82–95, label y 100–108; icon centres 89, 137.5, 186.5, 236, 285.5, 340, 394.5, 445, 501.5, 562, 616).
- Sub-navigation: 120 px column with a 1 px `#070709` divider; 104×24 px buttons at 8 px inset on a 32 px pitch, `#3B3B3B` fill, 14 px label, 12 px chevron; selected label `#57BECB`; plain rows share the 32 px pitch (221643: x 65–185, buttons x 73–177, y 131–155 and 163–187).
- Asset panel: 24 px tool row (Import 72×24 with a dashed outline, Record 62×24, search 148×24 on `#1C1C1C`), a 12 px `All` filter row, a 120×73 asset thumbnail with a 6 px gap to a 12 px name; audio rows 48 px on a 60 px pitch with a 32×32 thumbnail; effects grid 4 columns of 88 px cards, 15 px gaps, 116 px row pitch (221635: y 128–151, Import x 198–269, card x 198–318/y 188–261; 221703: cards y 188, 248, 308; 221727: cards x 198, 301, 403, 506).
- Player: 12 px/600 `#FFFFFF` title and a 16 px menu in the 42 px strip; 611×343 frame (16:9) inset 8 px with a 1 px `#FAFAFA` border; a 16 px white round fit button with a dark glyph, centred 16 px below it (221643: x 959.5–975, y 513.5–529); footer row y 576–592 with a 12 px tabular timecode (current `#57BECB`, duration `#E0E0E0`, slash `#404040`), a 16 px storyboard icon, a 12 px play triangle centred, and 12 px `Full`/`Ratio` outlined labels with 16 px fit/fullscreen icons at the right (221643: panel x 654–1281, frame x 662–1273/y 154–497, footer x 663–1273).
- Properties: 12 px/600 tabs (active `#57BECB`, inactive `#E0E0E0`) with a 28 px gap and 11 px left inset; a 461×24 four-cell sub-tab bar (about 115 px cells, active `#3B3B3B` with a 1 px `#333333` outline, inactive `#1C1C1C`) starting 13 px below the strip; 1 px `#373737` dividers; collapsed section rows 54 px on a 55 px pitch with a 16 px reset and a `<◇>` 10 px keyframe pair; label-left/control-right rows on a 38–39 px pitch, with a 55 px slider row (221643: tabs y 90–99 at x 1300–1676; sub-tabs x 1301.5–1762.5/y 129–153; dividers y 423/478/533/588; labels y 222/277/315/354).
- Property controls: 54×22 value field (`#1C1C1C`) plus a 16×22 stepper (`#3B3B3B`) 2 px to its right; 66.5×22 position fields each followed by a 15.5×22 stepper after a 7 px gap (group x 1421.5–1607, 185.5 px); 336×2 slider (white filled up to the 8×12 thumb, `#404040` rest); 36×16 toggle; 12×12 checkbox; 294×22 alignment bar with 7 cells (221643: value x 1655–1709/y 233–255, stepper 1711–1727; position fields x 1421.5–1488 and 1518.5–1585, steppers x 1495.5–1510 and 1592.5–1607, y 309–331; slider x 1301–1636/y 243–245, thumb x 1366–1374/y 238–250; toggle x 1727–1762/y 274–290).
- Timeline toolbar: 32 px; first icon 10 px from the panel edge; 24×24 icon buttons with 16 px glyphs on a 24–32 px pitch and 1 px `#3B3B3B` group dividers 16 px high (y 620–636); right group mic, three 24×24 toggles (`#1B1B1C`, `#57BECB` glyph) 12 px apart, fit, −, a 96×2 zoom slider with an 8×12 thumb, + (221643: y 612–643, buttons y 617–640, toggles x 1382/1418/1454, slider x 1639–1735).
- Ruler 22 px: major labels every 3 s at this zoom (214.5 px apart), minor ticks every 0.3 s (21.4 px), `#6C6C6C` 14 px tabular labels, each preceded by a 1 px tick (221643: y 645–666; labels x 257–288, 471–501, 686–715).
- Tracks: 123 px header column with 16 px control glyphs at x 73/99/122/146/171, 10 px vertical inset; video track 76 px; clip 74 px with a 1 px white selection border, `#1E4C51` fill, a 17 px teal top band, a 39 px filmstrip and a 16 px waveform band (`#33757D`). Playhead 1 px white with a 10×15 outlined handle in the ruler; horizontal scrollbar 6 px with a 24×6 `#404040` thumb on the panel colour (221643: header x 65–188, track y 823–898, clip y 824–897, filmstrip y 842–880, waveform y 881–896, playhead x 417, handle x 412–421/y 645–659).
- The single main track sits 156 px below the ruler with 160 px of empty track area below it; why is not visible, so stacking above or below the first track is [gap].
- Home (shot 221432, window origin (56,38); the 2026-11-04 desktop values in `.spikes/ui-reference/live/tokens.md` agree): rail 240 px; create banner 1428×143.5 at content (240,44), x 296–1724, y 82–225.5; project thumbnails 124×124 on a 146 px pitch (22 px gaps), x 301–425, y 760–884. Rail selected item 176×32 at x=20 on a 44 px row pitch, background `#090909`, selected row `#162326`, heading `#FAFAFA`, card title `#CCD3DA`, metadata `#7A7A7A`; banner gradient `#83D4E1` to `#53AECC`. Full Home and Shorts measurements belong to ticket 117.

### Layout grid

- Baseline window 1728×1030 logical px [measured 2026-11-05 captures]; minimum 1280×800 remains [proposed], not verified by resizing.
- Top bar 36 px; one 42 px strip for feature tabs, player title and properties tabs; timeline toolbar 32 px [measured]. The old 52 / 44 / 40 px bars are superseded.
- Columns 34 / 37 / 28 percent: tools, preview, properties; content and timeline rows 52 / 48 percent [measured].
- Panel gutters are 7 px `#1C1C1C`, row gutters 6 px, outer inset 8 px [measured]; there is no 1 px separator between panels. In-panel dividers are 1 px `#070709` (left sub-nav) or `#373737` (properties sections).
- Radii: 0 px on editor panels, 4 px on fields, tabs and buttons, 12 px on dialogs [proposed]; the captured windows' own corner is about 16 px (OS chrome, 221811 and 221643 corner rows), which is not an app token. The 4 px is [measured] on the sub-navigation buttons, sub-tab bar and Export button corners.
- Panel dividers are draggable; assets and properties stop at 15–40%, preview at 30–85%, timeline at 15–70%, so a drag never collapses a region. Reset restores the default columns above [proposed limits].
- Export is a 67×22 px button at the top-right of the top bar [measured]; the old 120×26 value is superseded.
- Home columns fill available width using the [measured rail and thumbnails](#measured-desktop-reference); Library asset-card geometry remains unmeasured.

### Theme and tokens

Dark and light both ship in v1 with the same geometry, type and density; only colour changes. The semantic table below covers Home, dialogs and non-editor surfaces; the editor frame uses the measured palette further down, which supersedes its Window/Panels/Fields/Accent rows until ticket 116 reconciles the semantic tokens. Ratios are WCAG 2.x against the named surface.

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

#### Measured editor palette (2026-11-05)

[measured] Sampled in the editor shots; the sample point for each value is in `.spikes/ui-reference/live/tokens.md`. These are the editor-frame colours ticket 116 adopts in place of the earlier chrome values above; the light column remains [proposed] because no light capture exists.

| Role | Value | Shot / region |
|---|---|---|
| Window chrome, top bar, panel gutters | `#1C1C1C` | 221643 (650,200) |
| 42 px strip (feature tabs, player title, properties tabs) | `#303030` | 221643 (300,80) |
| Panels, timeline canvas | `#262626` | 221643 (300,200) |
| Active tab, buttons, steppers | `#3B3B3B` | 221643 (100,140) |
| Fields, inactive sub-tab track | `#1C1C1C` | 221643 (1660,245) |
| In-panel divider | `#070709` | 221643 (185,300) |
| Text and icons | `#E0E0E0` | 221643 (1301,190) |
| Title, bright values | `#FAFAFA` | 221643 brightest in (900–945,48–63) |
| Secondary text, autosave | `#949494` | 221643 brightest in (138–250,49–63) |
| Ruler labels | `#6C6C6C` | 221643 dominant in (257–288,648–659) |
| Timecode slash, scrollbar thumb | `#404040` | 221643 (729–735,576–592) / (194–218,1051–1057) |
| Accent: active tab, current timecode, toggle, selection | `#57BECB` | 221643 (90,92) |
| Disabled label/icon | `#5E5E5E` / `#6C6C6C` | 221643 (1350,555) / (221,628) |
| Video clip fill / waveform | `#1E4C51` / `#33757D` | 221643 (1000,830) / (1000,895) |
| Selected clip border | `#FFFFFF` | 221643 (1000,824) |
| Top-bar action fill (`Pro`, `Share`) | `#4F4F4F` | 221643 (1650,56) |
| Dialog title bar / body / list | `#303030` / `#262626` / `#141414` | 221811 (400,55) / (400,120) / (110,160) |

### Type scale

- Measured editor sizes: 12 px for feature labels, player title, properties tabs, section headers, row labels, field values and timecode (capital height 8.5–9 px); 14 px for the top bar, sub-navigation, ruler labels and dialog rows (capital height 10 px); dialog tabs and title are 12 px (capital height 8 px). Capital heights are measured; the px size is the Inter equivalent (cap ÷ 0.72) and assumes Inter, which the capture does not prove.
- 16 / 18 / 28 px remain for modal and page headings [proposed]; no editor capture shows them.
- Weights: 400 body, 500 labels, 600 headings and tabs, 700 titles. Measured labels, tabs and section headers render semibold.
- Timecode, numeric fields and ruler figures use tabular figures.
- Inter 400/500/600/700, bundled TTF, no runtime font request.

### Iconography

- Phosphor `regular` only, embedded SVG; no other weight or set. The selected state is the accent colour on icon and label. Every button carries an icon; text-only buttons are limited to modal footers, the properties tabs and the player `Full`/`Ratio` controls.
- Sizes are boxes (glyph is about 80% of the box): 16 px in dense rows, timeline headers and feature tabs; 20 px in toolbars; 24 px for empty states. At 20 px the regular stroke is 1.25 px, matching the captured chrome's ~1.2 px.
- Feature toolbar tab [measured 2026-11-05 captures]: 16 px glyph, 5 px gap to a 12 px semibold label, 12 px horizontal padding (about 48 px minimum tab width), 42 px strip on the panel surface; inactive in text colour; selection follows the icon rule above (221643: icon x 81–97/y 82–95, label x 75–102/y 100–108).
- Properties tabs are text-only at 12 px with a 28 px gap; the active tab uses the accent (221643: y 90–99, x 1300–1676).
- As in the captured desktop app, the timeline toolbar, player controls and track-header controls are icon-only; labels appear only in the feature toolbar tabs, the top bar's primary actions and dialogs.
- Icon-only buttons are 24×24 px in the timeline toolbar and 16×16 px for the player fit button; the Shortcuts footer buttons are 72×22 text buttons. Icon-only buttons carry a 16 px icon and a tooltip naming the action and its shortcut (221643: toolbar toggles x 1382–1406/y 617–640; player fit x 959.5–975/y 513.5–529).

### Motion

- Modals 200 ms; accordions 200 ms ease-out; tooltips 150 ms.
- Scrubbing, trimming, dragging and playhead movement run at 0 ms.
- Reduced-motion preference forces 0 ms for modal and accordion transitions.

### Density

- Measured pitches: 32 px sub-navigation rows, 38–39 px property rows, 40 px shortcut rows, 54–55 px property section rows, 48 px audio rows on a 60 px pitch, 76 px video track. Field height 22 px (value, position and stepper fields); 24 px for sub-navigation buttons, sub-tab bar and asset tool row; top-bar buttons 22 px; shortcut footer buttons 22 px. The old 48 px row and 26 px field values are superseded. Export/settings modal footer buttons remain 34 px [proposed, dialog uncaptured].
- Home navigation follows [measurements](#measured-desktop-reference); bars follow [layout](#layout-grid) and [settings](#export-and-settings).
- Icon buttons follow [iconography](#iconography).
- Timeline: 22 px ruler, 76 px video track (74 px clip: 17 px top band, 39 px filmstrip, 16 px waveform), 123 px header column, 6 px scrollbar with a 24×6 thumb [measured]. Audio, text, sticker, effect, filter and adjustment track heights are unmeasured (only one video track is captured).
- Clip trim handles are 8 px wide with a 12 px capture area [proposed]; the captured clip is selected (1 px white border) and no trim handle is visible.

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
- The assets panel leads with the measured 42 px horizontal feature strip: Media, Audio, Text, Stickers, Effects, Transitions, Captions, Filters, Adjustment, Templates and the AI avatar, each a 16 px icon over a 12 px semibold label, active in the accent. The old 36 px vertical icon rail is superseded. Beneath it, a 120 px sub-navigation column and a 24 px tool row (Import/Record/search) sit above the asset grid or list. A selected element's properties view uses the measured 12 px type tabs at the 42 px strip row (active accent, no underline) and the 460×24 sub-tab bar; the empty and multi-select views keep the shared-field presentation, and `Project`/`Details` remain the inspector header switch. Geometry follows [layout](#layout-grid), styling follows [iconography](#iconography).
- Scrub number fields use the measured 22 px row: a 16×22 stepper and a typed entry in a 22 px field with an optional unit suffix; the active value previews during the drag and commits once on release.

### Preview

The preview panel follows the [measured player geometry](#measured-desktop-reference): a 42 px strip shared with the feature and properties tabs carries a 12 px semibold title and a 16 px menu; the 611×343 frame keeps 16:9 and an 8 px inset with a 1 px border; a 16 px round fit button sits centred below the frame; the footer has a 12 px tabular timecode (`current` in accent, `duration` in text, `/` in secondary), a storyboard icon, a centred play control, and `Full`/`Ratio` text buttons (12 px, outlined) with fit and fullscreen icons. Empty, loading, unavailable, invalid-timecode and snapshot failure states are explicit. Reference geometry for the snapshot, copy-frame and fullscreen header actions is unmeasured; ticket 120 decides their placement. The preview context menu is unmeasured.

### Effects

The assets Effects tab follows the captured 4-column grid [measured]: 88 px cards at 15 px horizontal gaps and a 116 px row pitch (221727: cards x 198/301/403/506, image y 192–278, label y 284–287), with a live thumbnail, name and a 28 px `+` action; the tile itself is the drag handle. The properties Effects tab reuses the measured properties sub-tab bar and 22 px field geometry: each stack row has a 28 px reorder handle, name, eye toggle and trash action, with scrub fields below. Empty, pending, failed, disabled, locked, unsupported and limit states are explicit. Reference geometry for the stack rows is unmeasured; treat their spacing as proposed until the next UI audit.

### Transform and colour controls

The Transform tab uses five existing scrub rows with an inline reset arrow:
position X/Y, independent scale X/Y and rotation. The reference's reset/value-row pattern
is recorded in `.spikes/ui-reference/06-properties/notes.md`; the classic implementation
properties registry confirms the five independent channels. The existing field
geometry and catalog tokens above remain the source for sizing and colour.

The shared colour window uses hex plus labelled HSV/alpha entries and sliders,
swatches and recent colours. Sliders preview immediately and publish once on
release; Escape/close cancels the draft. It is reused for text, text background,
graphic fill/stroke, mask stroke and canvas background. The implementation
reference's separate change/change-end events inform the interaction; the exact
window layout, sixteen-colour newest-first recent list and fixed two-pixel
graphic stroke are **proposed**, with coordinator approval to fill the uncaptured
layout gap. `inspector-transform` and `inspector-color` are the audit scenes.
The native sampler's unavailable/pending/failed copy is localized in the same
window; unavailable platforms retain all manual colour entry controls.

### Shortcuts

The desktop shortcut list is a separate 719 px-wide window that resizes with its tab (645 / 337 / 537 / 217 px tall for Timeline / Player / Basic / Other) [measured 221811, 221816, 221818, 221820]. It has a 36 px title bar, a 56 px control row, a `#141414` list panel with a 1 px column divider, and a 53 px footer. Controls: a 280×24 four-tab group (70 px cells, active `#3B3B3B`, inactive `#1C1C1C`), a 146×24 preset dropdown, 40 px list rows with the label left and key glyphs right-aligned, and 72×22 `Reset` (left) plus `Save`/`Cancel` (right) buttons. `Save` is the accent-filled primary and is disabled in the captured state. No search field or conflict/rebinding state is captured; those remain [gap].

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

- Hover, pressed, focus, drag and most disabled states are not captured; the only captured disabled states are the shortcuts `Save` button and the properties `Enhance quality` row. Light theme is not captured.
- Multi-track timeline geometry: only one video track is captured, so audio/text/sticker/effect track heights, clip colours and vertical stacking are unmeasured. The single main track's offset below the ruler is also unexplained.
- Menus, context menus, the keyframe lane, selected text/caption properties, the export dialog, and error/offline/import-failure states are not captured.
- Home and Shorts screens are only anchored (rail, banner, project thumbnails); their full geometry, cards, dialogs and states remain for ticket 117.
- Motion: no durations or easing have been measured.
- Panel min/max sizes: only one window size per screen has been observed.
- Shortcut search, rebinding capture, conflict state and preset management are not captured. The measured dialog shows four tabs, a preset dropdown, the list and Reset/Save/Cancel only.

## Evidence

Reference screenshots and behaviour notes live in `.spikes/ui-reference/` (local). UI behaviour tests and design-review captures follow [verification](../engineering/verification.md#test-selection).

### Captions workflow

Captions lives in the assets-panel tab. Generate and translation settings use
compact disclosures above a scrollable cue list; import and SRT/VTT/MP4 export
share the top action row. Times, source text and each target language stack at
panel width, with full fields on selected rows, compact unselected text, edited marks and profile-based CPS
warnings. Selection tools sit above the list; long lists page in groups of 50.
The exact-upload approval modal stays global while another tab is visible.
This extends the captured web captions/language entry in
`.spikes/ui-reference/live/06-text-captions/` using the existing desktop tokens
and field geometry. Cue editing, bilingual controls and glossary placement fill
uncaptured reference states under the coordinator-approved Captions-tab design;
the app's named captions audit scenes provide our rendered evidence.
