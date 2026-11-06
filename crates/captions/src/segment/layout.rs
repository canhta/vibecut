//! Fits speech units into the configured lines.

use super::units::Unit;
use super::{LayoutHooks, LineLimit, SegmentRules};
use crate::Error;
use unicode_segmentation::UnicodeSegmentation;

fn measure(text: &str, hooks: &LayoutHooks<'_>) -> Result<f64, Error> {
    let width = hooks
        .measure
        .ok_or(Error::Invalid("missing text measurement"))?(text)?;
    if !width.is_finite() || width < 0.0 {
        return Err(Error::Invalid("text measurement"));
    }
    Ok(width)
}

// Layout is incremental over legal units, including long indivisible words.
// Those words remain intact and produce TextLayout warnings instead of truncation.
#[derive(Default)]
pub(super) struct Lines {
    completed: Vec<String>,
    current: String,
    too_wide: bool,
    graphemes: usize,
    pending_space: bool,
}
impl Lines {
    pub(super) fn push(
        &mut self,
        unit: &Unit,
        rules: &SegmentRules,
        hooks: &LayoutHooks<'_>,
    ) -> Result<(), Error> {
        let value = unit.text.trim();
        let count = value.graphemes(true).count();
        let separator = !self.current.is_empty()
            && (self.pending_space || unit.text.starts_with(char::is_whitespace));
        let previous_len = self.current.len();
        let candidate_count = self.graphemes + usize::from(separator) + count;
        if separator {
            self.current.push(' ');
        }
        self.current.push_str(value);
        let fits = match rules.line_limit {
            LineLimit::Graphemes(max) => candidate_count as u128 <= u128::from(max),
            LineLimit::Pixels(max) => measure(&self.current, hooks)? <= max,
        };
        if previous_len > 0 && !fits {
            self.current.truncate(previous_len);
            self.completed.push(std::mem::take(&mut self.current));
            self.current.push_str(value);
            self.graphemes = count;
            self.too_wide |= match rules.line_limit {
                LineLimit::Graphemes(max) => count as u128 > u128::from(max),
                LineLimit::Pixels(max) => measure(value, hooks)? > max,
            };
        } else {
            self.graphemes = candidate_count;
            self.too_wide |= !fits;
        }
        self.pending_space = unit.text.ends_with(char::is_whitespace);
        Ok(())
    }
    pub(super) fn overflow(&self, rules: &SegmentRules) -> bool {
        self.too_wide
            || (self.completed.len() as u128 + u128::from(!self.current.is_empty()))
                > u128::from(rules.max_lines)
    }
}
pub(super) fn layout(
    units: &[Unit],
    rules: &SegmentRules,
    hooks: &LayoutHooks<'_>,
) -> Result<(String, bool), Error> {
    let mut lines = Lines::default();
    for unit in units {
        lines.push(unit, rules, hooks)?;
    }
    let overflow = lines.overflow(rules);
    if !lines.current.is_empty() {
        lines.completed.push(lines.current.trim().to_owned());
    }
    Ok((lines.completed.join("\n"), overflow))
}
