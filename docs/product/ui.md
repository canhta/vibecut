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
| Editor | Tool tabs, preview, properties, timeline; agent button bottom-right opens the agent chat docked bottom-right beside the timeline | Same four panels; header holds logo menu (back to projects), name, Export | [Editor window](ui-editor.md); header Window menu (focus Home) and project name; agent chat docked bottom-right |
| Long video to shorts | Own window from its Home card: upload (10 GB / 180 min, minutes quota shown), skeleton results while it works, then quick edit: list of shorts, 9:16 player, Caption style and Layout tools | none | Own window from its Home card: pick a local video (no upload, no quota), a skeleton grid while it works, then quick edit: list, 9:16 player with a bare transport row, Caption style and Layout tools, `Edit more` and `Export` |

Evidence: `.spikes/ui-reference/screens-2026-11-04/`, `01-project-list/`, `02-editor-shell/`, and the 2026-11-05 desktop capture set (index in its `notes.md`).

### Measured desktop reference

[measured] The 2026-11-05 desktop capture set (36 full-window shots, 2× Retina) replaces all earlier editor geometry. Values are logical px (native ÷ 2); the capture canvas is 1840×1142 and the app window is 1727×1029 at (56,38), using exclusive bounds (1783,1067). Citations name a shot by its capture time (HHMMSS); every sample point and region is recorded in `.spikes/ui-reference/live/tokens.md`. Desktop measurements take precedence over earlier proposed values; web-only observations do not replace desktop sizes or colours.

Each window owns its measured geometry; read only the one you work on:

| Window | Owner |
|---|---|
| Editor (frame, layout grid, density, panels, export, shortcuts, captions) | [ui-editor.md](ui-editor.md) |
| Home | [ui-home.md](ui-home.md) |
| Long video to shorts | [ui-shorts.md](ui-shorts.md) |

### Theme and tokens

The [token contract](tokens.md) links to the single executable value owner,
`theme.rs`. Geometry follows the [measured reference](#measured-desktop-reference);
colour and icon choices remain ours. Reference hex samples in the window files are capture
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

### States

- Import progress: 2 px accent bar, 14 px `Processing N%`, and a 320 px bottom-right toast.

### Gaps

A UI ticket that hits a gap captures a real desktop screenshot first.

- Hover, pressed, focus, drag and most disabled states are not captured; the only captured disabled states are the shortcuts `Save` button and the properties `Enhance quality` row. Light theme is not captured.
- Motion: no durations or easing have been measured.
- Panel min/max sizes: only one window size per screen has been observed.
- Window-specific gaps live in each window file.

## Evidence

Reference screenshots and behaviour notes live in `.spikes/ui-reference/` (local). UI behaviour tests and design-review captures follow [verification](../engineering/verification.md#test-selection).
