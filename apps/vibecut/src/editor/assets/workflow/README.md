# Assets workflow

Interface: `asset_kind`, `asset_row`, `asset_state_color`, `file_name`, `cuts`,
`refusal`, `selected_cut`, and the `Editor` methods that apply a choice.

Applies asset choices to the editor: import results, insertion, marquee selection, row
projection, sounds and transition targets. Every project change goes through an engine
command; this module never writes the project directly.
