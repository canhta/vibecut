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
| Editor | Tool tabs, preview, properties, timeline; agent button bottom-right opens the agent chat docked bottom-right beside the timeline | Same four panels; header holds logo menu (back to projects), name, Export | Layout below; header Window menu (focus Home) and project name; agent chat docked bottom-right |
| Long video to shorts | Own window from its Home card: upload (10 GB / 180 min, minutes quota shown), skeleton results while it works, then quick edit: list of shorts, 9:16 player, Caption style and Layout tools | none | Own window from its Home card: pick a local video (no upload, no quota), a skeleton grid while it works, then quick edit: list, 9:16 player with a bare transport row, Captions, Trim and Layout tools, `Edit more` and `Export` |

Evidence: `.spikes/ui-reference/screens-2026-11-04/`, `01-project-list/`, `02-editor-shell/`, and the 2026-11-05 desktop capture set (index in its `notes.md`).

### Measured desktop reference

[measured] The 2026-11-05 desktop capture set (36 full-window shots, 2× Retina) replaces all earlier editor geometry. Values are logical px (native ÷ 2); the capture canvas is 1840×1142 and the app window is 1727×1029 at (56,38), using exclusive bounds (1783,1067). Citations name a shot by its capture time (HHMMSS); every sample point and region is recorded in `.spikes/ui-reference/live/tokens.md`. Desktop measurements take precedence over earlier proposed values; web-only observations do not replace desktop sizes or colours.

**Editor frame** (shots 221513 empty, 221643 populated):

- Window 1727×1029. Top bar 36 px; a single 42 px strip below it carries the feature tabs (left), the player title (centre) and the properties tabs (right); content row 490 px; 6 px gutter; timeline 447 px; 8 px bottom inset. Content:timeline = 52:48 excluding the strip; full upper row:timeline = 532:447 (221643: column x=300, y 38–1067; row y=200 and y=700).
- Columns left 582 / player 628 / properties 486 px = 34 / 37 / 28 % of the 1696 px content width (1710 px inner width minus two 7 px gutters). Panels are separated by a 7 px `#1C1C1C` gutter, not a 1 px divider; the 8–9 px outer inset is the same colour. Panel surfaces `#262626`, the 42 px strip `#303030` (221643: x 647–653 and 1282–1288; x 65–1774 at y 120).
- Top bar: autosave `#949494` at the left, project name `#FAFAFA` centred, both 14 px; right group ends 11 px from the window edge: two 18 px layout icons, a 50×22 `Pro` pill and a 62×22 `Share` button (both `#4F4F4F`), a 67×22 `Export` button (`#57BECB`, white label). Buttons are 22 px high and start 7 px below the bar top (221643: x 1511–1772, y 45–67; text y 50–61).
- Feature tabs: 16 px icon over a 12 px/600 label, 5 px apart, 7 px inset top and bottom, content-width with about 12 px horizontal padding (min 48 px), pitch 49–61 px. Active icon and label `#57BECB`; inactive `#E0E0E0`; no underline (221643: icon x 81–97, label x 75–102, icon y 82–95, label y 100–108; icon centres 89, 137.5, 186.5, 236, 285.5, 340, 394.5, 445, 501.5, 562, 616).
- Sub-navigation: 120 px column with a 1 px `#070709` divider; 104×24 px buttons at 8 px inset on a 32 px pitch, `#3B3B3B` fill, 14 px label, 12 px chevron; selected label `#57BECB`; plain rows share the 32 px pitch (221643: x 65–185, buttons x 73–177, y 131–155 and 163–187).
- Asset panel: 24 px tool row (Import 72×24 with a dashed outline, Record 62×24, search 148×24 on `#1C1C1C`), a 12 px `All` filter row, a 120×73 asset thumbnail with a 6 px gap to a 12 px name; audio rows 48 px on a 60 px pitch with a 32×32 thumbnail; effects grid 4 columns of 88 px cards, 15 px gaps, 116 px row pitch (221635: y 128–151, Import x 198–269, card x 198–318/y 188–261; 221703: cards y 188, 248, 308; 221727: cards x 198, 301, 403, 506).
- Player: 12 px/600 `#FFFFFF` title and a 16 px menu in the 42 px strip; 611×343 frame (16:9) inset 8 px with a 1 px `#FAFAFA` border; a 16 px white round fit button with a dark glyph, centred 16 px below it (221643: x 959.5–975, y 513.5–529); footer row y 576–592 with a 12 px tabular timecode (current `#57BECB`, duration `#E0E0E0`, slash `#404040`), a 16 px storyboard icon, a 12 px play triangle centred, and 12 px `Full`/`Ratio` outlined labels with 16 px fit/fullscreen icons at the right (221643: panel x 654–1281, frame x 662–1273/y 154–497, footer x 663–1273).
- Properties: 12 px/600 tabs (active `#57BECB`, inactive `#E0E0E0`) with a 28 px gap and 11 px left inset; a 461×24 four-cell sub-tab bar (about 115 px cells, active `#3B3B3B` with a 1 px `#333333` outline, inactive `#1C1C1C`) starting 13 px below the strip; 1 px `#373737` dividers; collapsed section rows 54 px on a 55 px pitch with a 16 px reset and a `<◇>` 10 px keyframe pair; label-left/control-right rows on a 38–39 px pitch, with a 55 px slider row (221643: tabs y 90–99 at x 1300–1676; sub-tabs x 1301.5–1762.5/y 129–153; dividers y 423/478/533/588; labels y 222/277/315/354).
- Property controls: 54×22 value field (`#1C1C1C`) plus a 16×22 stepper (`#3B3B3B`) 2 px to its right; 68×22 position fields each followed by a 16×22 stepper 6 px to its right, the Y field 7 px after the X stepper (group x 1421–1608, 187 px; flood-measured, the earlier 66.5 and 15.5 were wrong); 336×2 slider (white filled up to the 8×12 thumb, `#404040` rest); 36×16 toggle; 12×12 checkbox; 294×22 alignment bar with 7 cells (221643: value x 1655–1709/y 233–255, stepper 1711–1727; position fields x 1421–1489 and 1518–1586, steppers x 1495–1511 and 1592–1608, y 309–331; slider x 1301–1636/y 243–245, thumb x 1366–1374/y 238–250; toggle x 1727–1762/y 274–290).
- Timeline toolbar: 32 px; first icon 10 px from the panel edge; 24×24 icon buttons with 16 px glyphs on a 24–32 px pitch and 1 px `#3B3B3B` group dividers 16 px high (y 620–636); right group mic, three 24×24 toggles (`#1B1B1C`, `#57BECB` glyph) 12 px apart, fit, −, a 96×2 zoom slider with an 8×12 thumb, + (221643: y 612–643, buttons y 617–640, toggles x 1382/1418/1454, slider x 1639–1735).
- Ruler 22 px: major labels every 3 s at this zoom (214.5 px apart), minor ticks every 0.3 s (21.4 px), `#6C6C6C` 14 px tabular labels, each preceded by a 1 px tick (221643: y 645–666; labels x 257–288, 471–501, 686–715).
- Tracks: 123 px header column with 16 px control glyphs at x 73/99/122/146/171, 10 px vertical inset; video track 76 px; clip 74 px with a 1 px white selection border, `#1E4C51` fill, a 17 px teal top band, a 39 px filmstrip and a 16 px waveform band (`#33757D`). Playhead 1 px white with a 10×15 outlined handle in the ruler; horizontal scrollbar 6 px with a 24×6 `#404040` thumb on the panel colour (221643: header x 65–188, track y 823–898, clip y 824–897, filmstrip y 842–880, waveform y 881–896, playhead x 417, handle x 412–421/y 645–659).
- The single main track sits 156 px below the ruler with 160 px of empty track area below it; why is not visible, so stacking above or below the first track is [gap].
- Home (shot 221432, window origin (56,38), 1728×1030; re-measured by ticket 117, logical px from the window's top-left): rail 240 px wide, rows 176×32 at x=20 on a 44 px pitch (selected row `#162326`, label 12 px/500), rules 1 px and 20 px from the neighbouring rows; page content x 240–1668 (60 px right margin); top band 44 px with the utility icons at y≈26, 36 px apart; create banner 1428×144 at y=44 (radius 8), centred 28 px glyph tile and 24 px/600 label; tool cards 242×136 (agent card 380 wide), 20 px gap, y 208–344, 44 px icon tile at 12 px inset, 13 px title at y+70 and 12 px hint at y+94; suite cards 272×174 (120 px image, 54 px text), 20 px gap, section title 14 px at x=248; section gaps 64 px; Projects heading 16 px/600 at x=240, toolbar controls 24 px high at y 682–706 (search 24, view 40, Trash 64, sync 99, 8 px gaps, right edge x=1668); project thumbnails 124×124 (radius 6, cover-cropped) on a 146 px pitch starting x=245, 16 px below the toolbar, name 12 px/600 2 px below the thumbnail and a 12 px secondary line 16 px lower, row runs 20 px past the banner edge. 115's 1441×143 banner at y=37 was a mis-read of the gradient edge; its rail, thumbnail and pitch values hold. Card surface `#111D20` and window `#071417` are samples only; colours stay ours.

- Long video to shorts (shots 221941 pick, 221946 working, 222002 and 222010 quick edit; ticket 126; window content is 1718×989 below the 36 px title bar, a 5 px frame excluded; logical px from the content's top-left): pick page: two-line 38 px/600 hero centred 65 px left of the page centre at y 76 and 125, drop card 920×440 at y 200 (radius 16, dashed frame 24 px inside, 44 px icon at y+154, 18 px label at y+202, 200×40 button at y+271). Working page: 16 px bar at y 68 (868 wide), 4 columns of 321 px 9:16 placeholders on a 346 px pitch from y 101, row pitch 687, three title lines under each (16/39/65 px below, widths 100/60/50 %), title row 18 px/600 centred at y 34. Quick edit: list 360 px wide flush left with two 156×277 cards at x 16 and 188 on a 362 px pitch, 14 px titles (three lines) 10 px below each card and a 2 px outline 4 px outside the selected card; main panel 8 px inset (radius 12) with a 22×76 fold handle on its left edge at the vertical centre; header buttons 40 px high at 12 px from the panel top (`Edit more` 97 wide, `Export` 99 wide, 24 px from the right edge, 8 px apart), title 14 px at x+24; player 411×731 (9:16) at 128 px below the panel top, centred between the panel's left edge and its right edge minus 92 px, with a 59 px bar below (16 px play at x+23, 14 px `current / duration`, 3 px scrub line at y+40); tool strip 56 px wide, 8 px from the panel's right edge and 129 px below its top, each tool 52 px (20 px glyph over an 11 px label); the open tool's panel is 242 px wide, 13 px left of the strip, same top, radius 12, 16 px padding, 16 px/600 title and a close icon.

### Layout grid

- Baseline window 1727×1029 logical px [remeasured 221643]; minimum 1280×800 remains [proposed], not verified by resizing.
- Top bar 36 px; one 42 px strip for feature tabs, player title and properties tabs; timeline toolbar 32 px [measured]. The old 52 / 44 / 40 px bars are superseded.
- Columns 582 / 628 / 486 of 1696 px excluding gutters: tools, preview, properties; full upper row and timeline are 532 / 447 of 979 px excluding the top bar, row gutter and bottom inset [remeasured 221643]. The upper row includes its area-owned 42 px strips. Persisted splitter fractions use these full panel sizes.
- Panel gutters are 7 px `#1C1C1C`, row gutters 6 px, outer inset 8 px [measured]; there is no 1 px separator between panels. In-panel dividers are 1 px `#070709` (left sub-nav) or `#373737` (properties sections).
- Radii: 8 px on editor frame panels [remeasured 221643: top edge begins x=73 at y=74, straight left edge x=65 at y=82]; square clips and in-panel regions; 4 px on fields, tabs and buttons [measured]; 12 px on dialogs [proposed]. The captured windows' own corner is about 16 px (OS chrome, 221811 and 221643 corner rows), which is not an app token. The 4 px is measured on the sub-navigation buttons, sub-tab bar and Export button corners.
- Panel dividers are draggable; assets and properties stop at 15–40%, preview at 30–85%, timeline at 15–70%, so a drag never collapses a region. Reset restores the default columns above [proposed limits].
- Export is a 67×22 px button at the top-right of the top bar [measured]; the old 120×26 value is superseded.
- Home columns fill available width using the [measured rail and thumbnails](#measured-desktop-reference); Library asset-card geometry remains unmeasured.

### Theme and tokens

The [token contract](tokens.md) links to the single executable value owner,
`theme.rs`. Geometry follows the [measured reference](#measured-desktop-reference);
colour and icon choices remain ours. Reference hex samples above are capture
evidence, not replacement app tokens. Dark/light and status colours keep the
existing app palette; there is no second numeric palette table to maintain.

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
- Corner radii (221643 corner scans): sub-tab bar and its active cell 4 px; value fields and steppers about 2 px. Row labels are regular weight, section titles 600.
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
- Timeline (221643, re-measured): toolbar 32 px with a 1 px rule below; ruler 22 px (ticks start 3 px under its top: 5.5 px minor, 10.5 px labelled; `MM:SS` captions with 8 px digits, about 11 px type; ten ticks per 3 s label at 71.5 px/s); lanes start 28 px below the ruler top and end 12 px above the panel bottom, where a 6 px scrollbar with a 24×6 thumb sits 4 px down; a 5 px shadow edges the lanes at the header boundary; a single track is centred in the lanes (152 px above, 148 px below); 76 px video track (74 px clip: 17 px top band, 39 px filmstrip, 16 px waveform), 123 px header column with five 24 px cells (kind glyph, lock, eye, mute, more), playhead handle 10×15 over a 1 px line [measured]. Audio, text, sticker, effect, filter and adjustment track heights are unmeasured (proposed: audio 40 px, others 32 px).
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
- Home is measured in full (ticket 117); its dialogs (rename, info, delete) and hover/pressed states are not captured. Shorts (ticket 126) is measured for the pick, working and quick-edit pages; the results grid with inline players, the caption template grid and the Fill/Fit/4:3/Split layout presets are captured but not built, because the engine only crops to 9:16.
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
