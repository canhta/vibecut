//! Computes line ranges from shaped advances and Unicode break opportunities.

use super::LineBreak;
use super::paragraph::ShapedGlyph;
use std::ops::Range;
use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;

/// Prefix sums of shaped advances by byte offset.
pub(super) fn advance_prefix(text: &str, glyphs: &[ShapedGlyph], base: usize) -> Vec<f32> {
    let mut advances = vec![0.0f32; text.len().saturating_add(1)];
    for glyph in glyphs {
        if let Some(local) = glyph.cluster.checked_sub(base)
            && let Some(slot) = advances.get_mut(local)
        {
            *slot += glyph.advance;
        }
    }
    let mut prefix = vec![0.0f32; text.len().saturating_add(1)];
    let mut total = 0.0f32;
    for (index, advance) in advances.iter().enumerate() {
        total += advance;
        if let Some(slot) = prefix.get_mut(index + 1) {
            *slot = total;
        }
    }
    prefix
}

pub(super) fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    boundaries.push(text.len());
    boundaries
}

pub(super) fn range_width(prefix: &[f32], start: usize, end: usize) -> f32 {
    let from = prefix.get(start).copied().unwrap_or(0.0);
    let to = prefix.get(end).copied().unwrap_or(from);
    (to - from).max(0.0)
}

/// End of `start..end` without trailing whitespace.
pub(super) fn trim_end(text: &str, start: usize, end: usize) -> usize {
    let mut trimmed = end;
    for (offset, c) in text.get(start..end).unwrap_or("").char_indices().rev() {
        if !c.is_whitespace() {
            break;
        }
        trimmed = start + offset;
    }
    trimmed
}

/// Line ranges with monotonic cursors. `limit` bounds the work as well as the output: once the
/// requested number of lines exists, the remaining text only sets the overflow flag.
pub(super) fn line_ranges(
    text: &str,
    max_width: f32,
    advances: &[f32],
    boundaries: &[usize],
    mode: LineBreak,
    limit: Option<usize>,
) -> (Vec<Range<usize>>, bool) {
    if text.is_empty() {
        return (std::iter::once(0..0).collect(), false);
    }
    let mut breaks: Vec<usize> = match mode {
        LineBreak::Unicode => linebreaks(text)
            .map(|(offset, _)| offset)
            .filter(|offset| boundaries.binary_search(offset).is_ok())
            .collect(),
        LineBreak::Grapheme => boundaries.to_vec(),
    };
    breaks.sort_unstable();
    breaks.dedup();
    if breaks.last().copied() != Some(text.len()) {
        breaks.push(text.len());
    }

    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut break_cursor = 0usize;
    let mut boundary_cursor = 0usize;
    while start < text.len() {
        if limit.is_some_and(|limit| ranges.len() >= limit) {
            return (ranges, true);
        }
        while breaks
            .get(break_cursor)
            .is_some_and(|candidate| *candidate <= start)
        {
            break_cursor += 1;
        }
        let mut best = None;
        let mut probe = break_cursor;
        while let Some(&candidate) = breaks.get(probe) {
            if candidate <= start {
                probe += 1;
                continue;
            }
            let full = range_width(advances, start, candidate);
            let fits = if full <= max_width + 0.001 {
                true
            } else {
                let content_end = trim_end(text, start, candidate);
                content_end > start
                    && range_width(advances, start, content_end) <= max_width + 0.001
            };
            if fits {
                best = Some((candidate, probe));
                if candidate == text.len() {
                    break;
                }
                probe += 1;
            } else {
                break;
            }
        }
        let end = match best {
            Some((candidate, _)) => candidate,
            None => {
                while boundaries
                    .get(boundary_cursor)
                    .is_some_and(|boundary| *boundary <= start)
                {
                    boundary_cursor += 1;
                }
                let mut cut = None;
                let mut probe = boundary_cursor;
                while let Some(&boundary) = boundaries.get(probe) {
                    if boundary <= start {
                        probe += 1;
                        continue;
                    }
                    if range_width(advances, start, boundary) <= max_width + 0.001 {
                        cut = Some(boundary);
                        probe += 1;
                    } else {
                        break;
                    }
                }
                cut.unwrap_or_else(|| {
                    boundaries
                        .get(boundary_cursor)
                        .copied()
                        .unwrap_or(text.len())
                })
            }
        };
        let end = end.max(start.saturating_add(1)).min(text.len());
        ranges.push(start..end);
        start = end;
    }
    (ranges, false)
}
