//! Paragraph layout: NFC normalization, bundled-font fallback, OpenType shaping, line breaking
//! at Unicode opportunities and alignment.
//!
//! Coordinates are pixels with y down. The origin is the top-left of the text box; the first
//! baseline sits at the first line's ascent. Text is laid out left to right; bidirectional
//! reordering is not part of this crate's contract.

use std::ops::Range;

use harfrust::{Buffer, Direction, ShapeOptions, ShaperFont, shape};
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
    #[error("font size must be finite and greater than zero")]
    InvalidSize,
    #[error("maximum width must be finite and greater than zero")]
    InvalidWidth,
    #[error("line height must be finite and not negative")]
    InvalidLineHeight,
    #[error("maximum lines must be greater than zero")]
    InvalidMaxLines,
    #[error("text exceeds the supported length of {limit} bytes")]
    TextTooLarge { limit: usize },
    #[error("shaping failed")]
    Shaping(#[from] harfrust::ShapeError),
}

/// The largest accepted input, bounding cluster and prefix allocations.
const MAX_TEXT_BYTES: usize = 16 << 20;

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
    if !(options.size.is_finite() && options.size > 0.0) {
        return Err(TextError::InvalidSize);
    }
    if !(options.max_width.is_finite() && options.max_width > 0.0) {
        return Err(TextError::InvalidWidth);
    }
    if !(options.line_height.is_finite() && options.line_height >= 0.0) {
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

    let normalized: String = text.nfc().collect();
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
        let paragraph = part.strip_suffix('\r').unwrap_or(part);
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
    let shaped = shape_paragraph(
        paragraph,
        base,
        context.primary,
        options.size,
        context.fallbacks,
    )?;
    let advances = advance_prefix(paragraph, &shaped, base);
    let boundaries = grapheme_boundaries(paragraph);
    let mut ranges = line_ranges(
        paragraph,
        options.max_width,
        &advances,
        &boundaries,
        options.line_break,
    );
    let mut overflow = false;
    if let Some(max_lines) = options.max_lines {
        let remaining = max_lines.saturating_sub(layout.lines.len());
        if ranges.len() > remaining {
            ranges.truncate(remaining);
            overflow = true;
        }
    }
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

    for range in ranges {
        let start = base + range.start;
        let end = base + range.end;
        let content_end = base + trim_end(paragraph, range.start, range.end);
        let content_width = range_width(
            &advances,
            range.start,
            trim_end(paragraph, range.start, range.end),
        );
        let shift = match options.align {
            Align::Left => 0.0,
            Align::Center => (options.max_width - content_width).max(0.0) / 2.0,
            Align::Right => (options.max_width - content_width).max(0.0),
        };
        let (ascent, descent) = line_metrics(&shaped, start..end, context.primary, options.size);
        if context.first_line {
            context.baseline = ascent;
            context.first_line = false;
        } else {
            context.baseline += step;
        }
        let glyph_start = layout.glyphs.len();
        let mut pen = 0.0f32;
        for glyph in shaped
            .iter()
            .filter(|g| g.cluster >= start && g.cluster < content_end)
        {
            layout.glyphs.push(Glyph {
                face: glyph.face,
                id: glyph.id,
                x: shift + pen + glyph.offset_x,
                y: context.baseline - glyph.offset_y,
                size: options.size,
                cluster: glyph.cluster,
                synth_bold: glyph.synth_bold,
                synth_italic: glyph.synth_italic,
            });
            pen += glyph.advance;
        }
        layout.lines.push(Line {
            range: start..end,
            baseline: context.baseline,
            x: shift,
            width: content_width,
            ascent,
            descent,
            glyphs: glyph_start..layout.glyphs.len(),
        });
    }
    Ok(overflow)
}

fn shape_paragraph(
    paragraph: &str,
    base: usize,
    primary: Resolved,
    size: f32,
    fallbacks: &[fonts::FaceId],
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
        shape_run(run, base + run_start, face_id, primary, size, &mut out)?;
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
    shape(&shaper, &mut buffer, ShapeOptions::new())?;
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

fn line_ranges(
    text: &str,
    max_width: f32,
    advances: &[f32],
    boundaries: &[usize],
    mode: LineBreak,
) -> Vec<Range<usize>> {
    if text.is_empty() {
        return std::iter::once(0..0).collect();
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
    while start < text.len() {
        let mut best = None;
        for candidate in breaks
            .iter()
            .copied()
            .filter(|candidate| *candidate > start)
        {
            let content_end = trim_end(text, start, candidate);
            if range_width(advances, start, content_end) <= max_width + 0.001 {
                best = Some(candidate);
                if candidate == text.len() {
                    break;
                }
            } else {
                break;
            }
        }
        let end = match best {
            Some(end) => end,
            None => {
                let mut cut = None;
                for boundary in boundaries
                    .iter()
                    .copied()
                    .filter(|boundary| *boundary > start)
                {
                    if range_width(advances, start, boundary) <= max_width + 0.001 {
                        cut = Some(boundary);
                    } else {
                        break;
                    }
                }
                cut.or_else(|| {
                    boundaries
                        .iter()
                        .copied()
                        .find(|boundary| *boundary > start)
                })
                .unwrap_or(text.len())
            }
        };
        let end = end.max(start.saturating_add(1)).min(text.len());
        ranges.push(start..end);
        start = end;
    }
    ranges
}

fn line_metrics(
    glyphs: &[ShapedGlyph],
    range: Range<usize>,
    primary: Resolved,
    size: f32,
) -> (f32, f32) {
    let metrics = fonts::face(primary.face).metrics(size);
    let mut ascent = metrics.ascent;
    let mut descent = metrics.descent;
    for glyph in glyphs.iter().filter(|glyph| range.contains(&glyph.cluster)) {
        if glyph.face != primary.face {
            let fallback = fonts::face(glyph.face).metrics(size);
            ascent = ascent.max(fallback.ascent);
            descent = descent.max(fallback.descent);
        }
    }
    (ascent, descent)
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
