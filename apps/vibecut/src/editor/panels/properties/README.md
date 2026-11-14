# Properties

The Adjust area exposes imported LUT selection, Import, static Strength and a Locate action for the typed missing-resource state. This is an adopted new composition using the existing 39 px row, 22 px field and 12 px type kit, without a reference-parity claim. The workflow lends the shared process media access to one bounded runner and captures Session/revision/snapshot/item before the async picker. Selection, project/revision, lock, tab and transaction changes invalidate before polling or applying a result; cancellation/disposal uses the runner's existing join. File completion follows the real workspace picker seam; dismissing that picker releases the capture and enables Import without editing the project. Import/register/application is one undo step; selection and strength use ordinary edits, and strength remains static.

Interface: `State`, `Action`, `Widget`, `show`, `apply`, `element_kind`, `ElementKind`,
`PropertiesTab`, `SharedField`, `shared_fields`, `ColorTarget`, `font_picker`,
`font_picker::State`.

The area owns its drafts, scrub gestures, picker and popovers, its font catalog
backend, the effect reorder draft and its gesture contracts, and returns `Action`
values the frame routes. A workflow reads the session through the port's read view and
writes through the port; every project change goes through an engine command and
the area never writes project JSON. Colour targets and their draft-to-command mapping
belong here; the target-free controls come from the shared
[colour service](../../../services/color_picker/README.md). A colour commit validates
the complete preference candidate before ending coalescing or publishing the project
edit and adopts it only after that edit succeeds, without a preference undo step.


Every animatable row (Transform scale, position and rotation, Blend opacity, Audio
volume, graphic and effect parameters, including colour rows) ends in the kit's
`< diamond >` cluster at its right edge; value controls stop short of it. The diamond
(`inspector.keyframe.<field>`) adds or removes a key at the playhead through
`keyframes.toggle_at`; it is disabled when the playhead is outside the item and filled
when a key exists. The arrows (`.previous`, `.next`) seek the playhead to the nearest
key before or after it, over every channel the diamond keys together (a colour spans its
four), and are disabled when that side has none. They stay live outside the item and
on a read-only keyed slider. A click queues `Action::SeekToKey`, which the workflow
sends through the shared `Request::Seek` transport request; no project edit, no undo step.

A selected filter clip (an effect item whose stack holds the `filter` instance) shows one Filter section in its Effects tab: the look's name (`inspector.filter.look`) and an Intensity slider with its field and keyframe cluster. Intensity is the clip's opacity channel (0 to 100 percent, default 100), so it uses the opacity row's widget ids (`inspector.opacity`, `.slider`, `.scrub`), the ordinary `timeline.set_item_param` path and one undo step per typed value or scrub gesture. `effect_refusal` refuses every user-added effect on a filter clip with the filter-clip message; effects go on an adjustment layer.

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
italic, underline, alignment; then the Case row of three buttons TT, tt, Tt, where pressing the active one returns to as typed; then line decoration, letter and line spacing), Style (fill
swatch, background box, outline and shadow each as a toggle with a swatch, glow switch,
saved styles, background geometry), Opacity, the shared Transform section (Scale, Position
X/Y, Rotate, Arrange, flips; the same component as video items), Blending (blend mode) and
Animation (the shared control below). Text has no hex colour field and no bare position fields: each colour is one
swatch, and text position is the Transform Position row. The Style colour rows are
`ColorTarget::Texts`, the same control for one text or many: texts that differ show a dash,
and each commit is one `set_item_param_many` edit. Video and image items keep Transform and
Blending under Basic, then Audio (see below) and Speed. Speed is a 0.1–100x scrub with a
reset and, for items with audio, a `Change audio pitch` switch that is on while pitch
follows speed (`maintain_pitch` off). A `Reverse` switch (video and audio) uses `timeline.set_reverse`, one undo step, and keeps the span. Speed edits use `timeline.set_speed`, one undo
step per commit; trim bounds extend by the retimed output length. A `Standard` / `Curve` bar
tops the tab: Curve shows eight preset tiles (None, Custom, Montage, Hero, Bullet, Jump cut,
Flash in, Flash out; `timeline.apply_speed_preset`, None is a constant 1x) and, for a curved
item, the speed plot (`view/playback/curve.rs`): a log 0.1x-10x axis with draggable points, the
Duration line, Reset and an add/remove point button. A drag commits the whole shape through
`timeline.set_speed_curve` on release, one undo step; `Standard` on a curved item resets to a
constant 1x. Caption generation
refuses a retimed source item. The text button inserts a five-second title at the
playhead in one undo step.

Each scrub field has a left drag handle, a click-to-type entry and clamping. A drag
previews through an engine gesture and commits one `timeline.set_item_param` undo step
on release; Escape cancels the gesture or draft. Changing selection or hiding the
field cancels the preview. Invalid or non-finite input refuses without change. Drag
sensitivity: opacity 0.01 per 4 px, audio gain 0.01 dB per 4 px, text size 1 px per
2 px, speed 0.01 per 2 px. Enter or blur commits a typed shared-field patch through the
engine registry in one undo step. Focused text editing owns keyboard input.

## Text to speech tab

A text item (not a caption item) has a third tab, `inspector.tab.text_to_speech`, after Text and before Effects. It narrates the item's words: the text box starts from the item's content and the cue is the item's duration, so a take is generated for exactly the time the text occupies. The tab is [proposed]: the reference shows Text to speech only as a Home card, so no row, width or spacing is measured and all of them reuse the 39 px property rows, 22 px fields and 12 px type. Voice holds four dropdowns, Provider, Model, Voice and Language (`inspector.narration.provider`, `.model`, `.voice`, `.language`, each option `<id>.<index>`); nothing is preselected for the person beyond the first listed entry, and changing the provider resets model and voice. The lists come from [`engine::dubbing::speech_options`](../../../../../../crates/engine/src/dubbing/README.md#credentials): OpenAI with its 13 built-in voices and ElevenLabs with its premade voices only, never a cloned voice.

Script holds the text box, a counter (characters and cue seconds) and Generate. Generate submits `dub.generate` for the item's text and time, the command an agent or the CLI sends, so there is one path, one job, one progress and one cancel. An unsupported language or voice, an unusable cue or a missing key is refused by the command and shows its typed line (`inspector.narration.failure`) before any sheet. Otherwise the job asks the run's `TaskEvents` consent prompt, and the exact-upload sheet (`narration.consent.send`, `.deny`, `.unknown_price`, `.policy`) shows provider, model, voice, language, the exact text with its character count, the provider speed when set, the cost bound or unknown price (which needs its own checkbox), the provider's policy link, retention, training and the AI-generated-voice statement. Nothing is sent before Send; Don't send, Escape or a click outside refuses with no project change. While the job runs the tab shows a generating line and Cancel (`inspector.narration.cancel`, which sends `dub.cancel` and releases the sheet); a job failure reads from its wire code, so Quota, Network, a missing key, a decline and a cancel are told apart and every other provider failure reads as the generic one; the key shortcuts are held while the sheet is open.

A finished take shows its length against the cue and the fit decision from `engine::dubbing::fit`, computed with the silent time after the cue read from the project: Exact, Stretch (shown as `x`), Gap, Provider speed (a Generate-again button at the shown speed, then the sheet again with the speed in it), Rephrase (the tab asks for a text of at most N characters; the next Generate counts as the rephrase), Overflow (a Place and overflow button) or a refusal. Place (`inspector.narration.place`) is `dub.place` at the item's start: asset, item, stretch and ducking commit as one undo step, and the narration item is not selected, so the tab stays on its text. A take that needs regeneration or cannot fit has no Place button; Discard (`inspector.narration.discard`) drops it. A text item that changed while the take was made refuses as stale.

Ducking (`inspector.narration.ducking.enabled`, `.depth`, `.attack`, `.release`) edits the pending take before placement and the placed narration item after it (`dub.set_ducking`, one undo step per released field). Behaviour: [properties jobs](workflow/jobs/README.md).

## Audio tab

Audio follows the Video tab for a video with source audio, and is the default tab of an
audio clip. Its section has three slider rows with fields: Volume (`volume`, -60 to
+20 dB, 0 neutral, keyframeable through the Volume diamond), then Fade in and Fade out
(`audio_fade_in` / `audio_fade_out`, 0 to 10 s, capped at the clip less the other
fade's window). Fade edits are `audio.set_fade` with the edge's current curve, one undo
step per commit; a drag is one gesture. The engine clamps a fade to the same room, so
a typed value above it lands on the cap. Every row is disabled on a locked track. Mute
is not on this tab: it is the track header and the clip menu. The engine's volume floor
is -60 dB and that left end is true silence: the field shows `-∞dB` there (the spec's `floor_label`; typed `-inf`, `-∞` or any value below
parses to the floor) and a stored floor mixes silent (`project::volume_gain` is exactly 0 at the floor). The clip panel has no ducking row (ducking stays with narration). A Noise reduction switch follows the fades (`inspector.noise_reduction.toggle`, the
39 px row with the shared toggle). The capture shows only the switch, so there is no strength control: on adds one
`noise_reduction` cleanup effect at `project::NOISE_REDUCTION_DEFAULT` (12 dB reduction, 6 dB threshold, 50 %
smoothing, full mix) or enables the item's existing one; off disables it and keeps its values. Each click is one
`audio.add_cleanup_effect` or `audio.update_cleanup_effect` undo step, and the switch reads on only while the item's
first noise reduction effect is enabled. A chain already holding eight effects refuses the add through the error line.
Equaliser bands have no control here: the capture shows no equaliser, so `audio.*_cleanup_effect` is their only
surface. A Voice enhancement switch ([new], no reference evidence) follows it (`inspector.voice_enhance.toggle`): on adds the
`project::voice_enhancement_effects` preset (80 Hz high-pass, de-reverb, noise reduction, de-esser, two-band equaliser) with the
next free effect ids after the chain's highest, the way every new effect gets one, or enables the marked effects already present;
off disables them and keeps tuned values. The preset's effects carry `CleanupEffect::preset` = `voice_enhancement`, the only marker:
the switch reads on only while five marked effects are present and all enabled, and the Noise reduction switch ignores marked
effects. New effects start at the reference's Intensity default of 75 percent (`project::VOICE_ENHANCEMENT_INTENSITY_DEFAULT`). While on, the reference's continuous Intensity slider (`inspector.voice_enhance.intensity` field and stepper, `.slider` rail, 0 to 100 with a tick mark every 25) sets every marked effect's dry/wet mix
to that percent. A rail drag previews on the rail and commits once on release; a click, step or typed value commits at once. Each commit is one batched edit, so one undo step. The preset adds no gain stage: loudness stays with the
project normalizer, which measures the chain's output, so bypassing the preset never changes what the normalizer applied. The
preset's individual bands and cutoffs have no control (no reference evidence); they are reachable through
`audio.*_cleanup_effect`.

Normalize loudness sits below the fades and Noise reduction. Clip and Master are two switches: on sends `audio.set_loudness` for that scope with the shown target, off sends `audio.clear_loudness`; each is one undo step. Target and True peak are draft fields committed on Enter or blur (Escape cancels). A commit is one `audio.set_loudness` batch over the switched-on scopes, so one undo step; an out-of-range draft sends nothing and the status line says so. Apply sends `audio.measure_loudness` through the session control path onto the panel's job runner: the frame polls it, the progress bar and Cancel (`runner.cancel_job`) show while it runs, and a completed job's gains are stored with `audio.apply_loudness`. A failure names its cause from the job's engine error code (a missing or changed source, a decode failure, cancelled, or the generic line for any other code), and a project change during the job stops it with a notice. The status line then shows the result per scope, or the silent (no gain) state. The rows are project-wide, shown on any selected audio or video item, and disabled on a locked track or while measuring. Layout: [editor design](../../../../../../docs/product/ui/editor.md#audio-tab).

## Transition inspector

A selected timeline transition replaces the clip properties with one `Transition` tab: the
catalogue name, a Duration field with stepper (0.1-5 s in 0.1 s steps, the reference's
range; Enter or blur commits, Escape cancels) and a Start / Center / End alignment
segmented control, then Remove. Duration is `transition.set_duration`, which the engine
clamps to the handles of both neighbouring clips and refuses with none. Alignment has no
command of its own: it is `transition.move` to the span position the alignment names,
batched with a `transition.set_duration` shrink when the span is longer than the handle it
must grow into, so one click is one undo step; a span moved by hand shows none selected.
`Apply to all` (under the alignment control, above Remove; its placement is not measured in
the capture) sends `transition.apply_to_all` with the selected transition: every other
transition takes its kind and duration as one undo step, clamped to its own handles, and
drops overrides when its kind changes. Transitions on locked tracks are left unchanged and a
toast reports it; the engine's `skipped` list carries the ids. Remove is `transition.remove`.

## Background sub-tab

A video clip's Background cell sits second in the Basic sub-tab bar. It is listed only while the clip leaves part of the canvas open at the playhead (`read::clip_background_visible`, which asks `render::arrange::covers_canvas`). The kind row is None, Colour and Blur. Choosing a kind commits one `timeline.set_background` undo step: Colour starts from the solid colour the Background tab last showed this session, or black, and Blur starts from the intensity it last showed, or 10. That memory is view state (`BackgroundMemory` on the properties state), never project data. Colour shows the colour row, whose swatch opens the shared colour popover. Blur shows Light, Medium and Heavy (100, 200 and 500). Apply to all copies the current background to every unlocked video item as one undo step. Controls send `Action::SetClipBackground`; the workflow sends `timeline.set_background`.

## Remove BG sub-tab

Video and image items get a `Remove BG` cell in the Basic sub-tab bar. It opens with the
`Auto removal` toggle row (`inspector.auto_removal`) above Chroma key. Switching it on sends one
`timeline.set_item_param` with field `background_removal` and `on` (feather 0), then
`footage.analyze {kind: person_matte}` for the clip; switching off sends `off`. While on, a
`Feather` slider row (`inspector.background_removal_feather`, field
`background_removal_feather`, 0–100, one undo step per gesture, no new analysis) and the
analysis status row under `inspector.auto_removal` follow: the same states as Stabilize's, plus
`Person data missing` with a `Download` button (`inspector.auto_removal.models`) that opens the
model manager on `pphumanseg-2023mar` and `Analyze` to retry. A clip without a stored matte
renders unmatted and export refuses to start naming it. Under the status row a `Custom removal` section holds the Brush and Eraser tool toggles
(`inspector.matte_brush.add`, `.remove`) and a Size row (`inspector.matte_brush.size` field,
`.size.slider`, `.size.increase`, `.size.decrease`). A tool toggle sends `SetMatteBrush`, which
becomes `Request::ArmMatteBrush`; clicking the armed tool sends `None`. `SetMatteBrushSize`
becomes `Request::SetMatteBrushSize`, which the frame clamps to 1 to 100. The armed clip and mode
and the Size are view state in the player (`inspect().matte_brush`, `inspect().matte_brush_size`),
never project data. Each finished drag reaches `paint_matte_stroke`, which sets the item's
`matte_strokes` list plus the new stroke as one `timeline.set_item_field` command, so one stroke
is one undo step; there is no Apply step because strokes render live. The values and the
Size-to-radius mapping are in
[Remove background](../../../../../../docs/product/ui/editor.md#remove-background). Switching
`Chroma key` on only stages the panel in app state and shows the eyedropper and a
disabled reset. Switching
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

An adjustment layer (an effect item on an Effect track, inserted from the Adjustment tab) takes a stack
like a visual clip: `effect_refusal` admits it alongside video, image, sticker and graphic items. Effect
items on any other track keep the unsupported-kind refusal.

## Adjust basic, HSL and the footer

The Adjust tab opens with Basic, HSL, Curves and Color wheel sub-tabs. Basic lists the reference's twelve controls in measured order: saturation, temperature, hue, brightness, contrast, shine, highlight, shadow, sharpness, vignette, fade and grain. All twelve are registered `color_grade` parameters on -50..=50 with 0 neutral and scalar keyframe admission through `effects.<instance>.params.<id>`; the same ids carry the Effects tab rows. HSL lists eight hue bands (red, orange, yellow, green, cyan, blue, purple, magenta) and edits the selected band's `hsl_<band>_hue`, `_saturation` and `_lightness`; switching bands changes no project data. `effect_scrub_field` and its draft/gesture contract above apply unchanged, and every edit is one `effects.update_param` undo step.

The footer's `Apply to all` copies the selected item's complete grade (including LUT reference and curves) onto every unlocked video and image item as one `batch` command, so it is one undo step; items without a grade gain an instance. `Save as preset` stores the same parameter map as a named app preference (`adjustment_presets`, `Preset N`), never in project data, and the Adjustment assets tab lists each saved preset with an apply action for the selected clip and a delete action. Saving and deleting are preference edits with no project history entry; a preset applies through the same commands as one undo step.

The Adjust tab appends three lift/gamma/gain wheels and one tone-curve editor inside their sub-tabs. Each wheel has a triangular chroma puck, a vertical master and a reset; the plot has Red, Green, Blue and Luma channel cells and a reset. A wheel drag writes its three stored percentages through `effects.update_param` inside one engine gesture, the release is one undo step, and reset writes the three neutrals as one batch. A curve click adds one point snapped to the exact 1/4096 grid, a drag moves an interior point within its neighbours, a double-click removes an interior point and endpoints stay fixed; every edit replaces the whole validated curve through `effects.update_param`. A non-finite point, a point with no grid room between its neighbours or an endpoint edit is refused with no command. Reset restores the two-point identity. Viewing a clip with no grade shows the neutral wheels and identity curves and adds nothing; the first edit creates `color_grade` carrying that value. Wheel and plot geometry is a [new composition](../../../../../../docs/product/ui/editor.md#adjust-tabs-basic-set-hsl-wheels-and-curves) below the measured rows; ids are `effects.<instance>.wheel.<family>`, `.master`, `.reset`, `effects.<instance>.curve.<channel>`, `.plot`, `.reset`, `effects.<instance>.tab.<basic|hsl|curves|wheel>`, `effects.<instance>.hsl.<band>`, `.apply_all` and `.save_preset`.

## Reduce image noise

A video item's Basic tab ends with the `Reduce image noise` toggle row (`inspector.reduce_noise`). A click sends one `timeline.set_item_param` with field `reduce_noise` and `true` or `false`, one undo step. Layout and the unmeasured parts: [editor design](../../../../../../docs/product/ui/editor.md).

## Stabilize

A video item's Basic tab shows the `Stabilize` toggle row (`inspector.stabilize`) under `Reduce image noise`. Switching on sends one `timeline.set_item_param` with field `stabilize` and `normal`, then `footage.analyze` for the clip; switching off sends `off`. While on, a `Level` row (`inspector.stabilize.level.<flexible|normal|stable>`) sets the level through the same field, one undo step each, and needs no analysis. A status row follows: nothing when the stored correction is present; `Needs analysis` with an `Analyze` button (`inspector.stabilize.analyze`) when the clip's source has none; `Analyzing` with a progress bar (`inspector.stabilize.progress`) and a `Cancel` button (`inspector.stabilize.cancel`) while the job runs. The row's state comes from `State::footage_status(item, kind)`, which [workflow/footage.rs](workflow/README.md) refreshes once per frame from the process media cache only when the project snapshot or the cache epoch changed. A clip without a correction renders uncorrected and export refuses to start naming it; a correction that finishes resyncs the preview, which reads the same cache. Layout and the unmeasured parts: [editor design](../../../../../../docs/product/ui/editor.md).

## Remove flicker

A video item's Basic tab shows the `Remove flicker` toggle row (`inspector.remove_flicker`) under
the Stabilize rows. A click sends one `timeline.set_item_param` with field `remove_flicker` and
`true` or `false`, one undo step; switching on also sends `footage.analyze {kind: flicker}` for the
clip. There is no level or strength row: the reference shows none. While on, the shared analysis
status row (`inspector.remove_flicker.analyze`, `.progress`, `.cancel`) shows `Needs analysis` with
an `Analyze` button, or progress with `Cancel`, and nothing once the track is stored. The analysis
needs no model. Layout: [editor design](../../../../../../docs/product/ui/editor.md).

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
step; it is `ResetBlend` in the workflow. The header's enable checkbox
(`inspector.section.blending.enable`) has no field of its own: it reads checked while the
blend mode is not Normal, so the project's blend mode stays the one truth. Unchecking
sets blend mode Normal; checking restores the mode it replaced (kept in the area's
`blend_restore` view memory, per item) or, with none remembered, the first non-Normal
mode. Each click is one `blendMode` edit, one undo step. Opacity is untouched by it.

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
Each look tile shows its preview (`view/animation/previews.rs`): a strip the engine's
[animation-preview job](../../../../../../crates/engine/src/artifacts/README.md#animation-previews)
renders once per slot and look and the panel keeps as one texture, so a draw only picks a
frame; the name sits beneath the 58 px tile and a hovered tile loops the strip. The None
tile is empty. Tiles and field are disabled, and a captured request refused, on a locked track, with
an open draft, transaction, save, inline or caption edit, like the other item edits.

The Style section's Preset row lists the project's saved text styles
(`inspector.text_preset.saved.<id>`) and a Save style action. Apply copies the saved
values onto this item; Save stores its current appearance as the first free `Style N`
using `text.save_preset`. Both use the engine preset commands from
[text presets](../../../../../../crates/engine/src/session/edit/README.md#text-presets),
and the row's selection compares the complete stored style. Words and animation keys are
never saved.

Below the box rows, a stroke on shows Thickness (`inspector.text_stroke_thickness`, 0-100) and a
shadow on shows Opacity (`inspector.text_shadow_opacity`), Bluriness
(`inspector.text_shadow_blurriness`), Distance (`inspector.text_shadow_distance`) and Angle
(`inspector.text_shadow_angle`, -180 to 180), the reference's controls and defaults.
These fields are not engine parameter rows. `write_field` sends them as `Action::SetField`, and
`commands::text_effect_patch` builds one `TextStroke` or `TextShadow` patch from the item's
current value with only that field replaced, so the colour and the other geometry survive.

A caption item selected alone shows a collapsed Keywords section in its Text tab
(`inspector.keywords`). It draws the read `Context.keywords` carries from the Captions panel for
the caption cue under the playhead, and says `No caption under the playhead.` when none is there;
its chips, Choose, Preview, Clear, Suggest and Apply emit `Action::Keywords`, and the frame routes
them to that panel with the playhead (one undo step each, Apply as one batch). A chosen chip uses the sub-navigation's selected style (accent label on
the hover fill, as the Media Timeline scene row), and the section spaces its chips and actions
at the kit density gap (8 px across, 4 px between wrapped rows). Behaviour: [captions owner](../assets/captions/README.md).

Exactly one unlocked Text and no active work, transaction, save, inline/caption or
Properties draft is required. Captured row actions revalidate their target and snapshot;
direct activation uses the same readiness before any unrelated draft can flush.

Audio and video source items share the batch inspector's volume and mute controls;
volume steps 0.01 dB over -60..20 dB. `inspector.volume` and
`inspector.muted` are stable automation IDs.

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

A single selected Video or Audio clip whose source file is gone has no properties to edit: the panel shows the reference's one grey line centred under the header and, ours, a `Link media` button that asks the frame to open the [Link media dialog](../../dialogs/link_media/README.md) (`Action::OpenLinkMedia`, `inspector.link_media`). Measurements: [Missing media](../../../../../../docs/product/ui/editor.md#missing-media).
