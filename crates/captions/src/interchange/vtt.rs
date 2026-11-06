//! Parses WebVTT subtitle files.

use super::{Format, Import, cue, decode, finish, read_blocks};
use crate::{Error, LanguageTag};

/// Decode WebVTT text, discard metadata and turn its timed blocks into a
/// repaired source track with entities decoded.
pub(super) fn parse(bytes: &[u8], language: LanguageTag) -> Result<Import, Error> {
    let mut warnings = Vec::new();
    let text = decode(bytes, &mut warnings)?;
    let cues = read_blocks(&text, Format::WebVtt, &mut warnings)?
        .into_iter()
        .enumerate()
        .map(|(index, block)| cue(block, index + 1, &language, true))
        .collect::<Result<Vec<_>, _>>()?;
    finish(cues, warnings)
}
