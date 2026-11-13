# Command declarations

Interface: `ItemParamPatch`, `ITEM_PARAMS`, `ItemParam`, `item_param`, `ParamBounds`, `ParamTarget`,
`ParamUnit` and `ParamParseError`, re-exported by the session.

| Module | Holds |
|---|---|
| `item_params` | The one declaration per item parameter, and what it generates. |

An item parameter is declared once in `item_params`: its wire name, the item kinds it addresses,
its bounds and unit, the animation channel it keys on when it has one, and how it writes into an
item. The `ItemParamPatch` variant, `compatible`, `validate`, `apply`, `parse` and the read-only
`ITEM_PARAMS` table are generated from that entry, so a new parameter touches only its entry and
its render or use site.

The wire name is the only name. It is the patch `field`, the table row name and the name an
adapter parses; the table's `path` column is the animation channel a keyed parameter writes,
which differs from the wire name only for the graphic scalars (`params.strokeWidth`).

`ParamBounds::Range` is checked by `validate` and read by the inspector; `clamp` marks the
transform fields, whose typed text is pulled into range while a patch outside it is refused.
`ItemParamPatch::parse` reads the text of an inspector field or a script as the patch of a wire
name; a crop is dragged, so no text reads as one.
