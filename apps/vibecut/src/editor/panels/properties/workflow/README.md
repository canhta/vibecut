# Properties workflow

Interface: `add_effect_to_item`, `apply`, `effect_refusal`,
`set_item_field`, `set_shared_field`, `transform_scrub`.

Applies property workflows: colour requests, canvas commits, command patches, effect
stack changes, numeric gestures and retiming. A workflow reads the editor through the
shared read view and writes only through the shared desk, so no area method reaches the
`Editor` fields; `apply` drains the area's actions after the frame has released its read
borrow and lends the area its own state apart from the editor. Every project change goes
through the shared write context and therefore through an engine command.
