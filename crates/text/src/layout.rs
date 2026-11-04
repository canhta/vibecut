//! Paragraph layout: NFC normalization, bundled-font fallback, OpenType shaping, line breaking
//! at Unicode opportunities and alignment.
//!
//! Coordinates are pixels with y down. The origin is the top-left of the text box; the first
//! baseline sits at the first line's ascent. Text is laid out left to right; bidirectional
//! reordering is not part of this crate's contract.

use std::collections::VecDeque;
use std::ops::Range;

use harfrust::{Buffer, Direction, Feature, ShapeOptions, ShaperFont, Tag, shape};
use unicode_linebreak::linebreaks;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use crate::fonts::{self, FaceId, Resolved};

/// Horizontal alignment within the requested maximum width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How a language profile wants lines broken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineBreak {
    /// Unicode UAX #14 opportunities (words, CJK ideographs, punctuation).
    #[default]
    Unicode,
    /// Every grapheme boundary, for scripts a profile wants to break anywhere.
    Grapheme,
}

/// Layout inputs. The maximum width is always applied, so every line wraps.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutOptions<'a> {
    pub family: &'a str,
    pub style: &'a str,
    /// Font size in pixels; finite and greater than zero.
    pub size: f32,
    /// Wrap width in pixels; finite and greater than zero.
    pub max_width: f32,
    pub align: Align,
    /// Baseline distance in pixels; zero uses the primary font's natural line height.
    pub line_height: f32,
    /// Maximum number of lines; further lines are dropped and [`Layout::overflow`] is set.
    pub max_lines: Option<usize>,
    /// Language-profile font fallback families, in order. Empty uses the generic bundled order.
    pub fallbacks: &'a [&'a str],
    /// Language-profile line-breaking mode.
    pub line_break: LineBreak,
}

impl Default for LayoutOptions<'_> {
    fn default() -> Self {
        Self {
            family: fonts::DEFAULT_FAMILY,
            style: "Regular",
            size: 100.0,
            max_width: 1000.0,
            align: Align::Left,
            line_height: 0.0,
            max_lines: None,
            fallbacks: &[],
            line_break: LineBreak::Unicode,
        }
    }
}

/// A positioned glyph. `(x, y)` is the glyph origin on its baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub face: FaceId,
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// UTF-8 byte offset of the glyph's cluster in [`Layout::text`].
    pub cluster: usize,
    pub synth_bold: bool,
    pub synth_italic: bool,
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Byte range in [`Layout::text`], including trailing spaces that hang past the content.
    pub range: Range<usize>,
    pub baseline: f32,
    pub x: f32,
    /// Width of the visible content, excluding trailing whitespace.
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    /// Range of [`Layout::glyphs`].
    pub glyphs: Range<usize>,
}

/// Content bounds `[x0, y0, x1, y1]` in layout coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// The shaped and positioned result.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// NFC-normalized source text. Cluster offsets and line ranges refer to this string.
    pub text: String,
    pub glyphs: Vec<Glyph>,
    pub lines: Vec<Line>,
    pub bounds: Bounds,
    /// The requested family was not found and Inter was substituted.
    pub missing_font: bool,
    /// Lines were dropped because of [`LayoutOptions::max_lines`].
    pub overflow: bool,
}

/// Invalid layout input or a shaping failure.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("font size must be finite, greater than zero and at most {MAX_FONT_SIZE} pixels")]
    InvalidSize,
    #[error("maximum width must be finite and greater than zero")]
    InvalidWidth,
    #[error("line height must be finite, not negative and at most {MAX_LINE_HEIGHT} pixels")]
    InvalidLineHeight,
    #[error("maximum lines must be greater than zero")]
    InvalidMaxLines,
    #[error("text exceeds the supported length of {limit} bytes")]
    TextTooLarge { limit: usize },
    #[error("layout produced a non-finite coordinate")]
    CoordinateOverflow,
    #[error("shaping failed")]
    Shaping(#[from] harfrust::ShapeError),
}

/// The largest accepted input, bounding cluster and prefix allocations.
const MAX_TEXT_BYTES: usize = 16 << 20;
/// The largest font size accepted; larger values only risk non-finite geometry.
const MAX_FONT_SIZE: f32 = 4096.0;
/// The largest explicit baseline distance accepted.
const MAX_LINE_HEIGHT: f32 = 16_384.0;

#[derive(Clone, Copy, Debug)]
struct ShapedGlyph {
    face: FaceId,
    id: u32,
    cluster: usize,
    advance: f32,
    offset_x: f32,
    offset_y: f32,
    synth_bold: bool,
    synth_italic: bool,
}

/// Shape and position a paragraph of text.
pub fn layout(text: &str, options: &LayoutOptions<'_>) -> Result<Layout, TextError> {
    if !(options.size.is_finite() && options.size > 0.0 && options.size <= MAX_FONT_SIZE) {
        return Err(TextError::InvalidSize);
    }
    if !(options.max_width.is_finite() && options.max_width > 0.0) {
        return Err(TextError::InvalidWidth);
    }
    if !(options.line_height.is_finite()
        && options.line_height >= 0.0
        && options.line_height <= MAX_LINE_HEIGHT)
    {
        return Err(TextError::InvalidLineHeight);
    }
    if options.max_lines == Some(0) {
        return Err(TextError::InvalidMaxLines);
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(TextError::TextTooLarge {
            limit: MAX_TEXT_BYTES,
        });
    }

    let normalized = canonical_text(text);
    if normalized.len() > MAX_TEXT_BYTES {
        return Err(TextError::TextTooLarge {
            limit: MAX_TEXT_BYTES,
        });
    }

    let primary = fonts::resolve(options.family, options.style);
    let fallback_chain = fonts::fallback_chain(options.fallbacks, options.style);
    let mut layout = Layout {
        text: normalized,
        glyphs: Vec::new(),
        lines: Vec::new(),
        bounds: Bounds::default(),
        missing_font: primary.missing,
        overflow: false,
    };

    let source = layout.text.clone();
    let mut paragraph_start = 0usize;
    let mut context = ParagraphContext {
        primary,
        options,
        fallbacks: &fallback_chain,
        baseline: 0.0,
        first_line: true,
    };
    for (index, part) in source.split('\n').enumerate() {
        if index > 0 && paragraph_start >= source.len() {
            break;
        }
        let paragraph = part;
        let paragraph_overflow =
            append_paragraph(&mut layout, paragraph, paragraph_start, &mut context)?;
        layout.overflow |= paragraph_overflow;
        paragraph_start = paragraph_start.saturating_add(part.len()).saturating_add(1);
        if layout.overflow {
            break;
        }
    }
    if layout.lines.is_empty() {
        let metrics = fonts::face(primary.face).metrics(options.size);
        layout.lines.push(Line {
            range: 0..0,
            baseline: metrics.ascent,
            x: 0.0,
            width: 0.0,
            ascent: metrics.ascent,
            descent: metrics.descent,
            glyphs: 0..0,
        });
    }
    layout.bounds = content_bounds(&layout.lines);
    Ok(layout)
}

struct ParagraphContext<'a, 'b> {
    primary: Resolved,
    options: &'a LayoutOptions<'b>,
    fallbacks: &'a [fonts::FaceId],
    baseline: f32,
    first_line: bool,
}

fn append_paragraph(
    layout: &mut Layout,
    paragraph: &str,
    base: usize,
    context: &mut ParagraphContext<'_, '_>,
) -> Result<bool, TextError> {
    let options = context.options;
    let remaining = options
        .max_lines
        .map(|max_lines| max_lines.saturating_sub(layout.lines.len()));
    if remaining == Some(0) {
        return Ok(true);
    }

    // The paragraph-wide shape supplies break measurements; every emitted line is reshaped on
    // its own span so a ligature can never straddle a line boundary.
    let measured = shape_paragraph(
        paragraph,
        base,
        context.primary,
        options.size,
        context.fallbacks,
        false,
    )?;
    let nominal = shape_paragraph(
        paragraph,
        base,
        context.primary,
        options.size,
        context.fallbacks,
        true,
    )?;
    let advances = advance_prefix(paragraph, &measured, base);
    let boundaries = grapheme_boundaries(paragraph);
    let (ranges, mut overflow) = line_ranges(
        paragraph,
        options.max_width,
        &advances,
        &boundaries,
        options.line_break,
        remaining,
    );
    if ranges.is_empty() {
        return Ok(overflow);
    }

    let primary_metrics = fonts::face(context.primary.face).metrics(options.size);
    let natural_step = primary_metrics.ascent + primary_metrics.descent + primary_metrics.line_gap;
    let step = if options.line_height > 0.0 {
        options.line_height
    } else {
        natural_step.max(options.size * 0.5)
    };
    if !step.is_finite() || step <= 0.0 {
        return Err(TextError::CoordinateOverflow);
    }

    let mut pending: VecDeque<Range<usize>> = ranges.into();
    let mut glyph_cursor = 0usize;
    let mut nominal_cursor = 0usize;
    let mut span_end = vec![base + paragraph.len(); measured.len()];
    let mut group_start = 0usize;
    while group_start < measured.len() {
        let Some(cluster) = measured.get(group_start).map(|glyph| glyph.cluster) else {
            break;
        };
        let mut group_end = group_start + 1;
        while measured
            .get(group_end)
            .is_some_and(|glyph| glyph.cluster == cluster)
        {
            group_end += 1;
        }
        let span = measured
            .get(group_end)
            .map_or(base + paragraph.len(), |glyph| glyph.cluster);
        for index in group_start..group_end {
            if let Some(slot) = span_end.get_mut(index) {
                *slot = span;
            }
        }
        group_start = group_end;
    }

    while let Some(range) = pending.pop_front() {
        if options
            .max_lines
            .is_some_and(|max_lines| layout.lines.len() >= max_lines)
        {
            overflow = true;
            break;
        }
        while measured
            .get(glyph_cursor)
            .is_some_and(|glyph| glyph.cluster < base + range.start)
        {
            glyph_cursor += 1;
        }
        let mut measured_end = glyph_cursor;
        while measured
            .get(measured_end)
            .is_some_and(|glyph| glyph.cluster < base + range.end)
        {
            measured_end += 1;
        }
        while nominal
            .get(nominal_cursor)
            .is_some_and(|glyph| glyph.cluster < base + range.start)
        {
            nominal_cursor += 1;
        }
        let mut nominal_end = nominal_cursor;
        while nominal
            .get(nominal_end)
            .is_some_and(|glyph| glyph.cluster < base + range.end)
        {
            nominal_end += 1;
        }
        let start_crosses = glyph_cursor > 0
            && span_end
                .get(glyph_cursor.saturating_sub(1))
                .is_some_and(|end| *end > base + range.start);
        let end_crosses = measured_end > glyph_cursor
            && span_end
                .get(measured_end.saturating_sub(1))
                .is_some_and(|end| *end > base + range.end);
        let context_changed = !glyphs_same(
            measured.get(glyph_cursor..measured_end).unwrap_or(&[]),
            nominal.get(nominal_cursor..nominal_end).unwrap_or(&[]),
        );
        let reshape = start_crosses || end_crosses || context_changed;

        let Some(line_text) = paragraph.get(range.clone()) else {
            return Err(TextError::CoordinateOverflow);
        };
        let line_base = base + range.start;
        let content_end_local = trim_end(line_text, 0, line_text.len());
        let content_end = line_base + content_end_local;
        let reshaped;
        let line_glyphs: &[ShapedGlyph] = if reshape {
            reshaped = shape_paragraph(
                line_text,
                line_base,
                context.primary,
                options.size,
                context.fallbacks,
                false,
            )?;
            &reshaped
        } else {
            measured.get(glyph_cursor..measured_end).unwrap_or(&[])
        };
        let content_width = if reshape {
            let line_advances = advance_prefix(line_text, line_glyphs, line_base);
            range_width(&line_advances, 0, content_end_local)
        } else {
            range_width(
                &advances,
                range.start,
                range.start.saturating_add(content_end_local),
            )
        };
        if !content_width.is_finite() {
            return Err(TextError::CoordinateOverflow);
        }
        if content_width > options.max_width + 0.01 && line_text.graphemes(true).count() > 1 {
            let boundary_index = boundaries.partition_point(|boundary| *boundary < range.end);
            if let Some(previous) = boundary_index
                .checked_sub(1)
                .and_then(|index| boundaries.get(index))
                .copied()
                .filter(|previous| *previous > range.start)
            {
                let suffix = previous..range.end;
                pending.push_front(suffix);
                pending.push_front(range.start..previous);
                continue;
            }
        }

        let shift = match options.align {
            Align::Left => 0.0,
            Align::Center => (options.max_width - content_width).max(0.0) / 2.0,
            Align::Right => (options.max_width - content_width).max(0.0),
        };
        let (ascent, descent) = line_metrics(line_glyphs, context.primary, options.size);
        if !ascent.is_finite() || !descent.is_finite() {
            return Err(TextError::CoordinateOverflow);
        }
        if context.first_line {
            context.baseline = ascent;
            context.first_line = false;
        } else {
            context.baseline += step;
        }
        if !context.baseline.is_finite() || !shift.is_finite() {
            return Err(TextError::CoordinateOverflow);
        }
        let glyph_start = layout.glyphs.len();
        let mut pen = 0.0f32;
        for glyph in line_glyphs
            .iter()
            .filter(|glyph| glyph.cluster < content_end)
        {
            let x = shift + pen + glyph.offset_x;
            let y = context.baseline - glyph.offset_y;
            if !x.is_finite() || !y.is_finite() {
                return Err(TextError::CoordinateOverflow);
            }
            layout.glyphs.push(Glyph {
                face: glyph.face,
                id: glyph.id,
                x,
                y,
                size: options.size,
                cluster: glyph.cluster,
                synth_bold: glyph.synth_bold,
                synth_italic: glyph.synth_italic,
            });
            pen += glyph.advance;
        }
        if !pen.is_finite() {
            return Err(TextError::CoordinateOverflow);
        }
        layout.lines.push(Line {
            range: line_base..line_base + line_text.len(),
            baseline: context.baseline,
            x: shift,
            width: content_width,
            ascent,
            descent,
            glyphs: glyph_start..layout.glyphs.len(),
        });
        glyph_cursor = measured_end;
        nominal_cursor = nominal_end;
    }
    Ok(overflow)
}

fn shape_paragraph(
    paragraph: &str,
    base: usize,
    primary: Resolved,
    size: f32,
    fallbacks: &[fonts::FaceId],
    nominal: bool,
) -> Result<Vec<ShapedGlyph>, TextError> {
    let mut out = Vec::new();
    let mut run_start = 0usize;
    while run_start < paragraph.len() {
        let Some((_, first)) = paragraph
            .get(run_start..)
            .and_then(|rest| rest.char_indices().next())
        else {
            break;
        };
        let face_id = fonts::fallback_for(first, primary.face, fallbacks);
        let mut run_end = run_start;
        for (offset, c) in paragraph.get(run_start..).unwrap_or("").char_indices() {
            let candidate = fonts::fallback_for(c, primary.face, fallbacks);
            if candidate != face_id {
                break;
            }
            run_end = run_start + offset + c.len_utf8();
        }
        let Some(run) = paragraph.get(run_start..run_end) else {
            break;
        };
        shape_run(
            run,
            base + run_start,
            face_id,
            primary,
            size,
            nominal,
            &mut out,
        )?;
        run_start = run_end;
    }
    Ok(out)
}

fn shape_run(
    run: &str,
    cluster_base: usize,
    face_id: FaceId,
    primary: Resolved,
    size: f32,
    nominal: bool,
    out: &mut Vec<ShapedGlyph>,
) -> Result<(), TextError> {
    let face = fonts::face(face_id);
    let Some(font) = face.font_ref() else {
        return Ok(());
    };
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    for (offset, c) in run.char_indices() {
        buffer.push(c as u32, (cluster_base + offset) as u32);
    }
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    let features = [
        Feature::new(Tag::new(b"kern"), 0, ..),
        Feature::new(Tag::new(b"liga"), 0, ..),
        Feature::new(Tag::new(b"clig"), 0, ..),
        Feature::new(Tag::new(b"calt"), 0, ..),
    ];
    let options = if nominal {
        ShapeOptions::new().features(&features)
    } else {
        ShapeOptions::new()
    };
    shape(&shaper, &mut buffer, options)?;
    let scale = face.scale(size);
    let synth_bold = primary.synth_bold && face_id == primary.face;
    let synth_italic = primary.synth_italic && face_id == primary.face;
    for (info, position) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
        out.push(ShapedGlyph {
            face: face_id,
            id: info.glyph_id,
            cluster: info.cluster as usize,
            advance: position.x_advance as f32 * scale,
            offset_x: position.x_offset as f32 * scale,
            offset_y: position.y_offset as f32 * scale,
            synth_bold,
            synth_italic,
        });
    }
    Ok(())
}

/// Prefix sums of shaped advances by byte offset.
fn advance_prefix(text: &str, glyphs: &[ShapedGlyph], base: usize) -> Vec<f32> {
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

fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    boundaries.push(text.len());
    boundaries
}

fn range_width(prefix: &[f32], start: usize, end: usize) -> f32 {
    let from = prefix.get(start).copied().unwrap_or(0.0);
    let to = prefix.get(end).copied().unwrap_or(from);
    (to - from).max(0.0)
}

/// End of `start..end` without trailing whitespace.
fn trim_end(text: &str, start: usize, end: usize) -> usize {
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
fn line_ranges(
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

fn glyphs_same(a: &[ShapedGlyph], b: &[ShapedGlyph]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(left, right)| {
            left.id == right.id
                && left.face == right.face
                && left.cluster == right.cluster
                && (left.advance - right.advance).abs() < 0.01
                && (left.offset_x - right.offset_x).abs() < 0.01
                && (left.offset_y - right.offset_y).abs() < 0.01
        })
}

fn line_metrics(glyphs: &[ShapedGlyph], primary: Resolved, size: f32) -> (f32, f32) {
    let metrics = fonts::face(primary.face).metrics(size);
    let mut ascent = metrics.ascent;
    let mut descent = metrics.descent;
    for glyph in glyphs {
        if glyph.face != primary.face {
            let fallback = fonts::face(glyph.face).metrics(size);
            ascent = ascent.max(fallback.ascent);
            descent = descent.max(fallback.descent);
        }
    }
    (ascent, descent)
}

/// Canonical line separators become LF, then the text is normalized to NFC.
fn canonical_text(text: &str) -> String {
    let mut canonical = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                canonical.push('\n');
            }
            '\n' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}' => {
                canonical.push('\n');
            }
            _ => canonical.push(c),
        }
    }
    canonical.nfc().collect()
}

fn content_bounds(lines: &[Line]) -> Bounds {
    let mut bounds = Bounds {
        x0: f32::MAX,
        y0: f32::MAX,
        x1: f32::MIN,
        y1: f32::MIN,
    };
    for line in lines {
        bounds.x0 = bounds.x0.min(line.x);
        bounds.y0 = bounds.y0.min(line.baseline - line.ascent);
        bounds.x1 = bounds.x1.max(line.x + line.width);
        bounds.y1 = bounds.y1.max(line.baseline + line.descent);
    }
    if !bounds.x0.is_finite() || !bounds.x1.is_finite() {
        return Bounds::default();
    }
    bounds
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn boundaries_of(text: &str) -> Vec<usize> {
        let mut boundaries: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
        boundaries.push(text.len());
        boundaries
    }

    fn options(width: f32) -> LayoutOptions<'static> {
        LayoutOptions {
            size: 40.0,
            max_width: width,
            ..LayoutOptions::default()
        }
    }

    #[test]
    fn wraps_on_spaces_and_never_splits_a_grapheme() {
        let text = "Người Việt ưỡn ngực quyết định đặt hàng";
        let layout = layout(text, &options(220.0)).expect("layout");
        assert!(layout.lines.len() > 1);
        for line in &layout.lines {
            assert!(line.width <= 220.0 + 0.01, "{line:?}");
            assert!(text.is_char_boundary(line.range.start));
            assert!(text.is_char_boundary(line.range.end));
        }
        let graphemes = boundaries_of(text);
        for line in &layout.lines {
            assert!(graphemes.binary_search(&line.range.start).is_ok());
            assert!(graphemes.binary_search(&line.range.end).is_ok());
        }
    }

    #[test]
    fn a_long_unbreakable_word_is_split_at_grapheme_boundaries() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let layout = layout(text, &options(60.0)).expect("layout");
        assert!(layout.lines.len() >= 4);
        let graphemes = boundaries_of(text);
        for line in &layout.lines {
            assert!(graphemes.binary_search(&line.range.start).is_ok());
            assert!(graphemes.binary_search(&line.range.end).is_ok());
            assert!(line.width <= 60.0 + 0.01, "{line:?}");
        }
    }

    #[test]
    fn alignment_places_lines_inside_the_box() {
        let text = "short\nlonger line";
        for align in [Align::Left, Align::Center, Align::Right] {
            let layout = layout(
                text,
                &LayoutOptions {
                    align,
                    ..options(400.0)
                },
            )
            .expect("layout");
            assert_eq!(layout.lines.len(), 2);
            for line in &layout.lines {
                assert!(
                    line.x >= -0.01 && line.x + line.width <= 400.01,
                    "{align:?} {line:?}"
                );
            }
        }
    }

    #[test]
    fn max_lines_truncates_and_flags_overflow() {
        let text = "one two three four five six seven eight nine";
        let full = layout(text, &options(120.0)).expect("layout");
        assert!(full.lines.len() > 2);
        let clipped = layout(
            text,
            &LayoutOptions {
                max_lines: Some(2),
                ..options(120.0)
            },
        )
        .expect("layout");
        assert_eq!(clipped.lines.len(), 2);
        assert!(clipped.overflow);
        assert_eq!(
            clipped.lines,
            full.lines
                .get(..2)
                .map(<[Line]>::to_vec)
                .unwrap_or_default()
        );
    }

    fn assert_complete_and_independently_shaped(
        text: &str,
        options: &LayoutOptions<'static>,
    ) -> Layout {
        let laid_out = layout(text, options).expect("layout");
        assert!(!laid_out.lines.is_empty());
        assert_eq!(laid_out.lines.first().map(|line| line.range.start), Some(0));
        for pair in laid_out.lines.windows(2) {
            assert_eq!(
                pair.first().map(|line| line.range.end),
                pair.get(1).map(|line| line.range.start),
                "{:?}",
                laid_out.lines
            );
        }
        assert_eq!(
            laid_out.lines.last().map(|line| line.range.end),
            Some(laid_out.text.len())
        );
        let reconstructed: String = laid_out
            .lines
            .iter()
            .filter_map(|line| laid_out.text.get(line.range.clone()))
            .collect();
        assert_eq!(reconstructed, laid_out.text, "line ranges cover the source");

        let independent_options = LayoutOptions {
            max_width: 10_000.0,
            align: Align::Left,
            max_lines: None,
            ..options.clone()
        };
        for line in &laid_out.lines {
            let span = laid_out.text.get(line.range.clone()).expect("line span");
            let independent = layout(span, &independent_options).expect("independent layout");
            assert_eq!(independent.lines.len(), 1);
            assert!(
                (independent.lines[0].width - line.width).abs() < 0.011,
                "line {:?} width {} vs independent {}",
                line.range,
                line.width,
                independent.lines[0].width
            );
            let line_glyphs = laid_out.glyphs.get(line.glyphs.clone()).unwrap_or_default();
            assert_eq!(line_glyphs.len(), independent.glyphs.len(), "{span:?}");
            for (actual, expected) in line_glyphs.iter().zip(&independent.glyphs) {
                assert_eq!(actual.id, expected.id, "{span:?}");
                assert_eq!(actual.face, expected.face, "{span:?}");
                assert_eq!(
                    actual.cluster - line.range.start,
                    expected.cluster,
                    "{span:?}"
                );
                assert!(
                    (actual.x - line.x - expected.x).abs() < 0.011,
                    "{span:?}: {} vs {}",
                    actual.x - line.x,
                    expected.x
                );
                let isolated_baseline = independent.lines[0].baseline;
                assert!(
                    (actual.y - line.baseline - (expected.y - isolated_baseline)).abs() < 0.011,
                    "{span:?}"
                );
                assert!((actual.size - expected.size).abs() < 0.001);
            }
        }
        laid_out
    }

    #[test]
    fn narrow_lines_cover_the_source_and_match_independent_shaping() {
        for width in [12.0, 20.0, 30.0] {
            let options = LayoutOptions {
                family: "Noto Serif",
                size: 40.0,
                max_width: width,
                ..LayoutOptions::default()
            };
            assert_complete_and_independently_shaped("office", &options);
        }
    }

    #[test]
    fn cross_boundary_kerning_does_not_change_standalone_line_metrics() {
        let cases = [
            ("Inter", "AV", 26.0, Align::Right),
            ("Inter", "To", 24.0, Align::Right),
            ("Inter", "AV", 30.0, Align::Center),
            ("Inter", "WA", 20.0, Align::Left),
            ("Inter", "Yo", 22.0, Align::Right),
            ("Inter", "Ta", 23.0, Align::Center),
        ];
        for (family, text, width, align) in cases {
            let options = LayoutOptions {
                family,
                size: 40.0,
                max_width: width,
                align,
                ..LayoutOptions::default()
            };
            let laid_out = assert_complete_and_independently_shaped(text, &options);
            for line in &laid_out.lines {
                let graphemes = laid_out
                    .text
                    .get(line.range.clone())
                    .map(|span| span.graphemes(true).count())
                    .unwrap_or(0);
                if graphemes > 1 {
                    assert!(line.width <= width + 0.011, "{line:?}");
                }
            }
        }
    }

    #[test]
    fn mandatory_unicode_separators_become_lines_without_tofu() {
        for input in [
            "ab\r\ncd",
            "ab\rcd",
            "ab\u{0085}cd",
            "ab\u{2028}cd",
            "ab\u{2029}cd",
            "ab\u{000B}cd",
            "ab\u{000C}cd",
        ] {
            let laid_out = layout(input, &options(400.0)).expect("layout");
            assert_eq!(laid_out.lines.len(), 2, "{input:?}");
            assert_eq!(laid_out.text, "ab\ncd", "{input:?}");
            assert!(
                laid_out.glyphs.iter().all(|glyph| glyph.id != 0),
                "{input:?} produced tofu"
            );
        }
    }

    #[test]
    fn extreme_finite_sizes_are_rejected_instead_of_producing_infinities() {
        let text = "AA\nAA";
        assert!(matches!(
            layout(
                text,
                &LayoutOptions {
                    size: f32::MAX,
                    ..options(1000.0)
                }
            ),
            Err(TextError::InvalidSize)
        ));
        assert!(matches!(
            layout(
                text,
                &LayoutOptions {
                    line_height: f32::MAX,
                    ..options(1000.0)
                }
            ),
            Err(TextError::InvalidLineHeight)
        ));
        let laid_out = layout(
            text,
            &LayoutOptions {
                size: MAX_FONT_SIZE,
                ..options(10_000.0)
            },
        )
        .expect("bounded size");
        assert!(
            laid_out
                .glyphs
                .iter()
                .all(|glyph| glyph.x.is_finite() && glyph.y.is_finite())
        );
        assert!(laid_out.bounds.x0.is_finite() && laid_out.bounds.y1.is_finite());
    }

    #[test]
    fn narrow_wrapping_is_bounded_by_max_lines_and_scales_linearly() {
        let text = "a".repeat(8_000);
        let started = Instant::now();
        let clipped = layout(
            &text,
            &LayoutOptions {
                max_lines: Some(2),
                ..options(1.0)
            },
        )
        .expect("clipped layout");
        assert_eq!(clipped.lines.len(), 2);
        assert!(clipped.overflow);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );

        let started = Instant::now();
        let full = layout(&text, &options(1.0)).expect("full layout");
        assert_eq!(full.lines.len(), text.len());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert!(matches!(
            layout(
                "x",
                &LayoutOptions {
                    size: 0.0,
                    ..options(10.0)
                }
            ),
            Err(TextError::InvalidSize)
        ));
        assert!(matches!(
            layout(
                "x",
                &LayoutOptions {
                    max_width: f32::NAN,
                    ..options(10.0)
                }
            ),
            Err(TextError::InvalidWidth)
        ));
        assert!(matches!(
            layout(
                "x",
                &LayoutOptions {
                    line_height: -1.0,
                    ..options(10.0)
                }
            ),
            Err(TextError::InvalidLineHeight)
        ));
        assert!(matches!(
            layout(
                "x",
                &LayoutOptions {
                    max_lines: Some(0),
                    ..options(10.0)
                }
            ),
            Err(TextError::InvalidMaxLines)
        ));
    }
}
