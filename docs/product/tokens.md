# UI tokens

[`theme.rs`](../../apps/vibecut/src/kit/theme.rs) owns executable colour and geometry
values. [`ui.md`](ui.md#measured-desktop-reference) owns capture provenance and
region measurements. Do not add a second numeric token table here or inline
copies in widgets.

| Contract | Rust owner | Measurement source in UI design |
|---|---|---|
| Spacing, gutters and icon/label gap | `kit::space` | Frame, feature tabs, properties |
| Panel and control corners | `kit::radius` | Layout grid; measured control corners only |
| Body/chrome type and semibold label family | `kit::type_size`, `label_font` | Type scale; sizes inferred from capital heights |
| Control boxes, rows, steppers, slider and media dimensions | `kit::size` | Player, properties, timeline and shortcuts |
| Dense glyph boxes and play/layout glyphs | `kit::icon` | Feature tabs and toolbar measurements |
| App surfaces and accents | Semantic colour functions | Existing app palette |
| Light surfaces and non-editor status colours | Semantic colour functions | Existing proposed accessible ramp |

The app palette remains unchanged; reference hex samples are not app tokens.
Dark and light use the existing app colours and share measured geometry. Hover,
pressed, focus and disabled presentation reuse app semantic roles. Reference
colour differences are intentional, not parity failures. Page heading sizes,
dialog margins, uncaptured track types and legacy panel layout calculations
remain caller-owned until their area tickets measure and replace them; they are
not admitted as measured foundation tokens.

[`kit.rs`](../../apps/vibecut/src/kit.rs) owns the kit shared by
later areas: icon buttons, tab strips, section headers, property rows and
slider/stepper drafts. Callers retain existing labels/tooltips, automation IDs,
preferences and preview/commit/rollback decisions. There is no standalone demo.
