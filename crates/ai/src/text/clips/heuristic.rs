//! Local heuristic clip proposals that need no remote provider.
use super::{
    ClipInput, ClipPiece, ClipProposal, ClipRole, ClipSource, ClipSuggester, HEURISTIC_SETTINGS,
    keyword_density, prepare, sentence,
};
use crate::Location;
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, ProviderConfig, Request, Scope, Stage,
};
/// Explicitly selected local provider. Sentence boundaries use transcript segment
/// edges; it never invents word timing or cuts inside a segment.
pub struct HeuristicClipSuggester {
    config: ProviderConfig,
}
impl HeuristicClipSuggester {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            config: ProviderConfig::new(
                "heuristic".into(),
                "sentence-pause-keyword".into(),
                HEURISTIC_SETTINGS.into(),
                Location::Local {
                    backend: "heuristic".into(),
                    model_ref: "built-in".into(),
                },
            )?,
        })
    }
}
impl ClipSuggester for HeuristicClipSuggester {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        super::remote::capabilities(&self.config, &scope.source_language)
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn source(&self) -> ClipSource {
        ClipSource::Heuristic
    }
    fn suggest(
        &self,
        window: &ClipInput,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<ClipProposal>, Error> {
        prepare(self, window, request)?;
        let mut proposals = Vec::new();
        let segments = &window.segments;
        for (start, first) in segments.iter().enumerate() {
            crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
            let mut best: Option<ClipProposal> = None;
            let mut text = String::new();
            for (end, last) in segments.iter().enumerate().skip(start) {
                let duration = last.range.end().0 - first.range.start().0;
                if duration > window.settings.max_duration.0 {
                    break;
                }
                text.push_str(last.text.as_str());
                text.push(' ');
                if duration
                    < window
                        .settings
                        .min_duration
                        .0
                        .max(window.settings.min_segment_duration.0)
                {
                    continue;
                }
                let sentence_end = sentence(last.text.as_str());
                let starts_sentence = start == 0
                    || segments
                        .get(start - 1)
                        .is_some_and(|s| sentence(s.text.as_str()));
                let pause = segments.get(end + 1).is_some_and(|next| {
                    next.range.start().0.saturating_sub(last.range.end().0)
                        >= time::TICKS_PER_SECOND / 2
                });
                let density = keyword_density(&window.settings.keywords, &text);
                let score = 0.15
                    + if starts_sentence { 0.2 } else { 0.0 }
                    + if sentence_end { 0.3 } else { 0.0 }
                    + if pause { 0.15 } else { 0.0 }
                    + density * 0.2;
                if best.as_ref().is_none_or(|p| p.score < score) {
                    // The opening sentence, trimmed; a bounded generic label avoids a long title.
                    let opening = first_sentence(first.text.as_str());
                    let title = if opening.len() <= 2048 {
                        opening
                    } else {
                        "Local clip"
                    };
                    best = Some(ClipProposal {
                        segments: vec![ClipPiece {
                            start_segment_id: first.id.clone(),
                            end_segment_id: last.id.clone(),
                            role: ClipRole::Payoff,
                        }],
                        title: title.into(),
                        hook_text: title.into(),
                        reason:
                            "Local sentence boundaries, pauses and keyword density; ranking only"
                                .into(),
                        score,
                    });
                }
            }
            if let Some(p) = best {
                proposals.push(p);
            }
        }
        Ok(proposals)
    }
}
/// The text through its first sentence-ending mark, or the whole text when none ends it.
fn first_sentence(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|(_, c)| matches!(*c, '.' | '?' | '!' | '。' | '？' | '！'))
        .map_or(text.len(), |(i, c)| i + c.len_utf8());
    text.get(..end).unwrap_or(text).trim()
}
