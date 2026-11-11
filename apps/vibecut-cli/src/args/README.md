# Arguments

Native argument interpretation with stable typed refusals.

Diagnostics consume Failure codes for both direct Kernel `engine::EditError` job causes
and the mixed `engine::Error` aggregate. Ordinary Kernel-to-CLI conversion composes
through `Error::Edit`; static diagnostics and lower source chains stay with that owner.

Transcript translation results retain their lower causes. The existing failed-cause diagnostic
also accepts explicit raw or engine-composed Transcript Runtime Failed, keeping the inner
static message and provider setup hint without changing scheduling policy.
