# Properties workflow

The LUT workflow captures its target before requesting the frame-owned file picker, submits the registered engine job through shared process media access, and applies only a completed descriptor whose session, snapshot, selection, lock and tab still match. The frame owns picker lifecycle and polling composition; the area's state is restored before it can receive another action. No area reads another area's private state or edits the project outside engine commands.

The footage workflow (`jobs/footage.rs`) starts `footage.analyze` for a stabilized, background-removed or flicker-corrected clip through shared process media access with its own runner, keyed by clip and correction kind, advances the running jobs once per frame, publishes a finished or failed job through `job_status`, shows progress, cancels on request and keeps the set of switched-on corrections that still lack a stored one. A refusal for a missing model is kept per clip and kind so the row can link to the model manager. It re-reads that set only when the project snapshot or the cache's correction epoch changes, then asks the frame to resync the preview so playback reads the new correction.

Interface: `add_effect_to_item`, `apply`, `effect_refusal`,
`set_item_field`, `set_shared_field`, `transform_scrub`.

Applies property workflows: colour requests, canvas commits, command patches, effect
stack changes, numeric gestures and retiming. A workflow reads the session through the
port's read view and writes only through the port, so no area method reaches the
`Editor` fields; the frame lends the area its own state and a port for each action
(`Editor::with_properties`) and applies the notes the step left before the next one.
An `Activate` action becomes a `Request::Action` the frame runs once the state is back.
The colour-picker open and the LUT pick completion are `State` entry points
(`open_color_picker`, `lut_picked_file`); the editor facade only lends the state. Every project change goes
through the port and therefore through an engine command.

Refused header activation preserves the current Project/Details choice and does not
stop later requests in the routing pass. Colour commits validate the full recent-colour
preference candidate before publication; a refused candidate changes neither project
history nor the remembered colour.
