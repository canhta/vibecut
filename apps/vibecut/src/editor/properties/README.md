# Properties

Animatable inspector rows carry a diamond toggle at the right of the label column:
Transform scale/position/rotation, Blending opacity and Audio volume add or remove
a key at the playhead through `keyframes.toggle_at`; the control is disabled when
the playhead is outside the item and filled when a key already exists.

The selection inspector shows audio gain/mute and text content, font, size, colour,
background and position for compatible selected items, with a dash for mixed
values and the compatible/selected count. A single element uses the per-type tabs
above instead: Text has content, font, background and position fields alongside a
size scrub and colour entry, Blending a blend menu and opacity scrub, Audio a
gain scrub and mute checkbox, and Speed (video and audio) a 0.01-5x scrub with a
reset button and, when the item has audio, a "Change pitch" checkbox. "Change
pitch" is on while pitch follows speed (`maintain_pitch` off), as in the reference.
Speed edits go through `timeline.set_speed`, one undo step per commit; the timeline
trim handles bound extension by the retimed output length. Caption generation
refuses a retimed source item until captions gain a rate-aware mapping.
`cargo xtask ui-audit inspector-speed` captures the populated Speed tab in both
themes at 1x/2x.
Each scrub field has a left drag handle, a click-to-type entry and clamps; the
drag previews through an engine gesture, commits as one registered
`timeline.set_item_param` undo step on release, and Escape cancels the gesture or
the typed draft. Changing selection or hiding the scrub field cancels its
preview before another edit can join the gesture. Invalid or non-finite input refuses without changing the item.
Drag sensitivity is our recorded decision because the inventory leaves it
unverified: opacity and audio gain move one 0.01/1 dB step per 4 px, and text size
one px per 2 px, speed 0.01 per 2 px. Enter or blur commits a typed shared-field patch through the
engine registry in one undo step. Text editing owns keyboard input while focused.
The text button inserts a five-second title at the playhead in one undo step.
Video volume awaits its owning contract before joining the same inspector.

The properties Effects tab lists the selected clip's ordered stack. Each header
has a drag-to-reorder handle, enable/disable toggle and remove action; registered
scalar parameters use the same scrub/typed field contract as other inspector
fields. Reorder gestures move in either direction and cancel when the selection
or inspector changes. Empty stacks, disabled effects, locked tracks, audio clips, unsupported
kinds, missed drops and the project's 64-effect limit render an explicit state or
localized refusal reason.

Every edit goes through `effects.add`, `effects.reorder`, `effects.toggle`,
`effects.remove` or `effects.update_param`; the UI never mutates a project copy.
`tests/it/effects_ui.rs` covers drag/drop, reorder, toggle, remove, parameter
gesture reversal/cancel/clamping, refusal boundaries and preview job states.

## Blend and opacity inspector

The Transform tab reuses the inspector's scrub fields for position X/Y, independent
scale X/Y and rotation, with an inline reset on each row. Units, bounds and defaults
belong to project's `TransformField`; every edit uses `timeline.set_item_param`.

Visual selections show a localized blend dropdown (`inspector.blendMode`) and
0..=1 opacity entry (`inspector.opacity`); options use
`inspector.blendMode.<persisted-id>`. Mixed selections show the existing mixed
value marker. Both controls dispatch the registered shared-field command, giving
one atomic undo step and retaining unrelated fields. Blend labels use the same
two-language catalog; opacity edits set a constant channel. Animated editing
continues through the engine keyframe commands. The desktop adapter implements
the same linear-light blend formulas in WGSL; unsupported device operations
fall back to the render reference with the unchanged backdrop.
`cargo xtask ui-audit inspector-blend inspector-blend-vi` renders these inspector
scenes through the shared audit registry in both themes and at 1x/2x.

Audio and video source items share the batch inspector's volume/mute controls;
volume uses a 0.01 dB step over -60..20 dB. Existing `inspector.audio_gain` and
`inspector.audio_muted` automation IDs remain stable. Playback retains render's
`AudioMixer` for each snapshot epoch; the app owns device IO and scheduling
adapters, while render owns every DSP/sample decision.
