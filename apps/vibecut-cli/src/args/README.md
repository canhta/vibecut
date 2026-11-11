# Arguments

Native argument interpretation with stable typed refusals.

Diagnostics consume Failure codes for both direct Kernel `engine::EditError` job causes
and the mixed `engine::Error` aggregate. Ordinary Kernel-to-CLI conversion composes
through `Error::Edit`; static diagnostics and lower source chains stay with that owner.

Transcript translation results retain their lower causes. The existing failed-cause diagnostic
also accepts explicit raw or engine-composed Transcript Runtime Failed, keeping the inner
static message and provider setup hint without changing scheduling policy.

Subtitle pipeline and ASS import failures compose through the Subtitles owner. Explicit raw
or engine Subtitles causes retain Transcript/provider hints and the existing Runtime Failed
message presentation; optional setup preserves its retained source.

Artifact causes retain their direct Render/Runtime/Lut unavailable details and existing
explicit Runtime Failed message presentation. Empty snapshots keep the existing rejected
category through the artifact owner.

Analysis causes retain their explicit raw or engine-composed provider setup hint, render
unavailable details and existing Runtime Failed message presentation.
