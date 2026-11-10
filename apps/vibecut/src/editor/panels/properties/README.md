# Properties

Interface: `State`, `Action`, `Widget`, `show`, `apply`, `element_kind`, `ElementKind`,
`PropertiesTab`, `SharedField`, `shared_fields`, `color_picker`, `font_picker`,
`font_picker::State`.

The area owns its drafts, scrub gestures, picker and popovers, its font catalog
backend, the effect reorder draft and its gesture contracts, and returns `Action`
values the frame routes. A workflow reads the editor through the shared read view and
writes through the shared desk; every project change goes through an engine command and
the area never writes project JSON.


Animatable rows (Transform scale, position and rotation, Blending opacity, Audio
volume) carry a diamond toggle right of the label that adds or removes a key at the
playhead through `keyframes.toggle_at`. It is disabled when the playhead is outside
the item and filled when a key exists.

The multi-selection inspector shows audio gain and mute plus text content, font, size,
colour, background and position for compatible items, a dash for mixed values and the
compatible/selected count. A single element uses per-type tabs: Text (content, font,
background, position, size scrub, colour), Blending (blend menu, opacity scrub), Audio
(gain scrub, mute), and Speed for video and audio. Speed is a 0.01–5x scrub with a
reset and, for items with audio, a `Change pitch` checkbox that is on while pitch
follows speed (`maintain_pitch` off). Speed edits use `timeline.set_speed`, one undo
step per commit; trim bounds extend by the retimed output length. Caption generation
refuses a retimed source item. The text button inserts a five-second title at the
playhead in one undo step.

Each scrub field has a left drag handle, a click-to-type entry and clamping. A drag
previews through an engine gesture and commits one `timeline.set_item_param` undo step
on release; Escape cancels the gesture or draft. Changing selection or hiding the
field cancels the preview. Invalid or non-finite input refuses without change. Drag
sensitivity: opacity 0.01 per 4 px, audio gain 0.01 dB per 4 px, text size 1 px per
2 px, speed 0.01 per 2 px. Enter or blur commits a typed shared-field patch through the
engine registry in one undo step. Focused text editing owns keyboard input.

## Remove BG sub-tab

Video and image items get a `Remove BG` cell in the Basic sub-tab bar. Switching
`Chroma key` on only stages the panel in app state and shows the eyedropper and a
disabled reset. The first eyedropper pick adds the clip's `chroma_key` effect carrying
that colour and the catalog's Intensity 40 and Shadow 0 in one undo step; the
key-colour field, the reset and the two effect scrub rows then appear. Switching it off
removes the effect in one undo step; with nothing picked yet it just clears the staged
switch. The eyedropper arms on one clip and the player's next click samples the
retained rendered preview frame (no second decode); Escape or the button again leaves
the mode without a pick, and a refused pick keeps the staged switch. Reset restores the
catalog key colour. Every value goes through an effect command; a command that adds
`chroma_key` directly gets the catalog defaults.

## Effects tab

Lists the selected clip's ordered stack. Each header has a drag-reorder handle (either
direction; cancels on selection or inspector change), an enable toggle and remove.
Scalar parameters use the scrub contract above. Empty stacks, disabled effects, locked
tracks, audio clips, unsupported kinds, missed drops and the engine's effect limit show
an explicit state or localized refusal. Edits go only through `effects.add`,
`effects.reorder`, `effects.toggle`, `effects.remove` and `effects.update_param`.

## Blend and opacity

Visual selections show a blend dropdown (`inspector.blendMode`, options
`inspector.blendMode.<persisted-id>`) and a 0..=1 opacity entry (`inspector.opacity`),
mixed values marked. Both dispatch the shared-field command: one atomic undo step,
unrelated fields kept. Opacity edits set a constant channel; animation goes through the
keyframe commands.

Audio and video source items share the batch inspector's volume and mute controls;
volume steps 0.01 dB over -60..20 dB. `inspector.audio_gain` and
`inspector.audio_muted` are stable automation IDs.

## Mask

The Basic tab of a video or image item opens with a `Mask` row showing the clip's
current shape or `None`; audio, text, graphic and effect items have no row. It opens the
[mask picker](mask_picker/README.md) sub-panel. The first mask is the one the row
edits: a shape on an empty clip runs `masks.add`, another shape keeps centre, size and
rotation through `masks.update`, and None runs `masks.remove`. One mask per clip from
the panel; commands and agents keep the multi-mask engine. A locked track opens
read-only.

Panel design and measured controls: [editor window](../../../../../../docs/product/ui/editor.md).
