# UI tokens

[`theme.rs`](../../../apps/vibecut/src/kit/theme.rs) owns executable colour and geometry values. [`index.md`](index.md#measured-desktop-reference) owns capture provenance and region measurements. Add no second numeric table here and no inline copies in widgets.

| Contract | Rust owner | Measurement source in UI design |
|---|---|---|
| Spacing, gutters and icon/label gap | `kit::space` | Frame, feature tabs, properties |
| Panel and control corners | `kit::radius` | Layout grid; measured control corners only |
| Body/chrome type and semibold label family | `kit::type_size`, `label_font` | Type scale; sizes inferred from capital heights |
| Control boxes, rows, steppers, slider and media dimensions | `kit::size` | Player, properties, timeline and shortcuts |
| Inline text edit caret width and selection opacity | `kit::size::TEXT_EDIT_*` | Live web editor observation (2026-11-09) |
| Preview snap guide width, dash and gap | `kit::size::SNAP_GUIDE*` | Live web editor observation (2026-11-10) |
| Player ruler strip, tick heights, caption gap, guide line stroke and colour | `kit::size::PLAYER_RULER*`, `GUIDE_LINE*`, `kit::guide_line` | Reference desktop captures 2026-11-14 (shots 100216, 111304) |
| Agent chat panel, title strip, input box and suggestion chip sizes | `kit::size::AGENT_*` | Desktop capture 221910 and the second reference shot ([Agent chat](editor.md#agent-chat)) |
| Dense glyph boxes and play/layout glyphs | `kit::icon` | Feature tabs and toolbar measurements |
| App surfaces and accents | Semantic colour functions | App palette |
| Light surfaces and non-editor status colours | Semantic colour functions | Accessible ramp |

Reference hex samples are not app tokens; colour differences from the reference are intentional. Dark and light share measured geometry. Hover, pressed, focus and disabled states reuse app semantic roles. The 24 px dialog inset, 90×34 footer buttons and 320 px toast are kit tokens (`space::DIALOG`, `size::DIALOG_BUTTON_*`, `size::TOAST_WIDTH`), taken from the export capture and applied to every modal. Page heading sizes, uncaptured track types and panel layout calculations stay caller-owned until measured.

[`kit.rs`](../../../apps/vibecut/src/kit.rs) owns the shared kit: icon buttons, tab rows, section headers, steppers and value fields, panel states, progress bars, toasts and dialog frames. Callers keep their labels, tooltips, automation IDs, preferences and preview/commit/rollback decisions.

The measured text styles, 22 px control height, spacing and 4 px corners are an editor-only density (`kit::install_editor_density`, called by the editor frame). Home, Shorts and the workspace shell keep toolkit defaults until measured.
