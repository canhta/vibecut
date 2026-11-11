//! Metric definitions for the labelled clip pipeline.
use crate::label::{LabelledClip, Priority, ResolvedSettings};
use ai::clips::ClipSourceSegment;
use captions::{LanguageProfile, Transcript, source_speech_units};
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick};

/// Summed source-segment IoU: pairwise intersections over summed source
/// durations minus those intersections. Gaps never contribute and an enclosing
/// range is never treated as a match for disjoint pieces.
pub fn source_iou(label: &[SourceRange], suggestion: &[SourceRange]) -> f64 {
    let mut intersection = 0i128;
    for piece in label {
        for other in suggestion {
            if let Some(overlap) = piece.intersect(*other) {
                intersection = intersection.saturating_add(i128::from(overlap.duration().0));
            }
        }
    }
    let label_sum: i128 = label
        .iter()
        .map(|range| i128::from(range.duration().0))
        .sum();
    let suggestion_sum: i128 = suggestion
        .iter()
        .map(|range| i128::from(range.duration().0))
        .sum();
    let union = label_sum
        .saturating_add(suggestion_sum)
        .saturating_sub(intersection);
    if union <= 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

pub fn clip_iou(label: &LabelledClip, suggestion: &[ClipSourceSegment]) -> f64 {
    let label_ranges: Vec<_> = label
        .segments
        .iter()
        .filter_map(|segment| SourceRange::new(segment.start, segment.end).ok())
        .collect();
    let suggestion_ranges: Vec<_> = suggestion.iter().map(|segment| segment.range).collect();
    source_iou(&label_ranges, &suggestion_ranges)
}

pub fn is_match(iou: f64) -> bool {
    iou >= 0.5
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metrics {
    pub suggestions: usize,
    pub must_total: usize,
    pub must_hit: usize,
    pub hit_rate: f64,
    pub precision_k: usize,
    pub precision_matches: usize,
    pub precision_at_k: f64,
    pub boundary_edges: usize,
    pub boundary_bad_edges: usize,
    pub boundary_quality: Option<f64>,
    pub duration_compliant: usize,
    pub duration_compliance: f64,
    pub snap_failures: usize,
    pub cost_micros: Option<u64>,
    pub wall_ms: u64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            suggestions: 0,
            must_total: 0,
            must_hit: 0,
            hit_rate: 0.0,
            precision_k: 0,
            precision_matches: 0,
            precision_at_k: 0.0,
            boundary_edges: 0,
            boundary_bad_edges: 0,
            boundary_quality: None,
            duration_compliant: 0,
            duration_compliance: 0.0,
            snap_failures: 0,
            cost_micros: Some(0),
            wall_ms: 0,
        }
    }
}

impl Metrics {
    pub fn merge(&mut self, other: &Self) {
        self.suggestions = self.suggestions.saturating_add(other.suggestions);
        self.must_total = self.must_total.saturating_add(other.must_total);
        self.must_hit = self.must_hit.saturating_add(other.must_hit);
        self.precision_k = self.precision_k.saturating_add(other.precision_k);
        self.precision_matches = self
            .precision_matches
            .saturating_add(other.precision_matches);
        self.boundary_edges = self.boundary_edges.saturating_add(other.boundary_edges);
        self.boundary_bad_edges = self
            .boundary_bad_edges
            .saturating_add(other.boundary_bad_edges);
        self.duration_compliant = self
            .duration_compliant
            .saturating_add(other.duration_compliant);
        self.snap_failures = self.snap_failures.saturating_add(other.snap_failures);
        self.cost_micros = self
            .cost_micros
            .zip(other.cost_micros)
            .map(|(a, b)| a.saturating_add(b));
        self.wall_ms = self.wall_ms.saturating_add(other.wall_ms);
        self.recompute();
    }

    pub fn recompute(&mut self) {
        self.hit_rate = ratio(self.must_hit, self.must_total);
        self.precision_at_k = ratio(self.precision_matches, self.precision_k);
        self.boundary_quality = if self.boundary_edges == 0 {
            None
        } else {
            Some(ratio(
                self.boundary_edges.saturating_sub(self.boundary_bad_edges),
                self.boundary_edges,
            ))
        };
        self.duration_compliance = ratio(self.duration_compliant, self.suggestions);
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// One provider run to score.
pub struct Evaluation<'a> {
    pub labels: &'a [LabelledClip],
    pub suggestions: &'a [Vec<ClipSourceSegment>],
    pub transcript: &'a Transcript,
    pub profile: &'a LanguageProfile,
    pub settings: &'a ResolvedSettings,
    pub k: usize,
    pub cost_micros: Option<u64>,
    pub wall_ms: u64,
    pub snap_failures: usize,
}

/// Evaluate one provider's suggestions. `suggestions` are already in rank order.
pub fn evaluate(input: &Evaluation<'_>) -> Result<Metrics, captions::Error> {
    let Evaluation {
        labels,
        suggestions,
        transcript,
        profile,
        settings,
        k,
        cost_micros,
        wall_ms,
        snap_failures,
    } = input;
    let mut metrics = Metrics {
        suggestions: suggestions.len(),
        precision_k: (*k).min(suggestions.len()),
        cost_micros: *cost_micros,
        wall_ms: *wall_ms,
        snap_failures: *snap_failures,
        ..Metrics::default()
    };
    for label in labels.iter().filter(|l| l.priority == Priority::Must) {
        metrics.must_total = metrics.must_total.saturating_add(1);
        if suggestions
            .iter()
            .any(|suggestion| is_match(clip_iou(label, suggestion)))
        {
            metrics.must_hit = metrics.must_hit.saturating_add(1);
        }
    }
    for suggestion in suggestions.iter().take(*k) {
        if labels
            .iter()
            .any(|label| is_match(clip_iou(label, suggestion)))
        {
            metrics.precision_matches = metrics.precision_matches.saturating_add(1);
        }
    }
    let units = source_speech_units(transcript, profile)?;
    let (words, sentences) = spans(&units)?;
    for suggestion in *suggestions {
        let total: i128 = suggestion
            .iter()
            .map(|segment| i128::from(segment.range.duration().0))
            .sum();
        let compliant = suggestion.len() <= settings.max_segments
            && total >= i128::from(settings.min_duration.0)
            && total <= i128::from(settings.max_duration.0)
            && suggestion
                .iter()
                .all(|segment| segment.range.duration() >= settings.min_segment_duration);
        if compliant {
            metrics.duration_compliant = metrics.duration_compliant.saturating_add(1);
        }
        for segment in suggestion {
            for edge in [segment.range.start(), segment.range.end()] {
                metrics.boundary_edges = metrics.boundary_edges.saturating_add(1);
                if inside(&words, edge) || inside(&sentences, edge) {
                    metrics.boundary_bad_edges = metrics.boundary_bad_edges.saturating_add(1);
                }
            }
        }
    }
    metrics.recompute();
    Ok(metrics)
}

fn spans(
    units: &[captions::SourceSpeechUnit],
) -> Result<(Vec<SourceRange>, Vec<SourceRange>), captions::Error> {
    let words: Vec<_> = units.iter().map(|unit| unit.range).collect();
    let mut sentences = Vec::new();
    let mut start: Option<Tick> = None;
    let mut end = Tick::ZERO;
    for unit in units {
        if start.is_none() {
            start = Some(unit.range.start());
        }
        end = unit.range.end();
        if unit.sentence_end
            && let Some(first) = start.take()
        {
            sentences.push(SourceRange::new(first, end)?);
        }
    }
    if let Some(first) = start {
        sentences.push(SourceRange::new(first, end)?);
    }
    Ok((words, sentences))
}

fn inside(ranges: &[SourceRange], edge: Tick) -> bool {
    ranges
        .iter()
        .any(|range| range.start() < edge && edge < range.end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::LabelSegment;
    use time::{SourceRange, TICKS_PER_SECOND, Tick};

    fn second(value: i64) -> Tick {
        Tick(value * TICKS_PER_SECOND)
    }
    fn range(start: i64, end: i64) -> SourceRange {
        SourceRange::new(second(start), second(end)).unwrap()
    }
    fn label(segments: &[(i64, i64)]) -> LabelledClip {
        LabelledClip {
            id: "l".into(),
            priority: Priority::Must,
            segments: segments
                .iter()
                .map(|(start, end)| LabelSegment {
                    start: second(*start),
                    end: second(*end),
                })
                .collect(),
        }
    }

    #[test]
    fn exact_partial_none_and_reordered_lists_have_expected_iou() {
        let exact = vec![range(0, 10)];
        assert_eq!(source_iou(&exact, &exact), 1.0);
        assert_eq!(source_iou(&exact, &[range(0, 5)]), 0.5);
        assert_eq!(source_iou(&exact, &[range(10, 20)]), 0.0);
        let reordered = vec![range(20, 30), range(0, 10)];
        assert_eq!(source_iou(&exact, &reordered), 0.5);
        let same_pieces = vec![range(20, 30), range(0, 10)];
        assert_eq!(
            source_iou(&[range(0, 10), range(20, 30)], &same_pieces),
            1.0
        );
    }

    #[test]
    fn disjoint_label_pieces_are_not_matched_by_their_envelope() {
        let pieces = [range(0, 5), range(20, 25)];
        let envelope = [range(0, 25)];
        let iou = source_iou(&pieces, &envelope);
        assert!((iou - 10.0 / 25.0).abs() < 1e-9, "{iou}");
        assert!(!is_match(iou));
    }

    #[test]
    fn overlapping_suggestions_are_counted_independently() {
        let suggestions = [range(0, 10), range(0, 8), range(20, 30)];
        let matched: Vec<_> = suggestions
            .iter()
            .map(|suggestion| is_match(source_iou(&[range(0, 10)], &[*suggestion])))
            .collect();
        assert_eq!(matched, vec![true, true, false]);
        let label = label(&[(0, 10)]);
        assert!(is_match(source_iou(&[range(0, 10)], &[range(0, 8)])));
        assert_eq!(label.segments.len(), 1);
    }

    #[test]
    fn boundary_quality_scores_words_and_sentences() {
        use ai::clips::{ClipRole, ClipSourceSegment};
        use captions::{Fingerprint, LanguageRegistry, LanguageTag, NfcText, Segment, Word};
        let transcript = Transcript {
            source_lang: LanguageTag::parse("en").unwrap(),
            fingerprint: Fingerprint {
                provider: "fixture".into(),
                model: "fixture".into(),
                settings: "fixture".into(),
            },
            segments: vec![Segment {
                id: "s0".into(),
                range: range(0, 10),
                text: "hello world again.".into(),
                words: vec![
                    Word {
                        text: NfcText::from("hello"),
                        range: Some(range(0, 2)),
                        speaker: None,
                        confidence: None,
                    },
                    Word {
                        text: NfcText::from("world"),
                        range: Some(range(2, 4)),
                        speaker: None,
                        confidence: None,
                    },
                    Word {
                        text: NfcText::from("again."),
                        range: Some(range(4, 6)),
                        speaker: None,
                        confidence: None,
                    },
                ],
                spans: None,
            }],
        };
        let registry = LanguageRegistry::standard().unwrap();
        let language = LanguageTag::parse("en").unwrap();
        let profile = registry.profile(&language);
        let settings = crate::label::Settings {
            min_duration_seconds: 1.0,
            max_duration_seconds: 60.0,
            ..crate::label::Settings::default()
        }
        .resolve()
        .unwrap();
        let segment = |start, end| ClipSourceSegment {
            range: range(start, end),
            role: ClipRole::Payoff,
            source_order: 0,
            repair: None,
        };
        let suggestions = vec![
            vec![segment(0, 6)],
            vec![segment(2, 4)],
            vec![segment(1, 3)],
        ];
        let metrics = evaluate(&Evaluation {
            labels: &[],
            suggestions: &suggestions,
            transcript: &transcript,
            profile,
            settings: &settings,
            k: 5,
            cost_micros: Some(0),
            wall_ms: 0,
            snap_failures: 0,
        })
        .unwrap();
        assert_eq!(metrics.boundary_edges, 6);
        assert_eq!(metrics.boundary_bad_edges, 4);
        assert!((metrics.boundary_quality.unwrap() - 2.0 / 6.0).abs() < 1e-9);
    }
}
