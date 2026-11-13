# Properties

The Adjust area exposes imported LUT selection, Import, static Strength and a Locate action for the typed missing-resource state. This is an adopted new composition using the existing 39 px row, 22 px field and 12 px type kit, without a reference-parity claim. The workflow lends the shared process media access to one bounded runner and captures Session/revision/snapshot/item before the async picker. Selection, project/revision, lock, tab and transaction changes invalidate before polling or applying a result; cancellation/disposal uses the runner's existing join. File completion follows the real workspace picker seam; dismissing that picker releases the capture and enables Import without editing the project. Import/register/application is one undo step; selection and strength use ordinary edits, and strength remains static.

Interface: `State`, `Action`, `Widget`, `show`, `apply`, `element_kind`, `ElementKind`,
`PropertiesTab`, `SharedField`, `shared_fields`, `ColorTarget`, `font_picker`,
`font_picker::State`.

The area owns its drafts, scrub gestures, picker and popovers, its font catalog
backend, the effect reorder draft and its gesture contracts, and returns `Action`
values the frame routes. A workflow reads the editor through the shared read view and
writes through the shared desk; every project change goes through an engine command and
the area never writes project JSON. Colour targets and their draft-to-command mapping
belong here; the target-free controls come from the shared
[colour service](../../../services/color_picker/README.md). A colour commit validates
the complete preference candidate before ending coalescing or publishing the project
edit and adopts it only after that edit succeeds, without a preference undo step.


Animatable rows (Transform scale, position and rotation, Blend opacity, Audio
volume) carry a diamond toggle right of the label that adds or removes a key at the
playhead through `keyframes.toggle_at`. It is disabled when the playhead is outside
the item and filled when a key exists.

The Transform section also carries two reflection toggles below Rotate. They activate
`timeline.flip_horizontal` / `timeline.flip_vertical`, the same registered commands as
the timeline toolbar, so a Properties click and a toolbar click share one typed edit,
one undo step and the retained selection. Both toggles are disabled while their axis
carries keys; Reset writes the five numeric channels and both reflections as one
atomic batch and refuses entirely if an axis is keyed.

A video or image item's Transform section starts with a Fit mode row: a dropdown (`inspector.fit`, options `inspector.fit.<id>`) over Fit, Fill, Stretch, Original size and Manual that sends `timeline.set_item_param` field `fit`, one undo step per choice. The row is [new]; the reference shows no clip fit mode. The scale rows keep their meaning under every mode: 100% is the fitted size. Choosing Manual records the fit into the scale rows first, and Reset returns a Manual item to Fill. Text, sticker, graphic, audio and multi-selection show no Fit row.

The same section's Arrange row carries six canvas alignments and two even-spacing
actions. They activate the registered `timeline.align_*` / `timeline.distribute_*`
actions, so keyboard, panel, CLI and agent paths share one command family. Align works
for one or more selected video, image, text, overlay or graphic items; distribute
requires three or more. Controls are disabled for captions, audio, a locked track, a
keyed position bound refusal or an item whose transient render placement would move it
elsewhere. The multi-selection inspector shows the same row under its shared fields.

The multi-selection inspector shows audio gain and mute plus text content, font, size,
colour and background for compatible items, a dash for mixed values and the
compatible/selected count. A single element uses per-type tabs. The Text tab is one tab of
sections in order: Font (content, font), Format (one row: size with its drag grip, bold,
italic, underline, alignment; then line decoration, letter and line spacing), Style (fill
swatch, background box, outline and shadow each as a toggle with a swatch, glow switch,
saved styles, background geometry), Opacity, the shared Transform section (Scale, Position
X/Y, Rotate, Arrange, flips; the same component as video items), Blending (blend mode) and
Animation (the shared control below). Text has no hex colour field and no bare position fields: each colour is one
swatch, and text position is the Transform Position row. The Style colour rows are
`ColorTarget::Texts`, the same control for one text or many: texts that differ show a dash,
and each commit is one `set_item_param_many` edit. Video and image items keep Transform and
Blending under Basic, then Audio (see below) and Speed. Speed is a 0.01–5x scrub with a
reset and, for items with audio, a `Change pitch` checkbox that is on while pitch
follows speed (`maintain_pitch` off). A `Reverse` switch (video and audio) uses `timeline.set_reverse`, one undo step, and keeps the span. Speed edits use `timeline.set_speed`, one undo
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

## Audio tab

Audio follows the Video tab for a video with source audio, and is the default tab of an
audio clip. Its section has three slider rows with fields: Volume (`audio_gain`, -60 to
+20 dB, 0 neutral, keyframeable through the Volume diamond), then Fade in and Fade out
(`audio_fade_in` / `audio_fade_out`, 0 to 10 s, capped at the clip less the other
fade's window). Fade edits are `audio.set_fade` with the edge's current curve, one undo
step per commit; a drag is one gesture. The engine clamps a fade to the same room, so
a typed value above it lands on the cap. Every row is disabled on a locked track. Mute
is not on this tab: it is the track header and the clip menu. The engine's volume floor
is -60 dB, so the reference's -60.1 dB silent point is not reachable here until
`project::valid_volume` changes. Noise reduction and Normalize loudness rows are not
here yet: their engine work (spectral noise reduction, the loudness controls) has not landed.

## Transition inspector

A selected timeline transition replaces the clip properties with one `Transition` tab: the
catalogue name, a Duration field with stepper (0.1-5 s in 0.1 s steps, the reference's
range; Enter or blur commits, Escape cancels) and a Start / Center / End alignment
segmented control, then Remove. Duration is `transition.set_duration`, which the engine
clamps to the handles of both neighbouring clips and refuses with none. Alignment has no
command of its own: it is `transition.move` to the span position the alignment names,
batched with a `transition.set_duration` shrink when the span is longer than the handle it
must grow into, so one click is one undo step; a span moved by hand shows none selected.
Remove is `transition.remove`.

## Background sub-tab

A video clip's Background cell sits second in the Basic sub-tab bar. It is listed only while the clip leaves part of the canvas open at the playhead (`read::clip_background_visible`, which asks `render::arrange::covers_canvas`). The kind row is None, Colour and Blur. Choosing a kind commits one `timeline.set_background` undo step: Colour starts from the current solid colour or black, and Blur starts from the current intensity or 10. Colour shows the colour row with Hex and the shared solid swatch grid. Blur shows Light, Medium and Heavy (100, 200 and 500). Apply to all copies the current background to every unlocked video item as one undo step. Controls send `Action::SetClipBackground`; the workflow sends `timeline.set_background`.

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
Headers and numeric labels use the shared i18n effect lookup, as do library search
and add feedback; switching locale changes display without a project/history edit.
Scalar parameters use the scrub contract above. Static channel choices use the
catalogue's exact index order in a compact dropdown; Reverse, Solid colour and Invert
use labelled checkboxes. Their model values are read each frame; actual changes use
one effects.update_param edit (choice index or bool0/1), with no keyframe diamond.
The six E1 RGB colours use the existing colour row/picker, resolved at real item-local
time. One SetAnimatedParam command edits the authoritative linear Color base or RGB
keys at that tick, preserving static fallback, other curve data and all alpha.
RGB diamonds add/remove RGB keys together in one undo; alpha keys do not fill them.
Keyed edits/diamonds outside the clip are disabled; unkeyed bases remain editable.
Selection, time, effect, lock and panel invalidation cancel the picker preview.
Chroma key keeps its static packed-colour command. Conversion stays with render's
colour display and ecolor's existing sRGB input conversion.
Locked tracks disable these controls. Popup identity includes item/effect/parameter,
so selection changes cannot apply an old popup to a different clip.
Empty stacks, disabled effects, locked tracks, audio clips, unsupported kinds, missed drops and the engine's effect limit show
an explicit state or localized refusal. Edits go only through `effects.add`,
`effects.reorder`, `effects.toggle`, `effects.remove`, `effects.update_param` and
`keyframes.set_param`.

## Adjust basic, HSL and the footer

The Adjust tab opens with Basic, HSL, Curves and Color wheel sub-tabs. Basic lists the reference's twelve controls in measured order: saturation, temperature, hue, brightness, contrast, shine, highlight, shadow, sharpness, vignette, fade and grain. All twelve are registered `color_grade` parameters on -50..=50 with 0 neutral and scalar keyframe admission through `effects.<instance>.params.<id>`; the same ids carry the Effects tab rows. HSL lists eight hue bands (red, orange, yellow, green, cyan, blue, purple, magenta) and edits the selected band's `hsl_<band>_hue`, `_saturation` and `_lightness`; switching bands changes no project data. `effect_scrub_field` and its draft/gesture contract above apply unchanged, and every edit is one `effects.update_param` undo step.

The footer's `Apply to all` copies the selected item's complete grade (including LUT reference and curves) onto every unlocked video and image item as one `batch` command, so it is one undo step; items without a grade gain an instance. `Save as preset` stores the same parameter map as a named app preference (`adjustment_presets`, `Preset N`), never in project data, and the Adjustment assets tab lists each saved preset with an apply action for the selected clip and a delete action. Saving and deleting are preference edits with no project history entry; a preset applies through the same commands as one undo step.

The Adjust tab appends three lift/gamma/gain wheels and one tone-curve editor inside their sub-tabs. Each wheel has a triangular chroma puck, a vertical master and a reset; the plot has Red, Green, Blue and Luma channel cells and a reset. A wheel drag writes its three stored percentages through `effects.update_param` inside one engine gesture, the release is one undo step, and reset writes the three neutrals as one batch. A curve click adds one point snapped to the exact 1/4096 grid, a drag moves an interior point within its neighbours, a double-click removes an interior point and endpoints stay fixed; every edit replaces the whole validated curve through `effects.update_param`. A non-finite point, a point with no grid room between its neighbours or an endpoint edit is refused with no command. Reset restores the two-point identity. Viewing a clip with no grade shows the neutral wheels and identity curves and adds nothing; the first edit creates `color_grade` carrying that value. Wheel and plot geometry is a [new composition](../../../../../../docs/product/ui/editor.md#adjust-tabs-basic-set-hsl-wheels-and-curves) below the measured rows; ids are `effects.<instance>.wheel.<family>`, `.master`, `.reset`, `effects.<instance>.curve.<channel>`, `.plot`, `.reset`, `effects.<instance>.tab.<basic|hsl|curves|wheel>`, `effects.<instance>.hsl.<band>`, `.apply_all` and `.save_preset`.

## Reduce image noise

A video item's Basic tab ends with the `Reduce image noise` toggle row (`inspector.reduce_noise`). A click sends one `timeline.set_item_param` with field `reduce_noise` and `true` or `false`, one undo step. Layout and the unmeasured parts: [editor design](../../../../../../docs/product/ui/editor.md).

## Blend and opacity

Visual selections show a blend dropdown (`inspector.blendMode`, options
`inspector.blendMode.<persisted-id>`) and an opacity entry (`inspector.opacity`) stored 0..=1
and shown 0–100 % through the shared scrub's `scale`,
mixed values marked. Both dispatch the shared-field command: one atomic undo step,
unrelated fields kept. Opacity displays and edits the real item-local playhead value.
Keyed edits outside the half-open item range refuse; unkeyed bases remain editable.
Typed drafts and scrubs belong to their project snapshot, item and playhead; time,
selection, lock, tab or snapshot changes cancel them before blur. Removing the final
key exposes the retained base.

The Blend section sits under Basic after Transform, with its own reset arrow. Reset
restores blend mode Normal and opacity 1.0 (the engine's base) as one batch, one undo
step; it is `ResetBlend` in the workflow. Blend has no enable checkbox yet: the project
model has no blend-enabled field, so none is invented here.

## Animation

One control (`animation_controls`, `view/animation.rs`) serves the Text tab's Animation
section and the Animation tab of Video, Image, Sticker and Graphic items, so there is one
set of ids. A segmented bar `inspector.animation.tab.in|out|combo` picks the slot (the
choice is view state, In first); below it a None tile and one tile per look
(`inspector.animation.<slot>.<look>`, `.none`; twelve looks for In and Out, thirteen for
Combo) and, for an applied In or Out, a Duration field (`inspector.animation.duration`,
`.increase`, `.decrease`; Enter or blur commits, Escape cancels, 0.1 s steps). The selected
tile is the look read back from the item's keys by `engine::ClipAnimations::of`: only an
exact key match is selected, so hand-edited keys select nothing. Every click, step or commit
is one `clip.set_animation` request carrying the project snapshot it was drawn from; a
look change keeps the slot's duration. The
[engine edit contract](../../../../../../crates/engine/src/session/edit/README.md#clip-animation)
owns the looks, the clamp, the In/Out overlap rule, Combo exclusivity and the trim re-fit.
Tiles and field are disabled, and a captured request refused, on a locked track, with
an open draft, transaction, save, inline or caption edit, like the other item edits.

The Style section's Preset row lists the project's saved text styles
(`inspector.text_preset.saved.<id>`) and a Save style action. Apply copies the saved
values onto this item; Save stores its current appearance as the first free `Style N`
using `text.save_preset`. Both use the engine preset commands from
[text presets](../../../../../../crates/engine/src/session/edit/README.md#text-presets),
and the row's selection compares the complete stored style. Words and animation keys are
never saved.

Exactly one unlocked Text and no active work, transaction, save, inline/caption or
Properties draft is required. Captured row actions revalidate their target and snapshot;
direct activation uses the same readiness before any unrelated draft can flush.

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

Streak Blur, Edge Lines and Colour Relief share the54x22 precise spatial field. Its
drag uses the catalogue soft range but typed values admit the hard range and display
authored precision; out-of-range and non-finite input refuses. Units are degrees for
direction, pixels for length/relief and percent for contrast/blend. Zero-key bases
stay authoritative; keyed edits affect real-local keys. Time, selection, lock,
effect or panel changes cancel pending edits. A queued row removal cancels drafts
and scrubs before removal, so a removed effect cannot be recreated. Other controls
retain behavior.

The unselected-project Settings view has a Timeline previews section. It inspects once on first
reveal; Refresh updates the snapshot, Clear removes eligible entries, Cancel stops further
removals and Retry repeats the failed operation and resets visual recovery. Disk bytes,
retained/in-use/missing/failed counts and partial results remain visible; an unavailable root
requests restart. This is shared process waveform/scene-analysis and timeline-thumbnail
storage only, excluding import thumbnails, models, other libraries and source/project data.

## Graphic appearance

Graphic controls expose RGBA fill and stroke, width, held stroke alignment and rectangle-only corner radius under stable `inspector.graphic.*` identities. Colors key their four independent linear channels together in one command; existing component keys and handles survive. Numeric controls use ordinary scalar Params and key diamonds. Present zero-key bases remain authoritative. Keyed edits refuse outside the half-open item window. Drafts, scrubs and pickers bind to the selected item, snapshot, playhead, visible tab and lock state; stale contexts cancel before blur. Static engine patches change fallbacks without erasing curves.

The area's `input_ready` predicate admits frame clipboard actions only with no focused
field, dirty draft, scrub, picker, reorder or font dialog. Refused actions leave drafts
in this owner untouched.
