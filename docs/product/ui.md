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

## Evidence

Reference screenshots and behaviour notes live in `.spikes/ui-reference/` (local).
A UI ticket cites the reference it took each decision from and verifies with a
screenshot of our app driven through automation ids.
