# Assets workflow

Interface: `asset_kind`, `asset_row`, `asset_state_color`, `file_name`, `cuts`,
`refusal`, `selected_cut`, and the `State` methods that apply a choice.

Applies asset choices: import results, insertion (the Media "+", a drag released on the lanes
and Ctrl+V of OS files all call `insert_assets`, one `timeline.insert` per asset in one
transaction; a paste imports first and inserts at the playhead when the batch finishes), marquee selection, row
projection, effects, sounds, music and transition targets. `actions` handles every `Action`
the view returns (`State::apply`) and the start-of-frame work (`State::begin_frame`). The
media, import and selection methods mutate only the area's `State`; every project change goes
through the `Port`'s session and an engine command, and what the area cannot do itself
(toast, error line, preview resync, preference write, a cross-area `Request`) it leaves as a
port note. The area never names `Editor`.
