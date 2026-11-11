# UI: Editor window

Measured geometry and panel designs for the Editor window. Shared rules: [UI design](index.md). Values are logical px; shot ids are capture times (HHMMSS).

## Measured frame

Imported 3D cube LUT controls in Adjust are a newly adopted composition, not a measured reference match: after the four grade controls, a LUT selector with Import, a static Strength row and conditional Locate for missing data. They reuse the existing 39 px property rows, 22 px fields, 12 px type and kit icons; no new visual token. The [Properties owner](../../../apps/vibecut/src/editor/panels/properties/README.md) defines capture/invalidation and file-completion behaviour. Original-file movement does not lose an imported look; missing copied bytes produce a visible typed failure until matching relink.

Shots 221513 (empty) and 221643 (populated). Window 1727×1029, minimum 1280×800 [proposed].

- Frame: top bar 36 px; one 42 px strip below it carrying the feature tabs (left), the player title (centre) and the properties tabs (right); content row 490 px; 6 px gutter; timeline 447 px; 8 px bottom inset. Content:timeline = 52:48 excluding the strip; full upper row:timeline = 532:447 (221643: column x=300, y 38–1067; row y=200 and y=700).
- Columns 582 / 628 / 486 px = 34 / 37 / 28 % of the 1696 px content width (1710 px inner width minus two 7 px gutters). The upper row includes its area-owned 42 px strips; persisted splitter fractions use these full panel sizes.
- Gutters: 7 px `#1C1C1C` between panels, 6 px between rows, an 8–9 px outer inset of the same colour; no 1 px separator between panels. In-panel dividers are 1 px `#070709` (left sub-navigation) or `#373737` (properties sections). Panel surfaces `#262626`; the 42 px strip `#303030` (221643: x 647–653 and 1282–1288; x 65–1774 at y 120).
- Radii: 8 px on frame panels (221643: top edge begins x=73 at y=74, straight left edge x=65 at y=82); 4 px on fields, tabs and buttons (sub-navigation buttons, sub-tab bar, Export button); 12 px on dialogs [proposed]. Clips and in-panel regions are square. The captured window's own 16 px corner is OS chrome, not an app token.
- Panel dividers are draggable; assets and properties stop at 15–40 %, preview at 30–85 %, timeline at 15–70 % [proposed], so a drag never collapses a region. Reset restores the default columns.
- Top bar: autosave `#949494` at the left, project name `#FAFAFA` centred, both 14 px; right group ends 11 px from the window edge: two 18 px layout icons, a 50×22 `Pro` pill and a 62×22 `Share` button (both `#4F4F4F`), a 67×22 `Export` button (`#57BECB`, white label). Buttons are 22 px high and start 7 px below the bar top (221643: x 1511–1772, y 45–67; text y 50–61).
- Feature tabs: 16 px icon over a 12 px/600 label, 5 px apart, 7 px inset top and bottom, content-width with about 12 px horizontal padding (min 48 px), pitch 49–61 px. Active icon and label `#57BECB`; inactive `#E0E0E0`; no underline (221643: icon x 81–97, label x 75–102, icon y 82–95, label y 100–108; icon centres 89, 137.5, 186.5, 236, 285.5, 340, 394.5, 445, 501.5, 562, 616).
- Sub-navigation: 120 px column with a 1 px `#070709` divider; 104×24 px buttons at 8 px inset on a 32 px pitch, `#3B3B3B` fill, 14 px label, 12 px chevron; selected label `#57BECB`; plain rows share the 32 px pitch (221643: x 65–185, buttons x 73–177, y 131–155 and 163–187).
- Asset panel: 24 px tool row (Import 72×24 with a dashed outline, Record 62×24, search 148×24 on `#1C1C1C`), a 12 px `All` filter row, a 120×73 asset thumbnail with a 6 px gap to a 12 px name; audio rows 48 px on a 60 px pitch with a 32×32 thumbnail; effects grid 4 columns of 88 px cards, 15 px gaps, 116 px row pitch (221635: y 128–151, Import x 198–269, card x 198–318/y 188–261; 221703: cards y 188, 248, 308; 221727: cards x 198, 301, 403, 506).
- Player: 12 px/600 `#FFFFFF` title and a 16 px menu in the 42 px strip; 611×343 frame (16:9) inset 8 px with a 1 px `#FAFAFA` border; a 16 px white round fit button with a dark glyph, centred 16 px below it (221643: x 959.5–975, y 513.5–529); footer row y 576–592 with a 12 px tabular timecode (current `#57BECB`, duration `#E0E0E0`, slash `#404040`), a 16 px storyboard icon, a 12 px play triangle centred, and 12 px `Full`/`Ratio` outlined labels with 16 px fit/fullscreen icons at the right (221643: panel x 654–1281, frame x 662–1273/y 154–497, footer x 663–1273).
- Properties: 12 px/600 tabs (active `#57BECB`, inactive `#E0E0E0`) with a 28 px gap and 11 px left inset; a 461×24 four-cell sub-tab bar (about 115 px cells, active `#3B3B3B` with a 1 px `#333333` outline, inactive `#1C1C1C`) starting 13 px below the strip; 1 px `#373737` dividers; collapsed section rows 54 px on a 55 px pitch with a 16 px reset and a `<◇>` 10 px keyframe pair; label-left/control-right rows on a 38–39 px pitch, with a 55 px slider row (221643: tabs y 90–99 at x 1300–1676; sub-tabs x 1301.5–1762.5/y 129–153; dividers y 423/478/533/588; labels y 222/277/315/354).
- Property controls: 54×22 value field (`#1C1C1C`) plus a 16×22 stepper (`#3B3B3B`) 2 px to its right; 68×22 position fields each followed by a 16×22 stepper 6 px to its right, the Y field 7 px after the X stepper (group x 1421–1608, 187 px, flood-measured); 336×2 slider (white filled up to the 8×12 thumb, `#404040` rest); 36×16 toggle; 12×12 checkbox; 294×22 alignment bar with 7 cells (221643: value x 1655–1709/y 233–255, stepper 1711–1727; position fields x 1421–1489 and 1518–1586, steppers x 1495–1511 and 1592–1608, y 309–331; slider x 1301–1636/y 243–245, thumb x 1366–1374/y 238–250; toggle x 1727–1762/y 274–290).
- Timeline toolbar: 32 px with a 1 px rule below; first icon 10 px from the panel edge; 24×24 icon buttons with 16 px glyphs on a 24–32 px pitch and 1 px `#3B3B3B` group dividers 16 px high (y 620–636); right group mic, three 24×24 toggles (`#1B1B1C`, `#57BECB` glyph) 12 px apart, fit, −, a 96×2 zoom slider with an 8×12 thumb, + (221643: y 612–643, buttons y 617–640, toggles x 1382/1418/1454, slider x 1639–1735).
- Ruler 22 px: ticks start 3 px under its top (5.5 px minor, 10.5 px labelled); major labels every 3 s at this zoom (214.5 px apart, 71.5 px/s, ten ticks per label), minor ticks every 0.3 s (21.4 px); `#6C6C6C` 14 px tabular `MM:SS` labels with 8 px digits, each preceded by a 1 px tick (221643: y 645–666; labels x 257–288, 471–501, 686–715).
- Tracks: lanes start 28 px below the ruler top and end 12 px above the panel bottom, where a 6 px scrollbar with a 24×6 `#404040` thumb sits 4 px down; a 5 px shadow edges the lanes at the header boundary. The single main track sits 156 px below the ruler with 160 px of empty track area below it. 123 px header column with five 24 px cells (kind glyph, lock, eye, mute, more) at x 73/99/122/146/171 and a 10 px vertical inset; video track 76 px; clip 74 px with a 1 px white selection border, `#1E4C51` fill, a 17 px teal top band, a 39 px filmstrip and a 16 px waveform band (`#33757D`). Playhead 1 px white with a 10×15 outlined handle in the ruler (221643: header x 65–188, track y 823–898, clip y 824–897, filmstrip y 842–880, waveform y 881–896, playhead x 417, handle x 412–421/y 645–659). Stacking above or below the first track is [gap].

## Density

Per-panel pitches are in the frame bullets above. Shortcut list rows are 40 px; export and
settings modal footer buttons are 34 px [proposed, dialog uncaptured]. Icon buttons follow
[iconography](index.md#iconography). Audio, text, sticker, effect, filter and adjustment
track heights are unmeasured [proposed: audio 40 px, others 32 px]. Clip trim handles are
8 px wide with a 12 px capture area [proposed]; the captured clip is selected and shows no
handle.

## Timeline gestures

Move and resize stage the group move and trim contract candidate: the timeline paints `preview_project()` while the committed project stays untouched, and one drop publishes a single undo step. Escape or a refusal discards the candidate.

- A move drop highlights the landing row with a 2 px accent outline on a 10 % accent wash, or draws a 2 px accent insertion line with an 8×4 left cap between lanes; the line marks the track a release creates. [proposed]
- A refused move or trim shows the engine's reason in an 11 px warning pill at the bottom-left of the lanes, 1 px warning outline on the panel fill, above the scrollbar. [proposed]
- A main-track item at the timeline origin is pinned: a 3 px accent edge marks the pinned start beside the left trim handle, and its tooltip explains the pin. [proposed]

## Marker editor

A 256 px popover opens 8 px below the clicked ruler bookmark, clamped inside the window. It stacks a note field, a colour swatch with a reset to the default marker colour, a duration field in seconds stepped by 0.1, and a full-width delete button, with 12 px between sections. Escape closes the colour window first, then the editor; no project change happens until a field commits as one undo step. [new] — no reference capture exists; width, padding, gap and offset come from the implementation reference (`bookmarks.tsx`: `w-64`, `gap-3`, `p-3`, `sideOffset={8}`), which the web editor copy does not implement.

## Preview gestures

Drag and scale stage the engine preview candidate: the player paints `preview_project()` while the committed project stays untouched, and one drop publishes a single undo step. Escape or a return to the gesture origin discards the candidate.

- A selected item shows a 2 px accent box, 10 px corner and side handles and an 18 px rotation grip 12 px beyond the box's bottom midpoint with a 12 px rotate glyph; the pointer target around each handle is 18 px. Four corners and the left/right side midpoints are draggable [observed: 2026-11-06 captures of a selected text item].
- Corner and side handles show diagonal or axis resize cursors and the grip shows a grab cursor; a drag on the item body moves it. [proposed]
- Move, scale and rotate snap to the canvas centre and edges and to other visible items' box edges, each axis independently, within 5 screen px at the preview scale; rotation snaps to 90° steps within 5°. Shift bypasses snapping for the gesture. A 1 px dashed accent guide marks each active target and disappears on release; a snapped rotation draws the item's own axis through its centre [observed: 2026-11-10 live web editor, where a 5 px offset still snapped and 6 px did not; the rotation pair and guide style follow the implementation reference].
- Hidden and locked items expose no handles and cannot be grabbed.

## Local models

The model manager reuses the export modal's 12 px radius, 24 px inner margin and 34 px footer buttons at an 880 px width. A scrollable model list shows each id and its installed or download state; the selected model's detail shows size, licence, source links and per-file SHA-256, with progress, Cancel, Resume and typed errors in the footer. No capture exists; spacing is [proposed]. Licence gates and consent: [model manager](../../../apps/vibecut/src/editor/dialogs/models/README.md).

## Export and settings

- Export is a 900×820 px modal: 227 px cover column plus settings, using the frame's radii and density.
- From the export capture (`capcut/capcut-desktop-export-2k4k.png`, dialog crop 900×830 at (665,277) in a 2235×1293 image, normalized to 900×820): cover at x=108, setting labels at x=449, values at x=580, relative to the dialog. Video value width about 280 px; video rows on a 48 px pitch. The 24 px frame inset leaves 84 px before the cover and 114 px between cover and form. Short viewports compact the form and keep a scrollable settings area.
- Footer: `Duration: Ns | Size: about N MB` at 14 px, then a 90×34 px `Export` button and a 90×34 px `Cancel` button.
- Settings is a 480 px modal: 34 px tab bar with Draft, Edit, Performance, General; standard rows and fields, 16 px checkboxes.
- The assets panel leads with the 42 px horizontal feature strip: Media, Audio, Text, Stickers, Effects, Transitions, Captions, Filters, Adjustment, Templates and the AI avatar, each a 16 px icon over a 12 px semibold label, active in the accent. Beneath it, a 120 px sub-navigation column and a 24 px tool row (Import, Record, search) sit above the asset grid or list. A selected element's properties view uses the 12 px type tabs at the strip row (active accent, no underline) and the 460×24 sub-tab bar; the empty and multi-select views keep the shared-field presentation, and `Project`/`Details` are the inspector header switch.

Options, ranges and filename rules: [export dialog](../../../apps/vibecut/src/editor/dialogs/export/README.md). Scrub fields: [properties](../../../apps/vibecut/src/editor/panels/properties/README.md).

## Preview

Geometry is in the [measured frame](#measured-frame). Empty, loading, unavailable, invalid-timecode and snapshot-failure states are explicit. Geometry for the snapshot, copy-frame and fullscreen header actions and for the preview context menu is unmeasured.

## Effects

The assets Effects tab is a 4-column grid: 88 px cards, 15 px horizontal gaps, 116 px row pitch (221727: cards x 198/301/403/506, image y 192–278, label y 284–287), each with a live thumbnail, name and a 28 px `+` action; the tile is the drag handle. The properties Effects tab reuses the properties sub-tab bar and 22 px fields: each stack row has a 28 px reorder handle, name, eye toggle and trash action, with scrub fields below. Empty, pending, failed, disabled, locked, unsupported and limit states are explicit. Stack-row spacing is [proposed].

## Sounds

The assets Audio tab uses the 48 px audio row on a 60 px pitch with a 32 px thumbnail, a 24 px search row and a 120 px sub-navigation column (221703: rows at y 188/248/308, search x 198–595/y 128–151). The search row adds a commercial-only switch at the right; the sub-navigation holds Discover and Saved. Each row shows the name, `author · duration · licence` and the full attribution text; preview, save and add are 24 px icon buttons at the right. Discover is provider-backed when a credential exists and otherwise the saved local library; Saved is always the offline local library. Empty, searching, no-token, auth, quota, rate-limit, network, cache progress and cancel, publication and no-audio-device states are explicit. Switch and row-action placement is [proposed].

## Transform and colour controls

The Transform tab has five scrub rows with an inline reset arrow: position X and Y, independent scale X and Y, and rotation. The reset and value-row pattern is in `.spikes/ui-reference/06-properties/notes.md`.

The shared colour window has hex plus labelled HSV and alpha entries and sliders, swatches and recent colours. Sliders preview immediately and publish once on release; Escape or close cancels the draft. It serves text, text background, graphic fill and stroke, mask stroke, marker colour and canvas background. Layout, the sixteen-colour newest-first recent list and the fixed two-pixel graphic stroke are [proposed] (coordinator-approved). `inspector-transform` and `inspector-color` are the audit scenes. The native sampler's unavailable, pending and failed copy is localized in the same window; unavailable platforms keep all manual entry controls.

## Remove background

Video and image items carry a `Remove BG` cell in their Basic sub-tab bar (desktop shot 221751 places it second of Basic, Remove BG, Mask, Retouch; our bar is Transform, Blending, Remove BG because Mask is a row in that group). The cell holds the panel from the web capture's Smart tools ▸ Remove background (`screens/18`), with the key colour, reset, `Intensity` (0–100, default 40) and `Shadow` (0–100, default 0). Row heights and the value column follow the property rows above; the three-cell bar keeps the measured 24 px control height and 4 px radius. [new] for the panel composition; the reveal order, toggle, eyedropper, reset and slider set are the web capture's measured controls. Reveal order and undo steps: [properties](../../../apps/vibecut/src/editor/panels/properties/README.md#remove-bg-sub-tab).

## Font picker

The text Font row opens a 320 px popover below the field, clamped inside the window: a search field, the source tabs All fonts, My fonts and Favorites, and inside All fonts a System/Google filter plus a disabled Custom category. Category chips use the app selected-chip roles: accent fill with on-accent ink for the chosen chip, field fill with text ink otherwise. Rows use fixed star, name, state and action columns, all left-aligned, and only informative rows carry state text on the row's centre line. No capture exists; the 320 px width, 288 px list ceiling and row spacing are [proposed]. Tabs, filters and download states: [font picker](../../../apps/vibecut/src/editor/panels/properties/font_picker/README.md).

## Canvas settings

With no element selected the inspector's Settings tab shows three sections using the property rows and 22 px fields. Project holds the ratio select (Original, 16:9, 4:3, 2:1, 9:16, 1:1, 3:4), custom width and height fields, the frame-rate select and a Reset to original action; Original and Reset stay disabled until the first visual clip arrives. Background holds the current colour row, the 242 solid swatches in a scrollable grid and the shared colour window for a custom value. Picking a preset or swatch commits one undo step; width, height and frame rate commit on Enter or blur. The frame-rate select lists 24, 25, 30, 60 and 120; a fractional project rate (23.976, 29.97) shows its exact label as an extra current row and stays stored as its exact rational until an integer is picked. An odd custom width or height commits the nearest even value on that axis (ties up), and the field shows the committed value. This layout is [new]; the settings panel is unmeasured.

Timeline previews follows Background in the scrollable Settings body, reusing section headers,
property rows, 22 px fields and the trash glyph. Refresh/Clear become Cancel during work;
failed operations offer Retry. Size is verified file bytes in the two shared process media
stores; the scope tooltip names exclusions and concurrent regeneration. Snapshot age, retained,
in-use, missing and failed counts accompany partial results; removed roots request restart.
This composition is new and has no matching reference image for comparison metrics.

## Shortcuts

The shortcut list is a separate 719 px-wide window that resizes with its tab (645, 337, 537 and 217 px tall for Timeline, Player, Basic and Other; 221811, 221816, 221818, 221820). It has a 36 px title bar, a 56 px control row, a `#141414` list panel with a 1 px column divider, and a 53 px footer. Controls: a 280×24 four-tab group (70 px cells, active `#3B3B3B`, inactive `#1C1C1C`), a 146×24 preset dropdown, 40 px list rows with the label left and key glyphs right-aligned, and 72×22 `Reset` (left) plus `Save` and `Cancel` (right) buttons. `Save` is the accent-filled primary and is disabled in the captured state. The key list and rebinding rules are in [keyboard](../agent-control/keyboard.md).

## Captions workflow

Captions lives in the assets-panel tab. Generate and translation settings use compact disclosures above a scrollable cue list; import and SRT, VTT, TXT and MP4 export share the top action row. Times, source text and each target language stack at panel width, with full fields on selected rows, compact unselected text, edited marks and profile-based CPS warnings. Selection tools sit above the list. The exact-upload approval modal stays global while another tab is visible. Cue editing, bilingual controls and glossary placement fill uncaptured states ([proposed], coordinator-approved); the named captions audit scenes are our rendered evidence. Reference: `.spikes/ui-reference/live/06-text-captions/`. Behaviour: [captions panel](../../../apps/vibecut/src/editor/captions/README.md).

The Captions Style chooser sits below the caption-item selector, above cue tools, using the
shared 22 px field and 12 px label system. Outline/Box and Custom/Mixed describe the current
saved style. This uncaptured composition is new; desktop reference parity is unverified.
Availability and edit behavior belong to the [subtitles owner](../../../apps/vibecut/src/subtitles/README.md).

The selected-Text Animation row is a new, explicitly adopted composition: Fade in,
Fade out and Both use the current segmented control, 22 px fields, 12 px type and
property-row spacing. Tooltips explain editable opacity keys and the need to undo or
remove existing keys before changing a fade. No matching selected-title desktop
reference exists; these captures are non-comparable for reference metrics. Eligibility
and input lifetime belong to [Properties](../../../apps/vibecut/src/editor/panels/properties/README.md).

## Gaps

- Multi-track timeline geometry: only one video track is captured, so audio, text, sticker and effect track heights, clip colours and vertical stacking are unmeasured.
- Menus, context menus, the keyframe lane, selected text and caption properties, and error, offline and import-failure states are not captured.
- Shortcut search, rebinding capture, conflict state and preset management are not captured.

The Captions Style chooser lists Saved in project as explicit copy actions, retaining Outline/Box/Custom/Mixed as value summaries. Save current style and Manage saved styles open a small name editor using the existing 22 px/12 px control system. Manage remains available for a nonempty bank without a caption target. Enter/blur confirms once; Escape/Cancel changes nothing. Snapshot, target, time, tab and readiness changes discard stale input. This bank composition is adopted new UI, with reference parity unverified.

### Generated Shapes

The Stickers tab now contains a local Shapes shelf with Rectangle and Ellipse only. The adopted composition uses existing kit tokens,39px Properties rows,22px fields,12px labels and4px corners, with English/Vietnamese labels. Graphic Properties expose alpha-capable fill/stroke, stroke width/alignment and rectangle corner radius with ordinary key controls. Captures are source-backed new composition; no matching reference screenshot is available, so they are noncomparable rather than a metric pass. Placement and transform limits are owned by [render](../../../crates/render/src/scene/README.md#generated-graphic-geometry).

The keyframe-lane Copy/Paste menu and selected-item Paste row adopt the current menu
pattern, localized labels, existing icons and 20 px lanes as a new composition. Copy
uses selected keys; Paste can target an existing zero-key Param. Scope/draft admission:
[frame input](../../../apps/vibecut/src/editor/frame/input/README.md#clipboard-actions).
No comparable captured menu reference exists; product/native parity remains unverified.

## AI preferences

Window / Settings opens the AI model rows, grouped by provider and feature. The
shipped default comes first; Refresh adds models for the selected key, followed by
recent choices and a custom-ID field with Apply. Without a key or offline, default
and custom remain usable. Refresh alone requests the remote list. Pending, empty,
failed and stale-key states are explicit. Unknown-price upload sheets require a
separate checkbox naming the exact model already disclosed above it. These new
compositions use the existing 22 px fields and are unmeasured against desktop
reference; `ai-preferences` and `ai-preferences-vi` are the audit scenes.
