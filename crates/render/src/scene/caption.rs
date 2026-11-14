//! Resolves caption presentation from verified source text.

use super::{Fit, Placement, Rect};
use crate::{Error, Result};
use captions::{
    CaptionBackground, CaptionOutline, CaptionPosition, CaptionStyle, CueStyle, VerticalAlignment,
};
use project::{TextAlign, TextDecoration};

/// ASS records no outline width in the cue style table, so an imported outline
/// keeps the default caption thickness instead of inventing one.
const IMPORTED_OUTLINE_THICKNESS: f32 = 20.0;
/// Per-side ceiling shared with [`CaptionStyle::safe_area_percent`], so a hostile
/// margin can never produce an empty destination.
const MAX_CUE_MARGIN: f64 = 0.4;

/// Project settings with one imported cue's static style applied. A cue
/// without a style keeps the settings untouched.
pub(crate) fn for_cue(base: &CaptionStyle, cue: Option<&CueStyle>) -> CaptionStyle {
    let Some(cue) = cue else {
        return base.clone();
    };
    CaptionStyle {
        font: cue.font_family.as_str().to_owned(),
        font_style: text::style_name(if cue.bold { 700 } else { 400 }, cue.italic),
        // The stored ratio is the ASS font size over PlayResY; caption sizes
        // are pixels at 1080 output lines.
        size: cue.font_size_ratio * 1080.0,
        color: cue.primary_color,
        outline: (cue.border_style != 3).then_some(CaptionOutline {
            color: cue.outline_color,
            thickness: IMPORTED_OUTLINE_THICKNESS,
        }),
        background: (cue.border_style == 3).then_some(CaptionBackground {
            color: cue.background_color,
            padding: 0.0,
            radius: 0.0,
        }),
        position: match cue.alignment.vertical {
            VerticalAlignment::Top => CaptionPosition::Top,
            VerticalAlignment::Middle => CaptionPosition::Middle,
            VerticalAlignment::Bottom => CaptionPosition::Bottom,
        },
        safe_area_percent: base.safe_area_percent,
        max_lines: cue.max_lines.map_or(base.max_lines, usize::from),
        word_highlight_color: cue
            .word_highlight_color
            .unwrap_or(base.word_highlight_color),
    }
}

/// Complete style for saving one uniform imported or materialized cue value.
pub fn style_from_cue(cue: &CueStyle) -> CaptionStyle {
    for_cue(&CaptionStyle::default(), Some(cue))
}

/// Effective full style of a caption item that selected a live saved preset.
/// `None` keeps the transient project settings and per-cue imported values.
pub(crate) fn item_style(
    data: &project::ProjectData,
    params: &project::CaptionParams,
) -> Option<CaptionStyle> {
    let id = params.preset?;
    let mut style = data
        .caption_presets
        .iter()
        .find(|preset| preset.id == id)?
        .style
        .clone();
    params.overrides.apply(&mut style);
    Some(style)
}

/// Destination band for one cue. A styled cue keeps its per-side ASS margins;
/// an unstyled cue keeps the uniform safe-area inset. Both share the same
/// bounded line-height band.
pub(crate) fn cue_placement(style: &CaptionStyle, cue: Option<&CueStyle>) -> Placement {
    let (left, right, vertical) = cue.map_or_else(
        || {
            let inset = f64::from(style.safe_area_percent) / 100.0;
            (inset, inset, inset)
        },
        |cue| {
            (
                bounded_margin(cue.margin_left_ratio),
                bounded_margin(cue.margin_right_ratio),
                bounded_margin(cue.margin_vertical_ratio),
            )
        },
    );
    let decoration = style
        .outline
        .as_ref()
        .map_or(0.0, |o| captions::outline_width(o.thickness, style.size))
        .max(style.background.as_ref().map_or(0.0, |b| b.padding));
    let height = ((f64::from(style.size) * style.max_lines as f64 * 2.0
        + f64::from(decoration) * 2.0
        + 4.0)
        / 1080.0)
        .min(1.0 - 2.0 * vertical);
    let y = match style.position {
        CaptionPosition::Top => vertical,
        CaptionPosition::Middle => (1.0 - height) / 2.0,
        CaptionPosition::Bottom => 1.0 - vertical - height,
    };
    Placement {
        destination: Rect {
            x: left,
            y,
            width: 1.0 - left - right,
            height,
        },
        fit: Fit::Stretch,
        ..Default::default()
    }
}

pub(crate) fn validate(style: &CaptionStyle) -> Result<()> {
    if style.is_valid() {
        Ok(())
    } else {
        Err(Error::Invalid("caption style limits"))
    }
}

fn bounded_margin(ratio: f32) -> f64 {
    f64::from(ratio).clamp(0.0, MAX_CUE_MARGIN)
}

/// Horizontal placement of a cue's text inside its band.
pub(super) fn align(cue: Option<&CueStyle>) -> TextAlign {
    match cue.map(|cue| cue.alignment.horizontal) {
        Some(captions::HorizontalAlignment::Left) => TextAlign::Left,
        Some(captions::HorizontalAlignment::Right) => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

/// The one line a cue style draws; underline wins when both are set.
pub(super) fn decoration(cue: Option<&CueStyle>) -> TextDecoration {
    match cue {
        Some(cue) if cue.underline => TextDecoration::Underline,
        Some(cue) if cue.strikeout => TextDecoration::LineThrough,
        _ => TextDecoration::None,
    }
}

/// Exact source byte correspondence for one displayed cue. Preflight and pixels
/// consume the same plan; a token count is never used as alignment evidence.
pub(super) struct WordPlan {
    pub spans: Vec<(Option<time::SourceRange>, usize, usize)>,
    /// Canonical byte spans of the cue's stored emphasis words that are visible, ascending.
    pub emphasis: Vec<(usize, usize)>,
    pub reason: Option<captions::WordHighlightReason>,
}
pub(super) fn word_plan(
    params: &project::CaptionParams,
    cue: &captions::Cue,
    languages: &[captions::LanguageTag],
    ambiguous: bool,
) -> WordPlan {
    use captions::{CueWordTiming, WordHighlightReason as Reason};
    let refused = |reason| WordPlan {
        spans: Vec::new(),
        emphasis: Vec::new(),
        reason: Some(reason),
    };
    if !params.word_highlight {
        return WordPlan {
            spans: Vec::new(),
            emphasis: Vec::new(),
            reason: None,
        };
    }
    let CueWordTiming::Words {
        language,
        text,
        records,
    } = &cue.word_timing
    else {
        return refused(Reason::MissingTiming);
    };
    if language.primary() != cue.source_lang.primary() {
        return refused(Reason::SourceChanged);
    }
    let displayed: Vec<_> = if languages.is_empty() {
        cue.texts
            .get_key_value(&cue.source_lang)
            .or_else(|| cue.texts.first_key_value())
            .into_iter()
            .collect()
    } else {
        languages
            .iter()
            .filter_map(|language| cue.texts.get_key_value(language))
            .collect()
    };
    let Some((index, (_, source))) = displayed
        .iter()
        .enumerate()
        .find(|(_, (tag, _))| *tag == language)
    else {
        return refused(Reason::SourceNotDisplayed);
    };
    if source.edited || source.translation_ref.is_some() || source.text != *text {
        return refused(Reason::SourceChanged);
    }
    if records.is_empty() {
        return refused(Reason::MissingTiming);
    }
    if ambiguous {
        return refused(Reason::AmbiguousTiming);
    }
    let mut cursor = 0;
    let mut previous = None;
    for word in records {
        let range = match word.projection {
            captions::WordProjection::Visible(range) => Some(range),
            captions::WordProjection::Clipped => None,
        };
        if word.bytes.start < cursor
            || text
                .as_str()
                .get(cursor..word.bytes.start)
                .is_none_or(|gap| !gap.trim().is_empty())
            || text
                .as_str()
                .get(word.bytes.clone())
                .is_none_or(|value| value.trim().is_empty())
            || range.is_some_and(|range| {
                range.duration() > word.source_range.duration()
                    || previous.is_some_and(|end| end > range.start())
            })
        {
            return refused(Reason::AmbiguousTiming);
        }
        cursor = word.bytes.end;
        if let Some(range) = range {
            previous = Some(range.end());
        }
    }
    if text
        .as_str()
        .get(cursor..)
        .is_none_or(|tail| !tail.trim().is_empty())
    {
        return refused(Reason::SourceChanged);
    }
    let mut prefix = String::new();
    for (_, prior) in displayed.iter().take(index) {
        if prior.text.as_str().trim().is_empty() {
            continue;
        }
        if !prefix.is_empty() {
            prefix.push('\n');
        }
        prefix.push_str(prior.text.as_str());
    }
    if !prefix.is_empty() {
        prefix.push('\n');
    }
    let offset = text::canonical_text(&prefix).len();
    let Ok(boundaries) = text::canonical_boundaries(text.as_str()) else {
        return refused(Reason::ShapedBoundary);
    };
    let mut spans = Vec::with_capacity(records.len());
    for word in records {
        let Ok(start) = boundaries.binary_search_by_key(&word.bytes.start, |&(source, _)| source)
        else {
            return refused(Reason::ShapedBoundary);
        };
        let Ok(end) = boundaries.binary_search_by_key(&word.bytes.end, |&(source, _)| source)
        else {
            return refused(Reason::ShapedBoundary);
        };
        let (Some(&(_, start)), Some(&(_, end))) = (boundaries.get(start), boundaries.get(end))
        else {
            return refused(Reason::ShapedBoundary);
        };
        spans.push((
            match word.projection {
                captions::WordProjection::Visible(range) => Some(range),
                captions::WordProjection::Clipped => None,
            },
            offset + start,
            offset + end,
        ));
    }
    // Emphasis rides the same verified spans: stored ids that no longer match the cue's
    // words resolve to nothing, so a stale selection is never painted.
    let stored = params.emphasis.get(&cue.id).map_or(&[][..], Vec::as_slice);
    let emphasis = captions::resolve_selection(cue, stored)
        .iter()
        .filter_map(|&id| spans.get(usize::try_from(id).ok()?))
        .filter(|(range, _, _)| range.is_some())
        .map(|&(_, start, end)| (start, end))
        .collect();
    WordPlan {
        spans,
        emphasis,
        reason: None,
    }
}

pub(super) fn display_text(
    cue: &captions::Cue,
    languages: &[captions::LanguageTag],
    hooks: &dyn crate::ExecutionHooks,
) -> Result<(String, Box<dyn crate::Reservation>)> {
    let selected: Vec<_> = if languages.is_empty() {
        cue.texts
            .get(&cue.source_lang)
            .or_else(|| cue.texts.values().next())
            .into_iter()
            .collect()
    } else {
        languages
            .iter()
            .map(|language| {
                cue.texts
                    .get(language)
                    .ok_or(Error::Unsupported("requested caption language unavailable"))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let bytes = selected
        .iter()
        .filter(|value| !value.text.as_str().trim().is_empty())
        .try_fold(0usize, |sum, value| {
            sum.checked_add(value.text.as_str().len() + 1)
        })
        .ok_or(Error::Invalid("caption text bytes"))?;
    if bytes > 16384 {
        return Err(Error::Invalid("render caption byte limit"));
    }
    let joined = hooks.reserve(crate::MemoryKind::Compositor, bytes as u64)?;
    let mut value = String::new();
    value.try_reserve_exact(bytes)?;
    for text in selected
        .into_iter()
        .filter(|value| !value.text.as_str().trim().is_empty())
    {
        if !value.is_empty() {
            value.push('\n');
        }
        value.push_str(text.text.as_str());
    }
    Ok((value, joined))
}

/// Mark every member of an overlapping interval set in O(n log n), retaining
/// half-open touching boundaries. Preflight calls this once for the captured item.
pub(super) fn ambiguous_cues(params: &project::CaptionParams) -> Vec<bool> {
    let mut intervals: Vec<_> = params
        .cues
        .cues
        .iter()
        .enumerate()
        .map(|(index, cue)| (cue.range.start(), cue.range.end(), index))
        .collect();
    intervals.sort_unstable();
    let mut result = vec![false; intervals.len()];
    let mut furthest: Option<(time::Tick, usize)> = None;
    for (start, end, index) in intervals {
        if let Some((prior, owner)) = furthest {
            if start < prior {
                if let Some(value) = result.get_mut(index) {
                    *value = true;
                }
                if let Some(value) = result.get_mut(owner) {
                    *value = true;
                }
            }
            if end <= prior {
                continue;
            }
        }
        furthest = Some((end, index));
    }
    result
}
