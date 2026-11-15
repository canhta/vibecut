# UI: Home window

Measured geometry for the Home window. Shared rules, measurement conventions, type, icons and motion: [UI design](index.md).

## Measured frame

Shot 221432, window origin (56,38), 1728×1030; logical px from the window's top-left.

- Rail 240 px wide, rows 176×32 at x=20 on a 44 px pitch (selected row `#162326`, label 12 px/500), rules 1 px and 20 px from the neighbouring rows.
- Page content x 240–1668 (60 px right margin); top band 44 px with the utility icons at y≈26, 36 px apart.
- Create banner 1428×144 at y=44 (radius 8), centred 28 px glyph tile and 24 px/600 label.
- Tool cards 242×136 (agent card 380 wide), 20 px gap, y 208–344, 44 px icon tile at 12 px inset, 13 px title at y+70 and 12 px hint at y+94.
- Agent prompt card 380×136 (measured on 221432, native / 2): a 18 px glyph and the title in a row at the top left (12 px in), a 20 px headline centred 52 px below the card top, and a 356×40 input 12 px in and 12 px above the bottom with a 24 px send button 10 px from its right edge. Our card follows these values: the title is `Agent`, the headline and placeholder are our words, the send glyph (a caret) and the italic headline face are [new], and the card's gradient fill is not built (`kit::panel`). It sits first in the tool row, with the Shorts card after the 20 px gap. Sending creates a project, opens its editor and starts the agent's first turn with the typed prompt once the consent sheet is accepted.
- Suite cards 272×174 (120 px image, 54 px text), 20 px gap, section title 14 px at x=248; section gaps 64 px.
- Projects heading 16 px/600 at x=240; toolbar controls 24 px high at y 682–706 (search 24, view 40, Trash 64, sync 99, 8 px gaps, right edge x=1668).
- Project thumbnails 124×124 (radius 6, cover-cropped) on a 146 px pitch starting x=245, 16 px below the toolbar; name 12 px/600 2 px below the thumbnail and a 12 px secondary line 16 px lower; the row runs 20 px past the banner edge.
- Trash view: the Trash control is selected and the grid lists trashed projects instead; the secondary line shows whole days left before the automatic purge, and restore or delete forever are the row menu and selection actions. Behaviour from the web reference (2026-11-10), layout from the desktop row above.
- Card surface `#111D20` and window `#071417` are samples only; colours stay ours.
- Columns fill the available width using these rail and thumbnail values; Library asset-card geometry remains unmeasured.

## Brand kit page

Capture 2026-11-14b frame 111111, a 1600 x 1142 pt region right of the rail (logical = native / 2), measured from the page's left edge. The rail's Brand kit row (after Shorts, on the 44 px pitch) opens it in place of the projects page; the reference reaches it as Home sidebar > Spaces > a space > the Brand kit tab.

- Intro banner 712 x 100 (radius 8): title `Get started with brand kit` 12 px semibold, cap top 16 px down and 24 px in; list lines 11 px on an 18 px pitch, the first cap top 39 px down, each led by a sparkle mark centred 26 px in, text 36 px in; the close glyph centred 16 px from the right and top edges. The reference's third line (`Try brand kit features in the free-trial period`) and its illustration advertise a paid trial and are not built; the fill is `kit::panel`, ours (the capture's purple is not a token).
- Sections in order Videos, Images, Stickers, Fonts, Backgrounds, Music: an 11 px label with its collapse caret, cap top 13.5 px under the block above; 120 x 80 tiles (radius 8) whose top sits 30.5 px under the label's cap top; the next label's cap top 35.5 px under the tiles (146 px section pitch). The add tile centres the section glyph (18.5 px measured, drawn at 20) with a plus beside it. The capture shows only empty sections, so a filled tile (glyph, name, a damaged or missing file's state, a menu with Rename and Remove) and the 12 px gap between tiles are ours, unmeasured.
- The reference's header card (space name, storage, Upgrade, members, cloud) and its `Space` tab belong to its online spaces and are not built; our page title `Brand kit` sits in a 24 px row with the banner 13 px below it, ours.
- A file section's add tile opens the native file picker for its kind, as the reference's Videos tile does (frame 111120); Fonts' add tile opens a family chooser, ours (the capture does not open it).

## Gaps

- Dialogs (rename, info, delete) and hover/pressed states are not captured.
