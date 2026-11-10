# Captions panel

Interface: `show`, `queue_requests`.

The Captions tab mounts the [subtitles](../../subtitles/README.md) panel inside the
assets area. Opening a picker constructs no adapter and downloads no model. Settings
persist in preferences; approval, transcripts and the session glossary do not. Requests
the panel cannot apply itself are queued and drained by the frame after the tab
returns.

Cue times seek in output coordinates. Shift extends a range from a cue identity (after
a merge the surviving selection is the anchor); Cmd/Ctrl toggles. Text edits keep
independent language edit and provenance flags and normalize through captions. Drafts
commit on leaving a field, switching tabs, and on cue, page, item, language and timing
actions; unrelated project edits keep them, and a concurrent change to the same
caption keeps a copyable draft until explicit reload. Focused text never triggers
timeline shortcuts. All edits are registered engine commands in the shared history.

Cue, import, text and glossary limits belong to [captions](../../../../../crates/captions/README.md).
Design: [captions workflow](../../../../../docs/product/ui/editor.md#captions-workflow).
