# Compound clip is a clip whose source is a nested timeline

Status: Accepted 2026-11-14. The current contract lives in [project compound clips](../../crates/project/src/model/README.md#compound-clips), [render compound clips](../../crates/render/src/scene/README.md#compound-clips) and [clip edits](../../crates/engine/src/session/edit/clips/README.md#compound-clips).

A compound clip is an item kind, `ItemParams::Compound`, that owns a nested `Timeline` of its members and behaves as one ordinary clip whose picture and sound come from that timeline. The item keeps the clip fields every media clip has (source window, retime, effects, masks, animations, blend) and `CompoundParams::clip` reuses `VideoParams` for its volume, mute, fades, cleanup and crop. The renderer evaluates the nested timeline at the clip's source time into an isolated transparent raster and draws it as a video clip's decoded frame; the mixer mixes the nested timeline at the clip's source samples and treats the result as the clip's source audio.

The reason is the reference: a compound clip (subproject) is trimmed, split, moved, sped up, faded, transformed, masked and given effects like any clip, and nothing inside it (adjustment layers, blends) reaches the tracks below. Treating the nested timeline as a source gives every clip edit for free through the existing clip paths, and the isolated raster gives the containment. Member, nested track and transition ids stay unique across the project so analysis keyed by item (stabilization, mattes) and undo compound keep every identity; a copy or a split piece renumbers what it nests.

Undo compound clip puts the members back with their own timing. When the clip has edits of its own (a trimmed window, a speed, a look, changed sound settings) the release would lose them, so it is refused with a typed reason (`CompoundHasOwnEdits`) rather than baked into every member: a speed or an effect has no faithful per-member equivalent once members overlap in time or sit on several tracks.

A group composes by closure: compounding a grouped item takes its whole group, the members inside carry no group, and the compound clip itself joins none.

Consequences: each compound clip in a frame costs one extra output-sized raster and one nested mix per block, bounded by `MAX_COMPOUND_DEPTH`; settings that read per-source analysis (stabilize, background removal, flicker, noise reduction) stay off on a compound clip; the nested timeline is not yet editable in place.

Rejected: releasing compound clips into their members at the render boundary (no clip edits on the compound, and adjustment layers and blends leaked onto the tracks below); a compound as a reference to a secondary scene (scenes are user-visible and exportable, copies would share one scene, and deleting the clip would orphan it); a compound as a collapsed group with members still on the outer tracks (they would block the time they occupy on other tracks).
