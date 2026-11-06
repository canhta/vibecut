//! Rejects transcripts outside the segmentation contract.

use crate::{Error, Transcript};
use std::collections::BTreeSet;
use time::Tick;

pub(super) fn validate_transcript(transcript: &Transcript) -> Result<(), Error> {
    let mut previous = Tick::ZERO;
    let mut ids = BTreeSet::new();
    let mut bytes = 0usize;
    for segment in &transcript.segments {
        bytes = bytes
            .checked_add(segment.id.as_str().len())
            .ok_or(Error::TooLarge)?;
        bytes = bytes
            .checked_add(segment.text.as_str().len())
            .ok_or(Error::TooLarge)?;
        for word in &segment.words {
            bytes = bytes
                .checked_add(word.text.as_str().len())
                .ok_or(Error::TooLarge)?;
        }
        if let Some(spans) = &segment.spans {
            bytes = bytes
                .checked_add(spans.byte_cost())
                .ok_or(Error::TooLarge)?;
        }
        if bytes > 16 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        if segment.id.as_str().is_empty()
            || !ids.insert(segment.id.clone())
            || segment.range.start() < previous
        {
            return Err(Error::Invalid("segment identity or order"));
        }
        previous = segment.range.end();
        let mut word_end = segment.range.start();
        for word in &segment.words {
            if let Some(range) = word.range {
                if range.start() < word_end || range.end() > segment.range.end() {
                    return Err(Error::Invalid("word range"));
                }
                word_end = range.end();
            }
        }
        if let Some(spans) = &segment.spans {
            spans.validate_segment(&segment.text, segment.range, &transcript.source_lang)?;
        }
    }
    Ok(())
}
