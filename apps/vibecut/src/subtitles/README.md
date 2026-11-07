# Subtitles

Presents the captions panel: cue editing, translation staging and export
requests over engine's subtitle owners.

## Panel contract

The spoken-language select lists Auto plus every language the configured ASR
adapters accept. Auto is the default; a project's explicit choice is stored in
`Options::project_sources`, so opening another project starts from Auto instead
of inheriting the previous project's language. Auto runs the local language
detector first and passes the detected tag explicitly to every recognition
request.

A silent clip returns `NoSpeech`; a result whose mean recognized-word confidence
is below the engine floor returns `LowConfidence`. Both show a specific message
naming the likely wrong language, and an explicit-language run offers one click
to retry the same source with Auto. Auto itself never offers that retry.

`subtitles.split` still cuts at the playhead. In a cue field, Enter cuts at the
caret's nearest word boundary, Shift+Enter inserts a line break, and Backspace at
the start or Delete at the end merges the selected cue with its previous or next
neighbour. Unspaced scripts cut on grapheme boundaries; separated scripts cut
only between words. When the cue carries native ASR word timings, the source
language's cut time is the exact end of the word before the caret; otherwise the
span divides at the caret's text fraction (and the playhead split snaps to the
nearest native word edge). Empty or zero-length results are refused. Split and
both merges each commit one engine command, so one undo step restores them.

Import accepts SRT/VTT. Export offers SRT, VTT, TXT and burned MP4; TXT is the
caption-only download (one cue per line, no timings) and is serialized by
captions like the other subtitle formats.
