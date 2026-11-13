# Properties workflow

The LUT workflow captures its target before requesting the frame-owned file picker, submits the registered engine job through shared process media access, and applies only a completed descriptor whose session, snapshot, selection, lock and tab still match. The frame owns picker lifecycle and polling composition; the area's state is restored before it can receive another action. No area reads another area's private state or edits the project outside engine commands.

Interface: `add_effect_to_item`, `apply`, `effect_refusal`,
`set_item_field`, `set_shared_field`, `transform_scrub`.

Applies property workflows: colour requests, canvas commits, command patches, effect
stack changes, numeric gestures and retiming. A workflow reads the editor through the
shared read view and writes only through the shared desk, so no area method reaches the
`Editor` fields; `apply` drains the area's actions after the frame has released its read
borrow and lends the area its own state apart from the editor (`Desk::lend_properties`).
The colour-picker open and the LUT pick completion are `State` entry points
(`open_color_picker`, `lut_picked_file`); the editor facade only lends the state. Every project change goes
through the shared write context and therefore through an engine command.

Refused header activation preserves the current Project/Details choice and does not
stop later requests in the routing pass. Colour commits validate the full recent-colour
preference candidate before publication; a refused candidate changes neither project
history nor the remembered colour.
