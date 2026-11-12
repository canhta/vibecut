# Assets workflow

Interface: `asset_kind`, `asset_row`, `asset_state_color`, `file_name`, `cuts`,
`refusal`, `selected_cut`, and the `State` methods that apply a choice.

Applies asset choices to the editor: import results, insertion, marquee selection, row
projection, effects, sounds and transition targets. The media, import and selection methods
mutate only the area's `State`; every project change goes through the shared write context
and an engine command.
