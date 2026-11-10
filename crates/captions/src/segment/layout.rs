//! Fits speech units into the configured lines.

use super::units::Unit;
use super::{LineLimit, SegmentRules};
use unicode_segmentation::UnicodeSegmentation;

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
    pub(super) fn push(&mut self, unit: &Unit, rules: &SegmentRules) {
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
        let LineLimit::Graphemes(max) = rules.line_limit;
        let fits = candidate_count as u128 <= u128::from(max);
        if previous_len > 0 && !fits {
            self.current.truncate(previous_len);
            self.completed.push(std::mem::take(&mut self.current));
            self.current.push_str(value);
            self.graphemes = count;
            self.too_wide |= count as u128 > u128::from(max);
        } else {
            self.graphemes = candidate_count;
            self.too_wide |= !fits;
        }
        self.pending_space = unit.text.ends_with(char::is_whitespace);
    }
    pub(super) fn overflow(&self, rules: &SegmentRules) -> bool {
        self.too_wide
            || (self.completed.len() as u128 + u128::from(!self.current.is_empty()))
                > u128::from(rules.max_lines)
    }
}
pub(super) fn layout(units: &[Unit], rules: &SegmentRules) -> (String, bool) {
    let mut lines = Lines::default();
    for unit in units {
        lines.push(unit, rules);
    }
    let overflow = lines.overflow(rules);
    if !lines.current.is_empty() {
        lines.completed.push(lines.current.trim().to_owned());
    }
    (lines.completed.join("\n"), overflow)
}
