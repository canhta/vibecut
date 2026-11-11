# Arguments

Native argument interpretation with stable typed refusals.

Diagnostics consume Failure codes for both direct Kernel `engine::EditError` job causes
and the mixed `engine::Error` aggregate. Ordinary Kernel-to-CLI conversion composes
through `Error::Edit`; static diagnostics and lower source chains stay with that owner.
