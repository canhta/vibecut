# UI: Long video to shorts window

Measured geometry for the Long video to shorts window. Shared rules: [UI design](index.md).

## Measured frame

Shots 221941 pick, 221946 working, 222002 and 222010 quick edit. Window content is 1718×989 below the 36 px title bar, a 5 px frame excluded; logical px from the content's top-left.

- Pick page: two-line 38 px/600 hero centred 65 px left of the page centre at y 76 and 125; drop card 920×440 at y 200 (radius 16, dashed frame 24 px inside, 44 px icon at y+154, 18 px label at y+202, 200×40 button at y+271).
- Pick page, Recent row (reference 221941, the `My projects` row; the heading is `Recent` here): 18 px/600 heading centred at y 691, left edge on the drop card's; cards from y 718, 217×122 with a black fill, a 1 px border and radius 8, the `N shorts` chip 12 px at x+13 and 20 px above the card's bottom, the source name on one line 16 px below the card, cut with an ellipsis. Open: the gap between cards and the second row's pitch (the reference shows one card), the heading and name font sizes (read from the shot, not measured), the remove control (ours, a 24 px quiet X inside the card's top right, the reference has none) and the card for the run in progress (ours, first in the row).
- Working page, ours beside the reference: the back arrow returns to the entry and the run goes on; `Cancel` is a 96×32 button 12 px right of the bar (unmeasured, the reference has none); the header count follows the shorts produced so far.
- Working page: 16 px bar at y 68 (868 wide); 4 columns of 321 px 9:16 placeholders on a 346 px pitch from y 101, row pitch 687; three title lines under each (16/39/65 px below, widths 100/60/50 %); title row 18 px/600 centred at y 34.
- Quick edit: list 360 px wide flush left with two 156×277 cards at x 16 and 188 on a 362 px pitch, 14 px titles (three lines) 10 px below each card and a 2 px outline 4 px outside the selected card; main panel 8 px inset (radius 12) with a 22×76 fold handle on its left edge at the vertical centre.
- Quick-edit header: buttons 40 px high at 12 px from the panel top (`Edit more` 97 wide, `Export` 99 wide, 24 px from the right edge, 8 px apart), title 14 px at x+24.
- Player 411×731 (9:16) at 128 px below the panel top, centred between the panel's left edge and its right edge minus 92 px, with a 59 px bar below (16 px play at x+23, 14 px `current / duration`, 3 px scrub line at y+40).
- Tool strip 56 px wide, 8 px from the panel's right edge and 129 px below its top, each tool 52 px (20 px glyph over an 11 px label); the open tool's panel is 242 px wide, 13 px left of the strip, same top, radius 12, 16 px padding, 16 px/600 title and a close icon.

## Gaps

- Not built (the engine only crops to 9:16): results grid with inline players, caption template grid, Fill/Fit/4:3/Split layout presets.
- A control stays only if a capture shows it and an engine command backs it. The trim tool, quick-edit header icons, options popover and a separate results grid have no capture and are hidden.
