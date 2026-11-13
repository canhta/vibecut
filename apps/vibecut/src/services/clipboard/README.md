# Clipboard reads

Read-only access to the OS clipboard. Each reader is injected, so tests never touch the
system clipboard, and an unavailable clipboard reads as empty rather than failing.

| Module | Contract |
|---|---|
| `text` | `TextClipboard` answers focused text reads under viewport request custody, which belongs to [input](../../input/README.md). Copy and Cut output stays with egui. |
| `file` | `FileClipboard` returns the file list copied in the OS file manager; the editor's paste imports it and inserts at the playhead ahead of the in-app clipboard. |
