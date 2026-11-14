# Kit widgets

Interface: `action_button`, `badge_button`, `checkbox`, `dashed_outline`, `icon_button`, `icon_tab`,
`keyframe_cluster` (with `KeyframeControl`), `rail_slider`, `section_header_with`,
`segmented`, `stepper`, `text_tabs`, `toggle`, `transport_button`, `value_field`, `zoom_rail`.

`keyframe_cluster` is the row-end `< diamond >` control: three hit cells, previous key,
toggle and next key, in one `KEYFRAME_CLUSTER`-wide rect. It paints and reports; the caller
owns the keys, the seek and the toggle. An arrow with no key on its side is disabled.
`section_header_with` takes an optional enable checkbox before the label; the click is the
caller's to apply.

Renders shared controls without application state. Callers keep their labels, tooltips,
automation IDs, preferences and preview/commit/rollback decisions. A widget enters the kit
with its first caller outside it; one without a caller is deleted, never kept for later.
