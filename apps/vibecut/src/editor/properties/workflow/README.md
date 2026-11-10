# Properties workflow

Interface: `apply`, `transform_scrub`.

Applies property workflows: colour requests, canvas commits, command patches, effect
stack changes, numeric gestures and retiming. `apply` drains the area's actions after
the frame has released its read borrow; every project change goes through the shared
write context and therefore through an engine command.
