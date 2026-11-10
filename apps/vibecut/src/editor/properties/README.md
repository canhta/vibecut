# Properties

Animatable rows (Transform scale/position/rotation, Blending opacity, Audio volume) carry a diamond toggle right of the label that adds or removes a key at the playhead through `keyframes.toggle_at`. It is disabled when the playhead is outside the item and filled when a key exists.

The multi-selection inspector shows audio gain/mute and text content, font, size, colour, background and position for compatible items, a dash for mixed values and the compatible/selected count. A single element uses per-type tabs: Text (content, font, background, position, size scrub, colour), Blending (blend menu, opacity scrub), Audio (gain scrub, mute), Speed for video and audio (0.01–5x scrub, reset, and for items with audio a "Change pitch" checkbox, on while pitch follows speed, i.e. `maintain_pitch` off). Speed edits use `timeline.set_speed`, one undo step per commit; timeline trim bounds extension by the retimed output length. Caption generation refuses a retimed source item. The text button inserts a five-second title at the playhead in one undo step.

Each scrub field has a left drag handle, a click-to-type entry and clamping. A drag previews through an engine gesture and commits one `timeline.set_item_param` undo step on release; Escape cancels the gesture or draft. Changing selection or hiding the field cancels the preview. Invalid or non-finite input refuses without change. Drag sensitivity is a recorded decision: opacity and audio gain one 0.01/1 dB step per 4 px, text size 1 px per 2 px, speed 0.01 per 2 px. Enter or blur commits a typed shared-field patch through the engine registry in one undo step. Focused text editing owns keyboard input.

## Remove BG sub-tab

Video and image items get a `Remove BG` cell in the Basic sub-tab bar (reference `221751`; the web capture's panel is `screens/18`). Its `Chroma key` switch matches the reference's reveal: switching it on only stages the panel (app state, no project edit), showing the eyedropper and a disabled reset; the first eyedropper pick adds the clip's `chroma_key` effect carrying that colour and the catalog's Intensity 40 and Shadow 0 in one undo step, after which the key-colour field, the reset and the two effect scrub rows with a keyframe diamond appear. Switching it off removes the effect in one undo step; with nothing picked yet it just clears the staged switch. The eyedropper arms on one clip and the player's next click samples the retained rendered preview frame (no second decode); Escape or the button again leaves the mode without a pick, and a refused pick (for example a locked track) keeps the staged switch so the creator can retry. Reset restores the catalog key colour. Every gesture is one undo step and every value goes through an effect command; agents and the CLI that add `chroma_key` directly still get the catalog defaults.

## Effects tab

Lists the selected clip's ordered stack. Each header has a drag-reorder handle (either direction; cancels on selection or inspector change), an enable toggle and remove. Scalar parameters use the scrub contract above. Empty stacks, disabled effects, locked tracks, audio clips, unsupported kinds, missed drops and the 64-effect project limit show an explicit state or localized refusal. Edits go only through `effects.add`, `effects.reorder`, `effects.toggle`, `effects.remove` and `effects.update_param`.

## Blend and opacity inspector

The Transform tab uses scrub fields for position X/Y, independent scale X/Y and rotation, each with an inline reset. Units, bounds and defaults belong to project's `TransformField`; edits use `timeline.set_item_param`.

Visual selections show a blend dropdown (`inspector.blendMode`, options `inspector.blendMode.<persisted-id>`) and a 0..=1 opacity entry (`inspector.opacity`), mixed values marked. Both dispatch the shared-field command: one atomic undo step, unrelated fields kept. Opacity edits set a constant channel; animation goes through the keyframe commands. The desktop adapter implements the same linear-light blend formulas in WGSL and falls back to the render reference with an unchanged backdrop on unsupported device operations.

Audio and video source items share the batch inspector's volume/mute controls; volume steps 0.01 dB over -60..20 dB. `inspector.audio_gain` and `inspector.audio_muted` are stable automation IDs. Playback keeps render's `AudioMixer` per snapshot epoch; the app owns device IO and scheduling, render owns every DSP and sample decision.

## Mask

The Basic tab of a video or image item opens with a `Mask` row showing the clip's current shape or `None`; audio, text, graphic and effect items have no row. It opens a sub-panel with the capture's shapes in order (None, Horizontal, Mirror, Circle, Rectangle, Heart, Stars) and Done. The first mask is the one the row edits: a shape on an empty clip runs `masks.add`, another shape keeps centre, size and rotation through `masks.update`, and None runs `masks.remove`. One mask per clip from the panel; commands and agents keep the multi-mask engine. Below the grid the panel shows feather (0–100 px, preview while dragging, one commit on release), size W/H for rectangle/circle/heart/star (Mirror height only), position X/Y and rotation as per cent of the placed item and degrees, and invert; every field edit is one `masks.update` undo step. The reference's corner radius has no engine parameter and is not rendered. A locked track opens read-only.

Shape mapping: Horizontal is `Split` with its default quarter turn so it keeps the top half, Mirror is `CinematicBars`, Circle is `Ellipse`, Stars is `Star`; the rest keep their names. Other engine shapes (`Diamond`, `Text`, `Path`) are shown as their name but have no tile.
