//! Parses SRT subtitle files.

use super::{Format, Import, cue, decode, finish, read_blocks};
use crate::{Error, LanguageTag};

/// Decode SRT text and turn its timed blocks into a repaired source track.
pub(super) fn parse(bytes: &[u8], language: LanguageTag) -> Result<Import, Error> {
    let mut warnings = Vec::new();
    let text = decode(bytes, &mut warnings)?;
    let cues = read_blocks(&text, Format::Srt, &mut warnings)?
        .into_iter()
        .enumerate()
        .map(|(index, block)| cue(block, index + 1, &language, false))
        .collect::<Result<Vec<_>, _>>()?;
    finish(cues, warnings)
}
