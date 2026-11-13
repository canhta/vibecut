# Kit widgets

Interface: `action_button`, `badge_button`, `dashed_outline`, `icon_button`, `icon_tab`,
`rail_slider`, `section_header_with`, `segmented`, `stepper`, `text_tabs`, `toggle`,
`transport_button`, `value_field`, `zoom_rail`.

Renders shared controls without application state. Callers keep their labels, tooltips,
automation IDs, preferences and preview/commit/rollback decisions. A widget enters the kit
with its first caller outside it; one without a caller is deleted, never kept for later.
