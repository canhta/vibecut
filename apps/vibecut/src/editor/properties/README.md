# Properties

Animatable rows (Transform scale/position/rotation, Blending opacity, Audio volume) carry a diamond toggle right of the label that adds or removes a key at the playhead through `keyframes.toggle_at`. It is disabled when the playhead is outside the item and filled when a key exists.

The multi-selection inspector shows audio gain/mute and text content, font, size, colour, background and position for compatible items, a dash for mixed values and the compatible/selected count. A single element uses per-type tabs: Text (content, font, background, position, size scrub, colour), Blending (blend menu, opacity scrub), Audio (gain scrub, mute), Speed for video and audio (0.01–5x scrub, reset, and for items with audio a "Change pitch" checkbox, on while pitch follows speed, i.e. `maintain_pitch` off). Speed edits use `timeline.set_speed`, one undo step per commit; timeline trim bounds extension by the retimed output length. Caption generation refuses a retimed source item. The text button inserts a five-second title at the playhead in one undo step.

Each scrub field has a left drag handle, a click-to-type entry and clamping. A drag previews through an engine gesture and commits one `timeline.set_item_param` undo step on release; Escape cancels the gesture or draft. Changing selection or hiding the field cancels the preview. Invalid or non-finite input refuses without change. Drag sensitivity is a recorded decision: opacity and audio gain one 0.01/1 dB step per 4 px, text size 1 px per 2 px, speed 0.01 per 2 px. Enter or blur commits a typed shared-field patch through the engine registry in one undo step. Focused text editing owns keyboard input.

## Effects tab

Lists the selected clip's ordered stack. Each header has a drag-reorder handle (either direction; cancels on selection or inspector change), an enable toggle and remove. Scalar parameters use the scrub contract above. Empty stacks, disabled effects, locked tracks, audio clips, unsupported kinds, missed drops and the 64-effect project limit show an explicit state or localized refusal. Edits go only through `effects.add`, `effects.reorder`, `effects.toggle`, `effects.remove` and `effects.update_param`.

## Blend and opacity inspector

The Transform tab uses scrub fields for position X/Y, independent scale X/Y and rotation, each with an inline reset. Units, bounds and defaults belong to project's `TransformField`; edits use `timeline.set_item_param`.

Visual selections show a blend dropdown (`inspector.blendMode`, options `inspector.blendMode.<persisted-id>`) and a 0..=1 opacity entry (`inspector.opacity`), mixed values marked. Both dispatch the shared-field command: one atomic undo step, unrelated fields kept. Opacity edits set a constant channel; animation goes through the keyframe commands. The desktop adapter implements the same linear-light blend formulas in WGSL and falls back to the render reference with an unchanged backdrop on unsupported device operations.

Audio and video source items share the batch inspector's volume/mute controls; volume steps 0.01 dB over -60..20 dB. `inspector.audio_gain` and `inspector.audio_muted` are stable automation IDs. Playback keeps render's `AudioMixer` per snapshot epoch; the app owns device IO and scheduling, render owns every DSP and sample decision.
