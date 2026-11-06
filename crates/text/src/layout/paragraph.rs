//! Turns shaped text runs into positioned lines.

use super::breaks::{grapheme_boundaries, line_ranges, range_width, trim_end};
use super::{Align, Bounds, Glyph, Layout, LayoutOptions, Line, TextError};
use crate::fonts::{self, FaceId, Resolved};
use harfrust::{Buffer, Direction, Feature, ShapeOptions, ShaperFont, Tag, shape};
use std::collections::VecDeque;
use std::ops::Range;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug)]
pub(super) struct ShapedGlyph {
    pub(super) face: FaceId,
    pub(super) id: u32,
    pub(super) cluster: usize,
    pub(super) advance: f32,
    pub(super) offset_x: f32,
    pub(super) offset_y: f32,
    pub(super) synth_bold: bool,
    pub(super) synth_italic: bool,
}

pub(super) struct ParagraphContext<'a, 'b> {
    pub(super) primary: Resolved,
    pub(super) options: &'a LayoutOptions<'b>,
    pub(super) fallbacks: &'a [fonts::FaceId],
    pub(super) baseline: f32,
    pub(super) first_line: bool,
}

pub(super) fn append_paragraph(
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
        options.letter_spacing,
        context.fallbacks,
        false,
    )?;
    let nominal = shape_paragraph(
        paragraph,
        base,
        context.primary,
        options.size,
        options.letter_spacing,
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
                options.letter_spacing,
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
    spacing: f32,
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
    if spacing != 0.0 {
        // One spacing per cluster: it follows the cluster's last glyph.
        let mut later = None;
        for glyph in out.iter_mut().rev() {
            if later != Some(glyph.cluster) {
                glyph.advance += spacing;
            }
            later = Some(glyph.cluster);
        }
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
pub(super) fn canonical_text(text: &str) -> String {
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

pub(super) fn content_bounds(lines: &[Line]) -> Bounds {
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
