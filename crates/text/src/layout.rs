//! Lays out a paragraph from normalized text into positioned lines.
//!
//! Coordinates are pixels with y down. The origin is the top-left of the text box; the first
//! baseline sits at the first line's ascent. Text is laid out left to right; bidirectional
//! reordering is not part of this crate's contract.
mod breaks;
mod paragraph;

use crate::fonts::{self, FaceId};
pub use paragraph::canonical_text;
use paragraph::{ParagraphContext, append_paragraph, content_bounds};
use std::ops::Range;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

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
    /// Extra advance in pixels after every grapheme cluster, including the last one of a
    /// line; negative values tighten. Finite and at most 4096 pixels in magnitude.
    pub letter_spacing: f32,
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
            letter_spacing: 0.0,
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
    /// Exclusive end supplied by the actual shaping run, in canonical text bytes.
    pub cluster_end: usize,
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

/// Source grapheme boundaries paired with byte offsets after the layout's exact
/// line-ending/NFC normalization. Input shares layout's 16 MiB ceiling.
pub fn canonical_boundaries(source: &str) -> Result<Vec<(usize, usize)>, TextError> {
    if source.len() > MAX_TEXT_BYTES {
        return Err(TextError::TextTooLarge {
            limit: MAX_TEXT_BYTES,
        });
    }
    let mut result = Vec::with_capacity(source.chars().count().saturating_add(1));
    result.push((0, 0));
    let mut offset = 0;
    for (start, grapheme) in source.grapheme_indices(true) {
        offset += canonical_text(grapheme).len();
        result.push((start + grapheme.len(), offset));
    }
    Ok(result)
}

/// A caret position in layout pixels with y down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Caret {
    /// Index into [`Layout::lines`].
    pub line: usize,
    /// Leading x of the caret.
    pub x: f32,
    /// Top of the line box containing the caret.
    pub top: f32,
    /// Bottom of the line box containing the caret.
    pub bottom: f32,
}

impl Layout {
    /// Whether a canonical byte span contains whole shaped clusters and graphemes.
    /// A word crossing a ligature or combining cluster cannot be painted independently.
    pub fn certifies_span(&self, span: Range<usize>) -> bool {
        self.certifies_spans(&[(span.start, span.end)])
    }

    /// Certify ordered, disjoint canonical spans against the same shaped glyphs.
    /// Visits each glyph once with ordered span lookup, avoiding words times glyphs.
    pub fn certifies_spans(&self, spans: &[(usize, usize)]) -> bool {
        let boundaries: std::collections::BTreeSet<_> = self
            .text
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain(std::iter::once(self.text.len()))
            .collect();
        let mut previous = 0;
        for &(start, end) in spans {
            if start < previous
                || start >= end
                || !boundaries.contains(&start)
                || !boundaries.contains(&end)
            {
                return false;
            }
            previous = end;
        }
        let mut covered = vec![false; spans.len()];
        for glyph in &self.glyphs {
            let index = spans.partition_point(|&(_, end)| end <= glyph.cluster);
            let Some(&(start, end)) = spans.get(index) else {
                continue;
            };
            if glyph.cluster_end <= start {
                continue;
            }
            if glyph.id == 0 || glyph.cluster < start || glyph.cluster_end > end {
                return false;
            }
            if let Some(value) = covered.get_mut(index) {
                *value = true;
            }
        }
        covered.into_iter().all(|value| value)
    }

    /// Caret at a character index in `source`, the string this layout was built from.
    ///
    /// Indices past the end clamp to the text end. A position inside a grapheme moves to
    /// the next cluster boundary, as a text caret cannot sit inside a cluster. An index at
    /// a line break stays on the line before it, and one at a line start sits on that line.
    pub fn caret(&self, source: &str, index: usize) -> Caret {
        let byte = source
            .char_indices()
            .nth(index)
            .map_or(source.len(), |(byte, _)| byte);
        let offset = source
            .get(..byte)
            .unwrap_or_default()
            .nfc()
            .map(char::len_utf8)
            .sum::<usize>()
            .min(self.text.len());
        let Some((line_index, line)) = self
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| offset <= line.range.end)
            .or_else(|| self.lines.iter().enumerate().next_back())
        else {
            return Caret::default();
        };
        let x = self
            .glyphs
            .get(line.glyphs.clone())
            .unwrap_or_default()
            .iter()
            .find(|glyph| glyph.cluster >= offset)
            .map_or(line.x + line.width, |glyph| glyph.x);
        Caret {
            line: line_index,
            x,
            top: line.baseline - line.ascent,
            bottom: line.baseline + line.descent,
        }
    }

    /// Source character index nearest a point in layout pixels.
    ///
    /// The point is clamped to the nearest line box by y, then to the nearest caret
    /// x (cluster boundary or line end) within that line. It returns a character
    /// offset in `source`, not a byte offset.
    pub fn index(&self, source: &str, point: [f32; 2]) -> usize {
        let Some(line) = self.nearest_line(point[1]) else {
            return 0;
        };
        let glyphs = self.glyphs.get(line.glyphs.clone()).unwrap_or_default();
        let mut candidates = Vec::new();
        let mut previous = None;
        for glyph in glyphs {
            if previous != Some(glyph.cluster) {
                candidates.push((glyph.x, glyph.cluster));
                previous = Some(glyph.cluster);
            }
        }
        candidates.push((line.x + line.width, line.range.end));
        let mut best = candidates
            .first()
            .copied()
            .unwrap_or((line.x, line.range.start));
        let mut distance = (point[0] - best.0).abs();
        for candidate in candidates.into_iter().skip(1) {
            let next = (point[0] - candidate.0).abs();
            if next < distance {
                best = candidate;
                distance = next;
            }
        }
        source_index(source, best.1)
    }

    fn nearest_line(&self, y: f32) -> Option<&Line> {
        if !y.is_finite() {
            return self.lines.first();
        }
        let distance = |line: &Line| {
            let top = line.baseline - line.ascent;
            let bottom = line.baseline + line.descent;
            if y < top {
                top - y
            } else if y > bottom {
                y - bottom
            } else {
                0.0
            }
        };
        self.lines
            .iter()
            .min_by(|left, right| distance(left).total_cmp(&distance(right)))
    }
}

/// Source character index at a normalized byte offset, snapping forward when the
/// offset falls inside a grapheme cluster.
fn source_index(source: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut normalized = 0usize;
    let mut chars = 0usize;
    for grapheme in source.graphemes(true) {
        let len: usize = grapheme.nfc().map(char::len_utf8).sum();
        let cluster_chars = grapheme.chars().count();
        if normalized + len > offset {
            return chars + cluster_chars;
        }
        normalized += len;
        chars += cluster_chars;
        if normalized == offset {
            return chars;
        }
    }
    chars
}

/// Source character range of the word at a character index.
///
/// Word boundaries follow Unicode word segmentation; whitespace and punctuation
/// runs are words too. Indices past the end clamp to the last word, and an empty
/// source returns an empty range at zero.
pub fn word_range(source: &str, index: usize) -> Range<usize> {
    let byte = source
        .char_indices()
        .nth(index)
        .map_or(source.len(), |(byte, _)| byte);
    for (start, word) in source.split_word_bound_indices() {
        let end = start + word.len();
        if (byte >= start && byte < end) || (byte == source.len() && end == byte) {
            return source[..start].chars().count()..source[..end].chars().count();
        }
    }
    0..0
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
    #[error("letter spacing must be finite and at most {MAX_FONT_SIZE} pixels in magnitude")]
    InvalidLetterSpacing,
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
    if !(options.letter_spacing.is_finite() && options.letter_spacing.abs() <= MAX_FONT_SIZE) {
        return Err(TextError::InvalidLetterSpacing);
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

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
    fn caret_tracks_cluster_edges_lines_and_line_breaks() {
        let text = "Ab\nCd";
        let laid_out = layout(text, &options(300.0)).expect("layout");
        let first = laid_out.caret(text, 0);
        assert_eq!(first.line, 0);
        assert!((first.x - laid_out.glyphs[laid_out.lines[0].glyphs.start].x).abs() < 1e-3);
        assert!((first.top - (laid_out.lines[0].baseline - laid_out.lines[0].ascent)).abs() < 1e-3);
        assert!(
            (first.bottom - (laid_out.lines[0].baseline + laid_out.lines[0].descent)).abs() < 1e-3
        );

        let before_break = laid_out.caret(text, 2);
        assert_eq!(before_break.line, 0);
        assert!((before_break.x - (laid_out.lines[0].x + laid_out.lines[0].width)).abs() < 0.011);

        let second_line = laid_out.caret(text, 3);
        assert_eq!(second_line.line, 1);
        assert!((second_line.x - laid_out.lines[1].x).abs() < 1e-3);

        let end = laid_out.caret(text, text.chars().count());
        assert_eq!(end.line, 1);
        assert!((end.x - (laid_out.lines[1].x + laid_out.lines[1].width)).abs() < 0.011);
        assert!((end.top - (laid_out.lines[1].baseline - laid_out.lines[1].ascent)).abs() < 1e-3);

        let blank = "A\n\nB";
        let laid_out = layout(blank, &options(300.0)).expect("layout");
        assert_eq!(laid_out.lines.len(), 3);
        assert_eq!(laid_out.caret(blank, 2).line, 1);
        assert_eq!(laid_out.caret(blank, 3).line, 2);
        assert!((laid_out.caret(blank, 2).x - laid_out.lines[1].x).abs() < 1e-3);
    }

    #[test]
    fn inverse_index_clamps_to_lines_and_cluster_boundaries() {
        let text = "AbC\ndef";
        let laid_out = layout(text, &options(300.0)).expect("layout");
        for index in 0..=text.chars().count() {
            let caret = laid_out.caret(text, index);
            let y = (caret.top + caret.bottom) / 2.0;
            assert_eq!(
                laid_out.index(text, [caret.x, y]),
                index,
                "caret {index} at {caret:?}"
            );
        }

        let first = &laid_out.lines[0];
        let end = first.x + first.width + 40.0;
        let y = (first.baseline - first.ascent + first.baseline + first.descent) / 2.0;
        assert_eq!(
            laid_out.index(text, [end, y]),
            text[..first.range.end].chars().count()
        );
        let first_y = first.baseline - first.ascent - 40.0;
        assert_eq!(laid_out.index(text, [first.x, first_y]), 0);
        let last = laid_out.lines.last().expect("line");
        let after = last.baseline + last.descent + 40.0;
        assert_eq!(
            laid_out.index(text, [last.x + last.width + 40.0, after]),
            text.chars().count()
        );

        for align in [Align::Left, Align::Center, Align::Right] {
            let laid_out = layout(
                "AbC\ndef",
                &LayoutOptions {
                    align,
                    ..options(300.0)
                },
            )
            .expect("layout");
            for index in 0..=text.chars().count() {
                let caret = laid_out.caret(text, index);
                let y = (caret.top + caret.bottom) / 2.0;
                assert_eq!(laid_out.index(text, [caret.x, y]), index, "{align:?}");
            }
        }

        let source = "e\u{301}x";
        let laid_out = layout(source, &options(300.0)).expect("layout");
        let caret = laid_out.caret(source, 1);
        let y = (caret.top + caret.bottom) / 2.0;
        assert_eq!(laid_out.index(source, [caret.x, y]), 2);
    }

    #[test]
    fn word_ranges_follow_unicode_boundaries_and_clamp() {
        let cases = [
            ("VibeCut", 3, 0..7),
            ("hello world", 8, 6..11),
            ("Hi, there", 2, 2..3),
            ("a\nb", 1, 1..2),
            ("Tiếng Việt", 2, 0..5),
            ("", 0, 0..0),
            ("hello", 99, 0..5),
        ];
        for (text, index, expected) in cases {
            assert_eq!(word_range(text, index), expected, "{text:?} {index}");
        }
    }

    #[test]
    fn caret_follows_alignment_and_clamps_out_of_range() {
        for align in [Align::Left, Align::Center, Align::Right] {
            let laid_out = layout(
                "AV",
                &LayoutOptions {
                    align,
                    ..options(300.0)
                },
            )
            .expect("layout");
            let line = &laid_out.lines[0];
            let end = laid_out.caret("AV", 9);
            assert_eq!(end, laid_out.caret("AV", 2), "{align:?}");
            assert!((end.x - (line.x + line.width)).abs() < 0.011, "{align:?}");
            assert!(line.x >= 0.0 && line.x + line.width <= 300.01, "{align:?}");
        }
        let centered = layout(
            "AV",
            &LayoutOptions {
                align: Align::Center,
                ..options(300.0)
            },
        )
        .expect("layout");
        let right = layout(
            "AV",
            &LayoutOptions {
                align: Align::Right,
                ..options(300.0)
            },
        )
        .expect("layout");
        assert!(right.lines[0].x > centered.lines[0].x);
    }

    #[test]
    fn caret_lands_after_a_combined_cluster_and_on_an_empty_line() {
        let source = "e\u{301}x";
        let laid_out = layout(source, &options(300.0)).expect("layout");
        let after_base = laid_out.caret(source, 1);
        let after_mark = laid_out.caret(source, 2);
        let x = laid_out
            .glyphs
            .iter()
            .find(|glyph| glyph.cluster >= 2)
            .map(|glyph| glyph.x)
            .expect("x glyph");
        assert!((after_base.x - x).abs() < 0.011);
        assert!((after_mark.x - x).abs() < 0.011);
        assert!(after_base.x > laid_out.caret(source, 0).x);

        let empty = layout("", &options(300.0)).expect("layout");
        let caret = empty.caret("", 0);
        assert_eq!(caret.line, 0);
        assert_eq!(caret.x, 0.0);
        assert!(caret.bottom > caret.top);
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
