# Key input

`src/input.rs` is the foundation under the window action tables: canonical chords (`Chord`, `Mods`, `Platform`), persisted binding deltas and import profiles (`KeybindingPreferences`, `KeybindingProfile`), the resolved `Keymap` with non-fatal `KeybindingIssue`s, and the `Dispatch` outcome. It knows nothing of the engine or any feature: a window supplies action ids, defaults, availability and bindability, and the registry resolves them.

Chords compare and persist by canonical text (`Ctrl+Alt+Shift+Key`); display uses platform glyphs. Validation caps action count, chords per action, profile bytes and chord text, refuses malformed ids and chords, and reports conflicts without dropping the user's map. Startup repair removes only invalid entries.

Global dispatch runs in the capture phase, waits while an overlay or dialog is open, a project is loading, recording is active or a text field has focus, and consumes one exact chord once. Paste is intercepted only when the in-app clipboard has an entry; otherwise media paste gets the event.
