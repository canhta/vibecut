# UI design

## Principles

- Layout cloned, palette and icons ours. The editor matches the reference captures in layout: panel structure and proportions, control sizes, spacing, radii, type scale, row and tab composition. Colour comes from our tokens and icons from our own set ([iconography](#iconography)); a reference colour or glyph is never copied, and a colour or icon difference is not a mismatch. Where a reference handles an interaction poorly for creators editing talking videos, say so and cite the reason; a design without a reference basis is marked new and listed in Gaps. Compare against the local reference set (code and screenshots).
- Three independent windows. Home, Editor and Long video to shorts each have their own size, top bar and chrome, matched against their own captures. Components and tokens are shared only through the common kit, never a frame.
- Creator words, few of them. Write every string from the view of a creator editing a video. A control an icon makes clear carries the icon and a tooltip, no visible label; visible text is limited to feature tabs, the top bar's primary actions, dialogs and values the creator reads or types. No section headings that repeat the panel or tab name, no instructional sentences in empty panels (an icon and at most four words), no internal names (provider, endpoint, payload, codec, session, snapshot, ASR, ticket or milestone ids), no "arrives later" placeholders (hide what does not work yet). An error says what happened to the creator's work and the one thing to do, in at most one short sentence; the technical cause goes to the log or a details disclosure. Number formats follow the reference (whole percents, no trailing zeros, no ranges like "(0-1)"). A reviewer returns `changes` for a failing string and for an added string without a reason an icon could not carry it.
- Familiar first. Standard panel placement, shortcuts and gestures beat novelty.
- Every state designed. Each panel has empty, loading, error and offline states; nothing appears blank or frozen.
- Agent-drivable. Every interactive widget has a stable automation id that survives locale and layout changes. Agents and tests send input through those ids into the real command path, read engine project state plus app-owned widget state, and capture a window or panel screenshot that waits for a presented frame and returns a typed unavailable or timeout failure.
- Decide visually, from source values. Choose an icon set, colour or layout from a side-by-side render at matched size against the reference. Take geometry from measured reference shots and colour from our tokens, not from screenshot pixels.
- Two languages. All strings come from the English and Vietnamese catalogs; layouts survive the longer string.

## Design language

### Screens

| Screen | Reference A (desktop) | Reference B (web) | Ours |
|---|---|---|---|
| Home | Rail: Home, Templates; "Create with AI" (generative studios); Library, Spaces. Content: `Create project` tile, agent prompt card beside AI tool cards, AI suite cards (incl. `Long video to shorts`), then `Projects` with search, view, trash | Projects page: breadcrumb, view toggle, search, `New project`, grid; no rail | Rail: Home, Library. Content: `Create project` tile, `Long video to shorts` card, agent prompt card once the agent is in scope, then `Projects` (search, sort, grid/list, multi-select). Generative studio, image/video generation and ad-maker cards are out of scope |
| Editor | Tool tabs, preview, properties, timeline; agent button bottom-right opens the agent chat docked bottom-right beside the timeline | Same four panels; header holds logo menu (back to projects), name, Export | [Editor window](editor.md); header Window menu (focus Home) and project name; agent chat docked bottom-right |
| Long video to shorts | Own window from its Home card: upload (10 GB / 180 min, minutes quota shown), skeleton results while it works, then quick edit: list of shorts, 9:16 player, Caption style and Layout tools | none | Own window from its Home card: pick a local video (no upload, no quota), a skeleton grid while it works, then quick edit: list, 9:16 player with a bare transport row, Caption style and Layout tools, `Edit more` and `Export` |

Evidence: the local reference set (project-list and editor-shell screens, and the 2026-11-05 desktop capture set with its notes).

### Measured desktop reference

[measured] The 2026-11-05 desktop capture set (36 full-window shots, 2× Retina) is the geometry source. Values are logical px (native ÷ 2); the capture canvas is 1840×1142 and the app window is 1727×1029 at (56,38), using exclusive bounds (1783,1067). Citations name a shot by capture time (HHMMSS); every sample point and region is recorded in the local reference set. Desktop measurements win over web-only observations.

Each window owns its measured geometry; read only the one you work on:

| Window | Owner |
|---|---|
| Editor (frame, layout grid, density, panels, export, shortcuts, captions) | [editor.md](editor.md) |
| Home | [home.md](home.md) |
| Long video to shorts | [shorts.md](shorts.md) |

### Theme and tokens

The [token contract](tokens.md) links to the executable value owner, `theme.rs`. Geometry follows the measured reference; colour and icon choices are ours. Reference hex samples are capture evidence, not app tokens.

### Type scale

- Measured editor sizes: 12 px for feature labels, player title, properties tabs, section headers, row labels, field values and timecode (capital height 8.5–9 px); 14 px for the top bar, sub-navigation, ruler labels and dialog rows (capital height 10 px); dialog tabs and title are 12 px (capital height 8 px). Capital heights are measured; the px size is the Inter equivalent (cap ÷ 0.72) and assumes Inter, which the capture does not prove.
- 16 / 18 / 28 px for modal and page headings [proposed]; no editor capture shows them.
- Weights: 400 body, 500 labels, 600 headings and tabs, 700 titles. Measured labels, tabs and section headers render semibold.
- Timecode, numeric fields and ruler figures use tabular figures.
- Inter 400/500/600/700, bundled TTF, no runtime font request.

### Iconography

- Phosphor `regular` only, embedded SVG. The selected state is the accent colour on icon and label. Every button carries an icon; text-only buttons are limited to modal footers, the properties tabs and the player `Full`/`Ratio` controls.
- Sizes are boxes (glyph is about 80% of the box): 16 px in dense rows, timeline headers and feature tabs; 20 px in toolbars; 24 px for empty states. At 20 px the regular stroke is 1.25 px, matching the captured chrome's ~1.2 px.
- Measured editor boxes, tab composition and corner radii: [editor measured frame](editor.md#measured-frame).
- The timeline toolbar, player controls and track-header controls are icon-only; labels appear only in the feature toolbar tabs, the top bar's primary actions and dialogs.

### Motion

- Modals 200 ms; accordions 200 ms ease-out; tooltips 150 ms.
- Scrubbing, trimming, dragging and playhead movement run at 0 ms.
- Reduced-motion preference forces 0 ms for modal and accordion transitions.

### States

- Import progress: 2 px accent bar, 14 px `Processing N%`, and a 320 px bottom-right toast.
- Panel state (empty, loading, offline, error): one 24 px glyph over one 12 px line of at most four words, centred in the panel; secondary ink, error ink for a failure. Kit: `panel_state`.
- Progress bar: 2 px accent over the field track, full width, percentage in the line above. Kit: `progress_bar`.
- Toast: 320 px wide on the modal surface, bottom-right, 16 px from both edges, 12 px corners and inset (the import toast's values, unmeasured). Kit: `toast`.
- Dialog: 12 px corners, 24 px inset, 18 px title and 90×34 footer buttons come from the export capture and apply to every modal (a decision, not a proposal); the primary button is filled with the accent. Kit: `dialog_frame`, `dialog_title`, `dialog_button`.

### Gaps

A change that hits a gap captures a real desktop screenshot first.

- Hover, pressed, focus, drag and most disabled states are not captured; the only captured disabled states are the shortcuts `Save` button and the properties `Enhance quality` row. Light theme is not captured.
- Motion durations and easing are unmeasured.
- Panel min/max sizes: one window size per screen observed.
- No glyph for offline or error exists in the icon set, and loading is a static glyph (no spinner); the toast values are unmeasured, and empty states are centred in their panel without a capture.
- Window-specific gaps live in each window file.

## Evidence

Reference screenshots and behaviour notes live in the local reference set. UI behaviour tests and design-review captures follow [verification](../../engineering/verification.md#test-selection).
