# Timeline

Interface: `State`, `show`, `show_scenes`, `apply`, `Action`, `Gesture`, `DropZone`.

The area owns its layout constants (toolbar, ruler, label and scrollbar sizes, zoom
bounds and padding, edge auto-scroll, ruler spacing, clip label length) in `timeline.rs`,
its snapping in `snapping.rs` and the persisted-view conversions on `State`
(`adopt_stored_view`, `stored_view`, `zoom_step`); the frame reads none of them.

`apply(state, port, actions)` applies the actions `show` returns through the frame's
`Port`: engine commands, gesture previews and publishes, scene renames, generation jobs,
lane and easing preferences, and the Replace media request. The selection scopes, the
inspector's open scrub and the gesture openers belong to the frame, so `apply` stops at
the first action that needs them and returns it with the rest; the frame applies it and
calls again. `State` holds the Replace media request until the window takes it for the
native picker; the area leaves it as `Request::ReplaceMedia`.

Each item has a 16 px chevron opening its keyframe lanes. The open set is presentation
state stored as `timeline_lanes` in the preference document, keyed by owning project so
restarting item ids cannot alias. Save As copies the issuing project's set to the copy's
fresh ProjectId; a refused Save As changes neither. Projects removed from the library are
pruned at the next preferences save.

A lane is 20 px high, one per nonempty animation channel (audio always offers volume), and
shows the channel's diamonds. Click selects, Cmd/Ctrl/Shift toggles, a lane marquee
selects, drag retimes with the clips' snapping candidates in one engine gesture, and
Delete removes selected diamonds through `keyframes.delete_selection` in one undo step.
Right-click offers before/after Hold or Linear through `keyframes.set_extrapolation`.
Audio clips draw a draggable volume line using the linear-gain squared map over -60..20
dB; drags commit on a 0.1 dB grid, field and stored edits keep 0.01 dB; volume keyframes
show on the line and in the lane.

Audio clips also carry a fade handle in each bottom corner. Dragging stages
`audio.set_fade` through the engine's replacement preview, so release publishes one undo
step and returning to the press point preserves a split fade's offset without history.
Escape, loss of window focus, fullscreen hiding the timeline or losing an editable audio
target restores the entry project. The stored window clamps to the clip and against the
other edge. The gain curve uses the project's fade-curve evaluation; right-clicking a
handle re-shapes that edge as one `audio.set_fade` command.

## Missing media

A Video or Audio clip whose asset file is gone keeps its asset record, which is the relink state. The clip view asks the display contract (`VisualQuery::availability`, fed from the engine's `MediaAccess::source_availability`; the area never probes a path) and, for a missing source, draws the field fill and the warning marker in the strip and waveform bands in place of footage. It asks the display queries and the generation commands for nothing. The bands carry the hover and accessibility name "Media missing. Replace media to relink", and the clip menu's Replace media is the relink action (it opens the picker for a file already in the project). The desktop owner holds each engine answer for two seconds per asset and path, so a file deleted while the project is open shows on a later paint, and replacing the clip's source, which names another asset, clears the state at once. A reference capture of CapCut's missing-media clip does not exist; colours and glyph reuse the failed-visual marker and are [proposed](../../../../../docs/product/ui/editor.md#gaps).

## Graph popover

The toolbar graph button (right of the flip buttons, glyph is the keyframe diamond until the
icon catalog gains a curve) is enabled while the selected keyframes include one with an
outgoing segment. It opens a popover below the button with a 200 pt plot of the segment's
normalized cubic Bezier (`x` is time, `y` the value fraction, so `y` may leave 0..1), two
draggable handles, per-component tabs when the selection spans several colour components,
a Presets/Saved tab, Save preset, Apply and Cancel. Every change (handle drag, preset or
saved click) is staged as `keyframes.set_handles` or `keyframes.apply_preset` previews
inside one engine gesture, so the committed project and history stay untouched until
Apply publishes one undo step; Cancel and Escape discard the gesture and restore the
pre-open project exactly. A handle drag edits the active component tab; a preset click
applies to every selected key. Presets and the saved list live in `preferences.easing`
(see [preferences](../../preferences/README.md)); Save preset appends the working curve as
the next free `Custom N` through the preference store, and the tab choice persists.

## Select and split modes

The pointer button at the start of the toolbar's left group shows the active tool and opens a menu
with Select (`A`) and Split mode (`B`). The mode is presentation state, starts in Select and is
never saved. In Split mode a click on an unlocked clip sends `split_at` at the frame-snapped pointer
time, one undo step; a locked clip, or a time at its edge, changes nothing. Clips take no drags in
Split mode, the pointer shows a crosshair and a guide runs from the ruler through the lanes at the
cut time. Volume lines and fade handles keep their own drags in both modes.

`[` and `]` select every unlocked item that starts before, or ends after, the playhead; an empty
result leaves the selection as it was. `Ctrl+Shift+B` runs `timeline.split_all`, the engine edit
described in [edit commands](../../../../../crates/engine/src/session/edit/README.md).

## Preview axis

The toolbar toggle (right of the ripple switch, shortcut `S`, persisted as `preview_axis`,
default on) scrubs the player from the pointer. While the pointer is over the ruler or the
lanes, `view/hover.rs` holds a frame-snapped nonnegative pointer time through empty tail
beyond the project end, paints a vertical guide there and the player footer shows that
time in place of the playhead. The frame hands the time to the playback service's
transient hover render, which fences one generation per pointer time, so only the latest
request is shown. Leaving the timeline or turning the axis off returns both the counter
and the player to the playhead. The hover time is presentation state: it never enters
Project JSON, the transport or the undo history, and clicking the ruler still moves the
playhead while the guide keeps following the pointer.

## Markers

The toolbar bookmark button and the `timeline.bookmark` shortcut toggle a bookmark at the
playhead through `bookmark.toggle`, which frame-snaps the time. Clicking a bookmark on the
ruler seeks there and opens its editor: a note field, a colour swatch that opens the shared
colour window plus a reset to the engine default, a duration field in seconds stepped by
0.1, and delete. Each committed field is one `bookmark.update` or `bookmark.remove` undo
step; drafts stay in the panel and the project changes only when the frame runs the
command. The target-free [colour service](../../services/color_picker/README.md) supplies
the controls; the marker target and commit action stay in timeline. Escape closes the
colour window, then the editor. Dragging a bookmark keeps `bookmark.move` and snapping.
Right-clicking the ruler opens its
[bookmark navigation menu](view/ruler/README.md); the lookup policy is owned by the
[action registry](../../actions/README.md#bookmark-navigation).

## Transitions

Each applied transition draws a 18 px marker on its track's clip band, spanning its span
(at least 20 px wide so it stays clickable at any zoom) with the catalogue name beside a
glyph once there is room. A click, or the accessibility action (Enter and Space are
transport chords, so there is no key-activated focus), selects it as the `Transition`
selection scope, which replaces the item selection; selecting an item, a marquee or
Escape clears it. Delete and Ripple delete remove just the transition through
`transition.remove`, one undo step. The inspector edits it
([Properties](../panels/properties/README.md#transition-inspector)).

## Reflection

The toolbar carries horizontal and vertical reflection buttons after Crop, bound to
`timeline.flip_horizontal` / `timeline.flip_vertical` with no default chord. They activate
the same registered commands as the Properties Transform toggles and keep the selection.
With several items selected the policy is all-flipped-to-false, otherwise all-to-true; a
locked, unsupported or keyed-axis target disables the button rather than editing part of
the list.

## Track controls

Header and row geometry follows [UI design](../../../../../docs/product/ui/editor.md#density),
with embedded regular SVG kind icons, output toggles and accessibility-only names. Logical
row sizes are constant across DPI. Header context menus remove unlocked secondary tracks
and rename tracks inline (Enter commits, Escape cancels). The toolbar carries the persisted
main-track magnet control. All edits are registered engine commands and refresh the shared preview
snapshot; paste comes from the clipboard owner.

Media insertion reuses an unlocked compatible track only when the playhead range fits,
otherwise creates a secondary track at the top of its group. Solid colour is generated
Video content, distinct from Sticker tracks. Lock protects item structure and properties,
masks, effects and keyframes; selection, playback and track metadata stay usable. Dragging
a header reorders within its overlay or audio section; main stays fixed; Escape cancels.
The Add track action uses the active item's compatible kind above its track, or Video at
the overlay section end with no active item. Commands and undo/persistence:
[engine](../../../../../crates/engine/src/session/edit/README.md#edit-semantics-and-limits).

## Preview proxies

`proxies.rs` queues proxy generation for the project's video sources on the media owner's runner,
one job at a time, and offers cached proxies to preview through the shared `ProxyIndex` while the
project's choice is on. The Settings tab's Proxy rows raise `Request::Proxy`; the frame applies the
toggle through `Session::set_proxy_preview` (no history entry) and re-renders the shown frame.
Cancel stops the running job and clears the queue. A cleared or deleted proxy file falls back to the
source and shows a warning until it is generated again.

## Clip visuals

Video and image clips paint the cached thumbnail strip across their visible source range;
audio clips, and the audio band inside a video clip, paint the cached waveform. The panel
passes its zoom in pixels per second to the engine display query, which maps trim and
retime and reads only the cache. Pending displays paint quiet placeholders and ask the
frame to run `media.generate_waveform`/`media.generate_clip_thumbnails`; other failed or
unsupported bands show a warning marker, and bars past 0 dBFS get a
warning tip. A cancelled regeneration publishes nothing, so the previous ready cache keeps
drawing. The workspace owns the one process `MediaAccess` and lends it here and to the MCP
inbox; `State` owns the media service (`set_media`, `generate`, `tick_media`, `request_cache`, `cache_ui`; the context lends its `visuals` to every area), and `visuals.rs` its in-flight jobs and uploaded strip textures. The
panel never decodes media or builds a cache key.

## Scene switcher

The toolbar's right group carries a scene-list button whose tooltip names the current
scene. The sheet lists all scenes and switches through `scene.switch`; Select mode allows
multi-delete but disables main; delete opens an undo-explaining confirmation. Escape
cancels the confirmation before closing the sheet. Per-scene zoom/scroll/playhead restore
through Session metadata; item, keyframe and mask-point selection clear on switching. The
plus action runs `scene.create` named `Scene N` as one undoable edit, then switches; the
sheet stays open. A row context menu opens inline rename for any scene including main
(`scene.rename`; Enter commits, Escape cancels); removal of the edited scene by a command
or undo discards the edit. New scenes inherit canvas, frame rate and background with an
empty timeline.

Typed cache corruption permits one regeneration per visual key (at most 256 remembered keys),
then stays failed until human Retry/Clear. A queued corruption reserves bounded metadata;
the attempt is spent only when generation admits a job. Gesture deferral and pending work
do not consume another attempt.
The first safe typed cause remains available to Settings. Media owns one panic-catching,
cancellable joined maintenance thread; it snapshots the shared access under `try_lock`, then
releases it before IO. Drop cancels and joins; active generation jobs continue unchanged.

Lane menus activate shared Copy/Paste actions, with component-distinct
`timeline.lane.<item>.<path>.<component>.copy|paste` IDs. The selected item's
`timeline.item.<item>.paste` menu supports existing Params with no keys. Current frame
eligibility supplies disabled states; opening another item's menu never retargets Paste.

Thumbnail display failures retain Artifact causes inside the mixed engine boundary. The
existing corrupt-cache recovery recognizes that explicit shape; waveform causes keep their
Runtime display failure; mixed query refusals compose the Analysis owner.
