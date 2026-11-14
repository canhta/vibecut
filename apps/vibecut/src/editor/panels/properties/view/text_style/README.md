# Text item sections

Interface: none; each module adds `Panel` methods the view calls.

| Module | Holds |
|---|---|
| `keywords` | A caption item's Keywords section in its Text tab. |
| `narration` | A text item's Text to speech tab and the exact-upload sheet its generation asks. |

The section draws the read `Context.keywords` carries and emits `KeywordAction`s; behaviour
belongs to the [captions owner](../../../assets/captions/README.md).
