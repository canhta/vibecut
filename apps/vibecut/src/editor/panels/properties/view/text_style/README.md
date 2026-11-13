# Text style sections

Interface: none; each module adds `Panel` methods the view calls.

| Module | Holds |
|---|---|
| `keywords` | A caption item's Keywords section in its Text tab. |

The section draws the read `Context.keywords` carries and emits `KeywordAction`s; behaviour
belongs to the [captions owner](../../../assets/captions/README.md).
