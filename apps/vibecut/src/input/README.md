# Key input

Interface: `Chord`, `ChordError`, `Mods`, `Platform`, `Dispatch`, `DispatchContext`,
`BlockedReason`, `KeybindingPreferences`, `KeybindingProfile`, `Keymap`,
`KeybindingIssue`, `KeybindingError`, `MAX_ACTIONS`, `MAX_BINDINGS_PER_ACTION`,
`MAX_CHORD_TEXT`, `MAX_PROFILE_BYTES`.

The foundation under the window action tables: canonical chords, persisted binding
deltas and import profiles, the resolved `Keymap` with non-fatal `KeybindingIssue`s, and
the `Dispatch` outcome. It knows nothing of the engine or any feature: a window supplies
action ids, defaults, availability and bindability, and the registry resolves them.

Chords compare and persist by canonical text (`Ctrl+Alt+Shift+Key`); display uses
platform glyphs. Validation caps 96 actions, 8 chords per action, 16 KiB per profile and
64 characters per chord, refuses malformed ids and chords, and reports conflicts
without dropping the user's map. Startup repair removes only invalid entries.

Global dispatch runs in the capture phase, waits while an overlay or dialog is open, a
project is loading, recording is active or a text field has focus, and consumes one
exact chord once. Paste is intercepted only when the in-app clipboard has an entry;
otherwise media paste gets the event.

`text_clipboard(ctx, owner)` bridges raw native clipboard keys before widgets in each
viewport. Native shortcut translation is disabled so nontext keyframe Paste works
independently of OS clipboard content. Focused Copy/Cut become egui events; focused
Paste requests text through the existing viewport command. Dedicated keys and Windows
Insert/Delete aliases retain text semantics. Nontext keys stay with the keymap and never
request system data. Keyframe snapshots are never serialized to the system clipboard.

One pending request per viewport retains widget, owner, frame and issue time; focus,
pointer, owner changes or a two-second timeout invalidate delivery. The locked eframe
Wgpu adapter performs requests synchronously after painting and queues a result for the
next frame; the bridge classifies it before replacement widgets. An empty result completes
on that next frame too, without confusing repeated egui discard passes with native frames.
Home and both immediate child-window consumers supply their own owner identities.
