# UI tokens

[`theme.rs`](../../../apps/vibecut/src/kit/theme.rs) owns executable colour and geometry values. [`index.md`](index.md#measured-desktop-reference) owns capture provenance and region measurements. Add no second numeric table here and no inline copies in widgets.

| Contract | Rust owner | Measurement source in UI design |
|---|---|---|
| Spacing, gutters and icon/label gap | `kit::space` | Frame, feature tabs, properties |
| Panel and control corners | `kit::radius` | Layout grid; measured control corners only |
| Body/chrome type and semibold label family | `kit::type_size`, `label_font` | Type scale; sizes inferred from capital heights |
| Control boxes, rows, steppers, slider and media dimensions | `kit::size` | Player, properties, timeline and shortcuts |
| Dense glyph boxes and play/layout glyphs | `kit::icon` | Feature tabs and toolbar measurements |
| App surfaces and accents | Semantic colour functions | App palette |
| Light surfaces and non-editor status colours | Semantic colour functions | Accessible ramp |

Reference hex samples are not app tokens; colour differences from the reference are intentional. Dark and light share measured geometry. Hover, pressed, focus and disabled states reuse app semantic roles. Page heading sizes, dialog margins, uncaptured track types and panel layout calculations stay caller-owned until measured.

[`kit.rs`](../../../apps/vibecut/src/kit.rs) owns the shared kit: icon buttons, tab strips, section headers, property rows and slider/stepper drafts. Callers keep their labels, tooltips, automation IDs, preferences and preview/commit/rollback decisions.

The measured text styles, 22 px control height, spacing and 4 px corners are an editor-only density (`kit::install_editor_density`, called by the editor frame). Home, Shorts and the workspace shell keep toolkit defaults until measured.
