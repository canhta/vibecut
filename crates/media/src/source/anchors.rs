//! Retains runtime-only decoded-audio sample anchors for repeated source analysis.

use std::collections::BTreeMap;
use time::Tick;

// Runtime-only, source-content-bound state; clones share it, persistence and
// relinking start cold. No codec handles or lock are retained across I/O.
#[derive(Default)]
pub(crate) struct MfccAnchors {
    entries: BTreeMap<i64, MfccAnchor>,
    spacing: i64,
}
#[derive(Clone, Copy)]
pub(crate) struct MfccAnchor {
    pub(crate) index: i64,
    pub(crate) tick: Tick,
    pub(crate) position: isize,
    pub(crate) ordinal: usize,
    pub(crate) samples: usize,
}
impl MfccAnchors {
    pub(crate) fn resume(&self, target: i64) -> (Option<MfccAnchor>, Option<MfccAnchor>) {
        let resume = self
            .entries
            .range(..=target)
            .next_back()
            .map(|(_, anchor)| *anchor)
            .filter(|anchor| anchor.index > 0);
        let seek = resume
            .and_then(|anchor| {
                self.entries
                    .range(..anchor.index)
                    .next_back()
                    .map(|(_, earlier)| *earlier)
            })
            .filter(|anchor| anchor.index > 0);
        (resume, seek)
    }

    pub(crate) fn record(&mut self, anchor: MfccAnchor, rate: u32) {
        self.spacing = self.spacing.max(i64::from(rate));
        if self
            .entries
            .range(..=anchor.index)
            .next_back()
            .is_some_and(|(index, _)| anchor.index - index < self.spacing)
        {
            return;
        }
        if self.entries.len() >= 4096 {
            // Thin uniformly rather than evicting the early-source anchors:
            // ordered requests retain linear total work on long sources.
            self.spacing = self.spacing.saturating_mul(2);
            let mut keep = true;
            self.entries.retain(|_, _| {
                let result = keep;
                keep = !keep;
                result
            });
        }
        self.entries.insert(anchor.index, anchor);
    }
}
