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

### Layout grid

- Baseline window 1440x900; minimum 1280x800.
- Top bar 52 px; panel header 44 px; properties tab row 40 px; timeline toolbar
  40 px.
- Columns 25 / 50 / 25 percent: tools, preview, properties.
- Rows 60 / 40 percent: main content, timeline.
- Separators are 1 px `#1F1F1F`; there is no gutter between panels.
- Radii: 0 px on editor panels, 4 px on fields, 12 px on modals.
- Panel dividers are draggable; the 25/50/25 split is the reset value.
- Export is a 120x26 px button at the top-right of the top bar.
- The library uses a 270 px left rail and 240 px project cards; columns fill the
  remaining width.

### Theme and tokens

- Dark and light both ship in v1. Dark: window and editor chrome `#0F0F0F`;
  top bar `#131313`.
- Light theme tokens: pending; same geometry, type and density as dark.
- Editor panels use `#0F0F0F`; they are not lighter than the window.
- Export modal `#262626`, fields `#303034`; settings modal `#141414`, controls
  `#262626`, active tab `#3B3B3B`.
- Text `#E5E5E5`; secondary text `#9A9A9A`; separators `#1F1F1F`; borders
  `#383838`.
- Accent `#00C1CD` for selection, checked boxes and the primary button.
- Video clips use a [proposed] `#0A2A24` teal border around the filmstrip.

### Type scale

- Four sizes: 14, 16, 18, 28 px; line heights 1.4, 1.35, 1.3, 1.2.
- 14 px metadata, labels, tabs and values; 16 px panel and section headings;
  18 px modal titles; 28 px page headings.
- Weights: 400 body, 500 labels, 600 headings, 700 modal titles.
- Timecode and numeric fields use tabular figures.
- Inter 400/500/600/700, bundled TTF, no runtime font request.

### Iconography

- One outline set at 16, 20 and 24 px; 1.5 px stroke.
- Properties tabs are text-only at 14 px; the active tab uses `#00C1CD`.
- Icon-only buttons are 28x28 px.

### Motion

- Modals 200 ms; accordions 200 ms ease-out; tooltips 150 ms.
- Scrubbing, trimming, dragging and playhead movement run at 0 ms.
- Reduced-motion preference forces 0 ms for modal and accordion transitions.

### Density

- Row pitch 48 px; field height 26 px; settings tab bar 34 px; properties tab
  row 40 px; library nav row 46 px; top bar 52 px.
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
- Space or K plays; J and L shuttle; arrows step one frame; Shift plus arrows
  jump one second; Home and End go to the ends.
- Delete and Backspace remove; Cmd/Ctrl+Z undoes; Shift+Cmd/Ctrl+Z redoes;
  Cmd/Ctrl+C, V, D copy, paste, duplicate; N toggles snapping.
- Cmd/Ctrl+, opens settings; ? opens the shortcut list.
- Rebinding captures the next chord, rejects a key already bound to another
  action, and offers reset to defaults.

### States

- Import progress: 2 px `#00C1CD` bar, 14 px `Processing N%`, and a 320 px
  bottom-right toast.

### Gaps

A UI ticket that hits a gap captures a real desktop screenshot first.

- Error, offline and import-failure states: no real desktop screenshot; no card
  size, colour or action is fixed.
- Audio and text clip colours: only a video clip has been sampled.
- Multi-track timeline geometry: only one video track has been sampled.
- Motion: no durations or easing have been measured.
- Shortcut dialog: keys are documented; layout, search and conflict UI are
  unmeasured.
- Panel min/max sizes: only one size per screenshot has been observed.

## Evidence

Reference screenshots and behaviour notes live in `.spikes/ui-reference/` (local).
A UI ticket cites the reference it took each decision from and verifies with a
screenshot of our app driven through automation ids.
