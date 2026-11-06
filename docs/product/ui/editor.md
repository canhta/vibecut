# UI: Editor window

Measured geometry and panel designs for the Editor window. Shared rules, measurement conventions, type, icons, motion and gaps: [UI design](index.md). Values are logical px from the 2026-11-05 desktop capture set; shot ids are capture times (HHMMSS).

## Measured frame

Shots 221513 (empty) and 221643 (populated).

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

## Layout grid

- Baseline window 1727×1029 logical px [remeasured 221643]; minimum 1280×800 remains [proposed], not verified by resizing.
- Top bar 36 px; one 42 px strip for feature tabs, player title and properties tabs; timeline toolbar 32 px [measured]. The old 52 / 44 / 40 px bars are superseded.
- Columns 582 / 628 / 486 of 1696 px excluding gutters: tools, preview, properties; full upper row and timeline are 532 / 447 of 979 px excluding the top bar, row gutter and bottom inset [remeasured 221643]. The upper row includes its area-owned 42 px strips. Persisted splitter fractions use these full panel sizes.
- Panel gutters are 7 px `#1C1C1C`, row gutters 6 px, outer inset 8 px [measured]; there is no 1 px separator between panels. In-panel dividers are 1 px `#070709` (left sub-nav) or `#373737` (properties sections).
- Radii: 8 px on editor frame panels [remeasured 221643: top edge begins x=73 at y=74, straight left edge x=65 at y=82]; square clips and in-panel regions; 4 px on fields, tabs and buttons [measured]; 12 px on dialogs [proposed]. The captured windows' own corner is about 16 px (OS chrome, 221811 and 221643 corner rows), which is not an app token. The 4 px is measured on the sub-navigation buttons, sub-tab bar and Export button corners.
- Panel dividers are draggable; assets and properties stop at 15–40%, preview at 30–85%, timeline at 15–70%, so a drag never collapses a region. Reset restores the default columns above [proposed limits].
- Export is a 67×22 px button at the top-right of the top bar [measured]; the old 120×26 value is superseded.

## Density

- Measured pitches: 32 px sub-navigation rows, 38–39 px property rows, 40 px shortcut rows, 54–55 px property section rows, 48 px audio rows on a 60 px pitch, 76 px video track. Field height 22 px (value, position and stepper fields); 24 px for sub-navigation buttons, sub-tab bar and asset tool row; top-bar buttons 22 px; shortcut footer buttons 22 px. The old 48 px row and 26 px field values are superseded. Export/settings modal footer buttons remain 34 px [proposed, dialog uncaptured].
- Bars follow [layout](#layout-grid) and [settings](#export-and-settings); icon buttons follow [iconography](index.md#iconography).
- Timeline (221643, re-measured): toolbar 32 px with a 1 px rule below; ruler 22 px (ticks start 3 px under its top: 5.5 px minor, 10.5 px labelled; `MM:SS` captions with 8 px digits, about 11 px type; ten ticks per 3 s label at 71.5 px/s); lanes start 28 px below the ruler top and end 12 px above the panel bottom, where a 6 px scrollbar with a 24×6 thumb sits 4 px down; a 5 px shadow edges the lanes at the header boundary; a single track is centred in the lanes (152 px above, 148 px below); 76 px video track (74 px clip: 17 px top band, 39 px filmstrip, 16 px waveform), 123 px header column with five 24 px cells (kind glyph, lock, eye, mute, more), playhead handle 10×15 over a 1 px line [measured]. Audio, text, sticker, effect, filter and adjustment track heights are unmeasured (proposed: audio 40 px, others 32 px).
- Clip trim handles are 8 px wide with a 12 px capture area [proposed]; the captured clip is selected (1 px white border) and no trim handle is visible.

## Local models

The model manager reuses the export modal's 12 px radius, 24 px inner margin and 34 px footer buttons at an 880 px width. A scrollable model list shows each id and its installed/download state; the selected model's detail shows provider, language scope, exact size, licence and source links, per-file SHA-256 and the policy restriction. Only the affirmative footer Download click grants transfer consent; non-permissive licences add an explicit review checkbox first. Progress, Cancel, Resume and typed storage/network/integrity errors stay in the same footer. The layout is our own application of the [layout grid](#layout-grid) and [density](#density); no external reference capture exists for it, so treat its spacing as proposed until the next UI audit.

## Export and settings

- Export is a 900x820 px modal: 227 px cover column plus settings, using [layout radii](#layout-grid) and [density](#density).
- The static export capture (`capcut/capcut-desktop-export-2k4k.png`) normalized
  from its 900x830 px dialog crop at (665,277) in the 2235x1293 image to the
  specified 900x820 size places the cover
  at x=108, the setting labels at x=449 and values at x=580, relative to the
  dialog. Video value width is about 280 px; video rows use a 48 px pitch. The 24 px
  frame inset leaves 84 px before the cover and 114 px between cover and form.
  Short viewports compact the form and retain a scrollable settings area. Name
  is the project name; destination selection remains the existing native save
  workflow after rendering, which the read-only destination row explains.
- Footer: `Duration: Ns | Size: about N MB` at 14 px, then a 90x34 px `Export` button and a 90x34 px `Cancel` button.
- Video section: Resolution (480P, 720P, 1080P, 2K, 4K, 8K), Bit rate, Codec, Format (MP4), Frame rate, Colour space; Audio is a collapsed section.
- Output settings extend the existing format/quality/audio choices: project-canvas
  resolution by default, ratio-preserving presets (including observed web 360P),
  custom even dimensions, project frame rate or 24/25/30/50/60, target bitrate and
  compatible codec/profile. Active scene and full timeline are defaults; all/named
  scene and frame-snapped in/out or selected-clip bounds are explicit choices.
  Invalid ranges disable Export. Canvas size remains in project properties;
  unsupported optical-flow and colour-space controls stay hidden.
  The captured Codec row is labelled `Video type` under the creator-copy rule;
  automatic resolves to H.264 for MP4/MOV and VP9 for WebM.
- Settings is a 480 px modal: 34 px tab bar with Draft, Edit, Performance, General; [standard rows/fields](#density), 16 px checkboxes.
- The assets panel leads with the measured 42 px horizontal feature strip: Media, Audio, Text, Stickers, Effects, Transitions, Captions, Filters, Adjustment, Templates and the AI avatar, each a 16 px icon over a 12 px semibold label, active in the accent. The old 36 px vertical icon rail is superseded. Beneath it, a 120 px sub-navigation column and a 24 px tool row (Import/Record/search) sit above the asset grid or list. A selected element's properties view uses the measured 12 px type tabs at the 42 px strip row (active accent, no underline) and the 460×24 sub-tab bar; the empty and multi-select views keep the shared-field presentation, and `Project`/`Details` remain the inspector header switch. Geometry follows [layout](#layout-grid), styling follows [iconography](index.md#iconography).
- Scrub number fields use the measured 22 px row: a 16×22 stepper and a typed entry in a 22 px field with an optional unit suffix; the active value previews during the drag and commits once on release.

## Preview

The preview panel follows the [measured player geometry](#measured-frame): a 42 px strip shared with the feature and properties tabs carries a 12 px semibold title and a 16 px menu; the 611×343 frame keeps 16:9 and an 8 px inset with a 1 px border; a 16 px round fit button sits centred below the frame; the footer has a 12 px tabular timecode (`current` in accent, `duration` in text, `/` in secondary), a storyboard icon, a centred play control, and `Full`/`Ratio` text buttons (12 px, outlined) with fit and fullscreen icons. Empty, loading, unavailable, invalid-timecode and snapshot failure states are explicit. Reference geometry for the snapshot, copy-frame and fullscreen header actions is unmeasured. The preview context menu is unmeasured.

## Effects

The assets Effects tab follows the captured 4-column grid [measured]: 88 px cards at 15 px horizontal gaps and a 116 px row pitch (221727: cards x 198/301/403/506, image y 192–278, label y 284–287), with a live thumbnail, name and a 28 px `+` action; the tile itself is the drag handle. The properties Effects tab reuses the measured properties sub-tab bar and 22 px field geometry: each stack row has a 28 px reorder handle, name, eye toggle and trash action, with scrub fields below. Empty, pending, failed, disabled, locked, unsupported and limit states are explicit. Reference geometry for the stack rows is unmeasured; treat their spacing as proposed until the next UI audit.

## Sounds

The assets Audio tab follows the captured 48 px audio row on a 60 px pitch with a
32 px thumbnail, a 24 px search row and a 120 px sub-navigation column
(221703: rows at y 188/248/308, search x 198–595/y 128–151). Our search row adds
the commercial-only switch at the right; the sub-navigation holds Discover and
Saved. Each row shows the name, `author · duration · licence` and the full
attribution text; preview, save and add actions are 24 px icon buttons at the
right. Discover is provider-backed when a credential exists and otherwise the
saved local library; Saved is always the offline local library. Empty, searching,
no-token, auth/quota/rate-limit/network, cache progress/cancel, publication and
no-audio-device states are explicit. Reference geometry for the switch and row
actions is unmeasured; treat their exact placement as proposed until the next UI
audit.

## Transform and colour controls

The Transform tab uses five existing scrub rows with an inline reset arrow: position X/Y, independent scale X/Y and rotation. The reference's reset/value-row pattern is recorded in `.spikes/ui-reference/06-properties/notes.md`; the classic implementation properties registry confirms the five independent channels. The existing field geometry and catalog tokens above remain the source for sizing and colour.

The shared colour window uses hex plus labelled HSV/alpha entries and sliders, swatches and recent colours. Sliders preview immediately and publish once on release; Escape/close cancels the draft. It is reused for text, text background, graphic fill/stroke, mask stroke and canvas background. The implementation reference's separate change/change-end events inform the interaction; the exact window layout, sixteen-colour newest-first recent list and fixed two-pixel graphic stroke are **proposed**, with coordinator approval to fill the uncaptured layout gap. `inspector-transform` and `inspector-color` are the audit scenes. The native sampler's unavailable/pending/failed copy is localized in the same window; unavailable platforms retain all manual colour entry controls.

## Canvas settings

With no element selected the inspector's Settings tab shows two sections using
the measured property rows and 22 px fields. Project holds the ratio select
(Original, 16:9, 4:3, 2:1, 9:16, 1:1, 3:4), custom width and height fields, the
frame-rate select and a Reset to original action; Original and Reset stay
disabled until the first visual clip arrives. Background holds the current
colour row, the 242 solid swatches in a scrollable grid and the shared colour
window for a custom value. Picking a preset or swatch commits one undo step;
width, height and frame rate commit on Enter or blur. The frame-rate select lists
24, 25, 30, 60 and 120; a fractional project rate (23.976, 29.97) shows its exact
label as an extra current row and stays stored as its exact rational until an
integer is picked. An odd custom width or height commits the nearest even value
on that axis (ties up), and the field shows the committed value, so every canvas
exports. This layout is **new**
(the settings panel is unmeasured) and reuses the captured property geometry.

## Shortcuts

The desktop shortcut list is a separate 719 px-wide window that resizes with its tab (645 / 337 / 537 / 217 px tall for Timeline / Player / Basic / Other) [measured 221811, 221816, 221818, 221820]. It has a 36 px title bar, a 56 px control row, a `#141414` list panel with a 1 px column divider, and a 53 px footer. Controls: a 280×24 four-tab group (70 px cells, active `#3B3B3B`, inactive `#1C1C1C`), a 146×24 preset dropdown, 40 px list rows with the label left and key glyphs right-aligned, and 72×22 `Reset` (left) plus `Save`/`Cancel` (right) buttons. `Save` is the accent-filled primary and is disabled in the captured state. No search field or conflict/rebinding state is captured; those remain [gap].

- Cmd/Ctrl+B splits at the playhead.
- I and O set in and out; Shift+X exports the in/out range.
- Space toggles playback; K pauses; J and L seek backwards/forwards one second; arrows step one frame; Shift plus arrows jump one second; Home and End go to the ends.
- Delete and Backspace remove; Cmd/Ctrl+Z undoes; Shift+Cmd/Ctrl+Z redoes; Cmd/Ctrl+C, V, D copy, paste, duplicate; N toggles snapping.
- Cmd/Ctrl+, opens settings; ? opens the shortcut list.
- Rebinding captures the next chord, rejects a key already bound to another action, and offers reset to defaults.

## Captions workflow

Captions lives in the assets-panel tab. Generate and translation settings use compact disclosures above a scrollable cue list; import and SRT/VTT/MP4 export share the top action row. Times, source text and each target language stack at panel width, with full fields on selected rows, compact unselected text, edited marks and profile-based CPS warnings. Selection tools sit above the list; long lists page in groups of 50. The exact-upload approval modal stays global while another tab is visible. This extends the captured web captions/language entry in `.spikes/ui-reference/live/06-text-captions/` using the existing desktop tokens and field geometry. Cue editing, bilingual controls and glossary placement fill uncaptured reference states under the coordinator-approved Captions-tab design; the app's named captions audit scenes provide our rendered evidence.

## Gaps

- Multi-track timeline geometry: only one video track is captured, so audio/text/sticker/effect track heights, clip colours and vertical stacking are unmeasured. The single main track's offset below the ruler is also unexplained.
- Menus, context menus, the keyframe lane, selected text/caption properties, and error/offline/import-failure states are not captured.
- Panel min/max sizes: only one window size has been observed.
- Shortcut search, rebinding capture, conflict state and preset management are not captured. The measured dialog shows four tabs, a preset dropdown, the list and Reset/Save/Cancel only.
