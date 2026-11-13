//! Maps source ticks to output ticks through edit cut lists.

use crate::{Cue, CueTrack, Error, NfcText};
use time::{SourceRange, Tick};

/// Identity of one occurrence of a source cue in an edit mapping.
/// Segment indices refer to the caller's ordered mapping segments, including
/// repeated source spans. Rebuilding with the same segment order preserves IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CueFragmentId {
    pub source_id: NfcText,
    pub segment: usize,
}

impl CueFragmentId {
    /// Encode both identities without interpreting any characters in the source ID.
    pub fn to_id(&self) -> NfcText {
        format!("{}/{}", self.segment, self.source_id.as_str()).into()
    }

    /// Recover source identity and mapping occurrence from a mapped cue's ID.
    /// Only call on IDs produced by [`EditMapping::map_cues`]. Malformed IDs return
    /// `Error::Invalid`; input text is never included in diagnostics.
    pub fn from_id(id: &NfcText) -> Result<Self, Error> {
        let (segment, source_id) = id
            .as_str()
            .split_once('/')
            .ok_or(Error::Invalid("cue fragment identity"))?;
        if segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::Invalid("cue fragment occurrence"));
        }
        Ok(Self {
            source_id: source_id.into(),
            segment: segment
                .parse()
                .map_err(|_| Error::Invalid("cue fragment occurrence"))?,
        })
    }
}

/// A retained tick with its mapping occurrence. All coordinates use integer ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappedTick {
    pub segment: usize,
    pub output: Tick,
}

/// One retained intersection; source and output ranges are equally long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappedRange {
    pub segment: usize,
    pub source: SourceRange,
    pub output: SourceRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Segment {
    source: SourceRange,
    output: SourceRange,
}

/// Pure unit-speed source-to-output mapping. Source spans may repeat or overlap;
/// output spans may also overlap. Gaps and an empty mapping are valid. No audio/stream offset, frame snapping, retiming or repair is applied.
/// Callers supply normalized source coordinates and build a new mapping after edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditMapping {
    segments: Vec<Segment>,
}

impl EditMapping {
    /// Validate `(source range, output start)` segments in occurrence order.
    /// Returns typed time errors when an output endpoint cannot be represented.
    /// Input order need not be temporal; signed coordinates are supported.
    pub fn new(segments: Vec<(SourceRange, Tick)>) -> Result<Self, Error> {
        let segments = segments
            .into_iter()
            .map(|(source, start)| {
                Ok(Segment {
                    source,
                    output: SourceRange::from_start_duration(start, source.duration())?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self { segments })
    }

    /// Return all occurrences of a source tick, in mapping order. End boundaries
    /// are excluded. No match returns an empty vector; overflow is a typed error.
    pub fn map_tick(&self, source: Tick) -> Result<Vec<MappedTick>, Error> {
        self.segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.source.contains(source))
            .map(|(segment, s)| {
                Ok(MappedTick {
                    segment,
                    output: translate(source, s.source.start(), s.output.start())?,
                })
            })
            .collect()
    }

    /// Return every retained intersection, keeping disjoint cuts and repeated
    /// occurrences separate. Uses half-open boundaries and never merges gaps.
    pub fn map_range(&self, source: SourceRange) -> Result<Vec<MappedRange>, Error> {
        self.segments
            .iter()
            .enumerate()
            .filter_map(|(segment, s)| {
                s.source.intersect(source).map(|source| {
                    Ok(MappedRange {
                        segment,
                        source,
                        output: SourceRange::new(
                            translate(source.start(), s.source.start(), s.output.start())?,
                            translate(source.end(), s.source.start(), s.output.start())?,
                        )?,
                    })
                })
            })
            .collect()
    }

    /// Invert one identified occurrence. Output in a gap, at an excluded end,
    /// or an unknown segment has no inverse. No search selects an arbitrary repeat.
    pub fn inverse_tick(&self, mapped: MappedTick) -> Result<Option<Tick>, Error> {
        let Some(s) = self
            .segments
            .get(mapped.segment)
            .filter(|s| s.output.contains(mapped.output))
        else {
            return Ok(None);
        };
        translate(mapped.output, s.output.start(), s.source.start()).map(Some)
    }

    /// Rebuild output cues from the original source track. Text, source language,
    /// translation references and per-language edits are cloned without alteration.
    /// Each fragment ID losslessly encodes `(source ID, mapping segment)` using
    /// [`CueFragmentId`]. Duplicate source IDs return `Error::Invalid`.
    ///
    /// Results stable-sort by output start; overlaps are preserved rather than
    /// shifted by subtitle repair. Always pass the source track again after an edit,
    /// never a previously mapped track. No input state is mutated on failure.
    pub fn map_cues(&self, source: &CueTrack) -> Result<CueTrack, Error> {
        self.rebuild(source, |cue, range| {
            CueFragmentId {
                source_id: cue.id.clone(),
                segment: range.segment,
            }
            .to_id()
        })
    }

    /// Shared retention rule: clone every retained cue, replace its range with the
    /// mapped output and derive its identity from the mapping occurrence.
    fn rebuild(
        &self,
        source: &CueTrack,
        identity: impl Fn(&Cue, MappedRange) -> NfcText,
    ) -> Result<CueTrack, Error> {
        let mut ids = std::collections::BTreeSet::new();
        let mut cues = Vec::new();
        let mut word_bytes = 0usize;
        for cue in &source.cues {
            if !ids.insert(cue.id.as_str()) {
                return Err(Error::Invalid("duplicate source cue identity"));
            }
            for range in self.map_range(cue.range)? {
                word_bytes = word_bytes
                    .checked_add(cue.word_timing.byte_cost()?)
                    .filter(|bytes| *bytes <= crate::timing::LIMIT)
                    .ok_or(Error::TooLarge)?;
                let mut fragment = cue.clone();
                fragment.range = range.output;
                if let crate::CueWordTiming::Words { records, .. } = &mut fragment.word_timing {
                    for word in records {
                        word.projection = if let crate::WordProjection::Visible(word_range) =
                            word.projection
                        {
                            let start = word_range.start().max(range.source.start());
                            let end = word_range.end().min(range.source.end());
                            if start < end {
                                crate::WordProjection::Visible(SourceRange::new(
                                    translate(start, range.source.start(), range.output.start())?,
                                    translate(end, range.source.start(), range.output.start())?,
                                )?)
                            } else {
                                crate::WordProjection::Clipped
                            }
                        } else {
                            crate::WordProjection::Clipped
                        };
                    }
                }
                fragment.id = identity(cue, range);
                cues.push(fragment);
            }
        }
        cues.sort_by_key(|cue| cue.range.start());
        Ok(CueTrack {
            styles: source.styles.clone(),
            cues,
        })
    }
}

/// Clip a source track to one source range. Retained cues keep their source IDs,
/// text, language, edit flags and translation references; output uses the range's
/// own coordinates, so a cue at the range start stays at the range start. Partial
/// cues clamp at the half-open edges, fully cut cues are dropped, and the result
/// stable-sorts by output start. Duplicate source IDs return `Error::Invalid`.
/// No input state is mutated on failure.
pub fn clip_track(source: &CueTrack, range: SourceRange) -> Result<CueTrack, Error> {
    EditMapping::new(vec![(range, range.start())])?.rebuild(source, |cue, _| cue.id.clone())
}

fn translate(value: Tick, from: Tick, to: Tick) -> Result<Tick, Error> {
    value
        .checked_sub(from)
        .and_then(|offset| to.checked_add(offset))
        .ok_or_else(|| time::TimeError::Overflow.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cue, CueText, Format, LanguageTag, export, parse};
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use time::{FrameRate, Rounding};

    fn range(start: i64, end: i64) -> SourceRange {
        SourceRange::new(Tick(start), Tick(end)).unwrap()
    }

    fn track(ranges: &[(i64, i64)]) -> CueTrack {
        let language = LanguageTag::parse("vi").unwrap();
        CueTrack {
            styles: Vec::new(),
            cues: ranges
                .iter()
                .enumerate()
                .map(|(i, &(start, end))| Cue {
                    word_timing: crate::CueWordTiming::Unverified,
                    id: format!("nguồn/{i}").into(),
                    range: range(start, end),
                    source_lang: language.clone(),
                    texts: BTreeMap::from([
                        (
                            language.clone(),
                            CueText {
                                text: "Tiếng Việt có dấu".into(),
                                edited: true,
                                translation_ref: None,
                            },
                        ),
                        (
                            LanguageTag::parse("en").unwrap(),
                            CueText {
                                text: "English".into(),
                                edited: false,
                                translation_ref: Some("translation/1".into()),
                            },
                        ),
                    ]),
                    style: None,
                })
                .collect(),
        }
    }

    #[test]
    fn half_open_cuts_clamp_and_drop_without_mutation() {
        let mapping = EditMapping::new(vec![(range(10, 20), Tick(100))]).unwrap();
        let source = track(&[(0, 10), (5, 15), (12, 18), (15, 25), (20, 30)]);
        let snapshot = source.clone();
        let mapped = mapping.map_cues(&source).unwrap();
        assert_eq!(
            mapped.cues.iter().map(|c| c.range).collect::<Vec<_>>(),
            vec![range(100, 105), range(102, 108), range(105, 110)]
        );
        for (cue, original) in mapped.cues.iter().zip(source.cues.iter().skip(1)) {
            assert_eq!(cue.texts, original.texts);
            assert_eq!(cue.source_lang, original.source_lang);
            assert_eq!(
                CueFragmentId::from_id(&cue.id).unwrap(),
                CueFragmentId {
                    source_id: original.id.clone(),
                    segment: 0
                }
            );
        }
        assert_eq!(source, snapshot);
        assert!(mapping.map_tick(Tick(9)).unwrap().is_empty());
        assert!(mapping.map_tick(Tick(20)).unwrap().is_empty());
        assert_eq!(
            mapping.map_tick(Tick(10)).unwrap(),
            vec![MappedTick {
                segment: 0,
                output: Tick(100)
            }]
        );
        assert_eq!(
            mapping
                .inverse_tick(MappedTick {
                    segment: 0,
                    output: Tick(110)
                })
                .unwrap(),
            None
        );
        assert_eq!(
            mapping
                .inverse_tick(MappedTick {
                    segment: 1,
                    output: Tick(100)
                })
                .unwrap(),
            None
        );
    }

    #[test]
    fn clip_track_rows_keep_source_ids_and_clamp_to_the_half_open_range() {
        for (case, cue, expected) in [
            ("inside", (112, 118), Some((112, 118))),
            ("crosses start", (105, 115), Some((110, 115))),
            ("crosses end", (115, 125), Some((115, 120))),
            ("exact range", (110, 120), Some((110, 120))),
            ("ends on start", (100, 110), None),
            ("starts on end", (120, 130), None),
            ("outside", (125, 135), None),
        ] {
            let source = track(&[cue]);
            let original = source.cues.first().unwrap();
            let clipped = clip_track(&source, range(110, 120)).unwrap();
            match expected {
                Some((start, end)) => {
                    let kept = clipped.cues.first().unwrap();
                    assert_eq!(
                        (kept.range.start().0, kept.range.end().0),
                        (start, end),
                        "{case}"
                    );
                    assert_eq!(kept.id, original.id, "{case}");
                    assert_eq!(kept.texts, original.texts, "{case}");
                    assert_eq!(kept.source_lang, original.source_lang, "{case}");
                }
                None => assert!(clipped.cues.is_empty(), "{case}"),
            }
        }
        let source = track(&[(105, 115), (112, 118), (115, 125), (125, 135)]);
        let clipped = clip_track(&source, range(110, 120)).unwrap();
        assert_eq!(
            clipped.cues.iter().map(|cue| cue.range).collect::<Vec<_>>(),
            vec![range(110, 115), range(112, 118), range(115, 120)]
        );
        assert_eq!(
            clipped
                .cues
                .iter()
                .map(|cue| cue.id.clone())
                .collect::<Vec<_>>(),
            source
                .cues
                .iter()
                .take(3)
                .map(|cue| cue.id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(clipped.styles, source.styles);
    }

    #[test]
    fn repeated_sources_and_disjoint_cuts_keep_occurrence_identity() {
        let mapping = EditMapping::new(vec![
            (range(10, 20), Tick(0)),
            (range(30, 40), Tick(10)),
            (range(10, 20), Tick(30)),
        ])
        .unwrap();
        let source = track(&[(5, 45)]);
        let cues = mapping.map_cues(&source).unwrap();
        assert_eq!(
            cues.cues.iter().map(|c| c.range).collect::<Vec<_>>(),
            vec![range(0, 10), range(10, 20), range(30, 40)]
        );
        assert_eq!(
            cues.cues
                .iter()
                .map(|c| CueFragmentId::from_id(&c.id).unwrap().segment)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(mapping.map_cues(&source).unwrap(), cues);
        let mapped = mapping.map_tick(Tick(12)).unwrap();
        assert_eq!(
            mapped,
            vec![
                MappedTick {
                    segment: 0,
                    output: Tick(2)
                },
                MappedTick {
                    segment: 2,
                    output: Tick(32)
                }
            ]
        );
        for tick in mapped {
            assert_eq!(mapping.inverse_tick(tick).unwrap(), Some(Tick(12)));
        }
        assert!(mapping.map_tick(Tick(25)).unwrap().is_empty());
        let intersections = mapping.map_range(range(15, 35)).unwrap();
        assert_eq!(
            intersections,
            vec![
                MappedRange {
                    segment: 0,
                    source: range(15, 20),
                    output: range(5, 10)
                },
                MappedRange {
                    segment: 1,
                    source: range(30, 35),
                    output: range(10, 15)
                },
                MappedRange {
                    segment: 2,
                    source: range(15, 20),
                    output: range(35, 40)
                },
            ]
        );
    }

    #[test]
    fn rebuild_from_original_after_changing_cut_has_no_cumulative_clipping() {
        let source = track(&[(0, 30)]);
        let old = EditMapping::new(vec![(range(10, 20), Tick(0))]).unwrap();
        assert_eq!(
            old.map_cues(&source).unwrap().cues.first().unwrap().range,
            range(0, 10)
        );
        let changed = EditMapping::new(vec![(range(5, 25), Tick(100))]).unwrap();
        let rebuilt = changed.map_cues(&source).unwrap();
        assert_eq!(rebuilt.cues.first().unwrap().range, range(100, 120));
        assert_eq!(
            rebuilt.cues.first().unwrap().texts,
            source.cues.first().unwrap().texts
        );
        assert_eq!(source.cues.first().unwrap().range, range(0, 30));
    }

    #[test]
    fn fractional_frames_audio_offset_and_srt_millisecond_accuracy() {
        let rate = FrameRate::FPS_29_97;
        let offset = Tick::from_units(500, 1000, Rounding::Nearest).unwrap();
        let cut_start = rate.tick_of(30).unwrap();
        let cut_end = rate.tick_of(90).unwrap();
        let source = track(&[(cut_start.0 + offset.0, cut_end.0 + offset.0)]);
        // Stream-local timestamps have been normalized once to asset source time.
        // Mapping consumes those coordinates without knowing or reapplying offset.
        let mapping = EditMapping::new(vec![(
            SourceRange::new(cut_start, cut_end).unwrap(),
            Tick::ZERO,
        )])
        .unwrap();
        let mapped = mapping.map_cues(&source).unwrap();
        let expected = SourceRange::new(offset, cut_end.checked_sub(cut_start).unwrap()).unwrap();
        assert_eq!(mapped.cues.first().unwrap().range, expected);
        let lang = LanguageTag::parse("vi").unwrap();
        let exported = export(&mapped, Format::Srt, std::slice::from_ref(&lang)).unwrap();
        assert!(exported.text.contains("00:00:00,500 --> 00:00:02,002"));
        let imported = parse(exported.text.as_bytes(), Format::Srt, lang).unwrap();
        let actual = imported.track.cues.first().unwrap().range;
        let tolerance = Tick::from_units(1, 1000, Rounding::Nearest).unwrap().0;
        assert!((actual.start().0 - expected.start().0).abs() <= tolerance);
        assert!((actual.end().0 - expected.end().0).abs() <= tolerance);
    }

    #[test]
    fn extreme_coordinates_overlaps_empty_mapping_and_bad_identities() {
        let mapping =
            EditMapping::new(vec![(range(i64::MIN, i64::MIN + 10), Tick(i64::MAX - 10))]).unwrap();
        assert_eq!(
            mapping
                .map_tick(Tick(i64::MIN + 9))
                .unwrap()
                .first()
                .unwrap()
                .output,
            Tick(i64::MAX - 1)
        );
        assert_eq!(
            mapping
                .map_range(range(i64::MIN, i64::MIN + 10))
                .unwrap()
                .first()
                .unwrap()
                .output,
            range(i64::MAX - 10, i64::MAX)
        );
        assert!(EditMapping::new(vec![(range(0, 10), Tick(i64::MAX - 9))]).is_err());
        let overlap =
            EditMapping::new(vec![(range(0, 10), Tick(-5)), (range(20, 30), Tick(-5))]).unwrap();
        assert_eq!(
            overlap
                .inverse_tick(MappedTick {
                    segment: 0,
                    output: Tick(0)
                })
                .unwrap(),
            Some(Tick(5))
        );
        assert_eq!(
            overlap
                .inverse_tick(MappedTick {
                    segment: 1,
                    output: Tick(0)
                })
                .unwrap(),
            Some(Tick(25))
        );
        let empty = EditMapping::default();
        assert!(empty.map_cues(&track(&[(0, 10)])).unwrap().cues.is_empty());
        assert!(empty.map_range(range(0, 10)).unwrap().is_empty());
        assert!(empty.map_tick(Tick(0)).unwrap().is_empty());
        for id in ["", "Tiếng/Việt", "7/a/b", "🙂/0", "00/1"] {
            let fragment = CueFragmentId {
                source_id: id.into(),
                segment: usize::MAX,
            };
            assert_eq!(CueFragmentId::from_id(&fragment.to_id()).unwrap(), fragment);
        }
        for id in [
            "",
            "1",
            "/source",
            "-1/source",
            "+1/source",
            "184467440737095516160/source",
        ] {
            assert!(CueFragmentId::from_id(&id.into()).is_err());
        }
        let mut duplicate = track(&[(0, 10), (10, 20)]);
        duplicate.cues.get_mut(1).unwrap().id = duplicate.cues.first().unwrap().id.clone();
        assert!(overlap.map_cues(&duplicate).is_err());
        assert!(clip_track(&duplicate, range(0, 10)).is_err());
    }

    proptest! {
        #[test]
        fn every_retained_occurrence_has_exact_inverse(
            start in -1_000_000_i64..1_000_000,
            duration in 1_i64..100_000,
            fraction in 0_i64..100_000,
            output in -1_000_000_i64..1_000_000,
        ) {
            let source = range(start,start+duration);
            let tick = Tick(start+fraction % duration);
            let mapping = EditMapping::new(vec![(source,Tick(output)),(source,Tick(output+duration+100))]).unwrap();
            let retained = mapping.map_tick(tick).unwrap();
            prop_assert_eq!(retained.len(),2);
            for mapped in retained {
                prop_assert_eq!(mapping.inverse_tick(mapped).unwrap(),Some(tick));
            }
            prop_assert!(mapping.map_tick(Tick(start-1)).unwrap().is_empty());
            prop_assert!(mapping.map_tick(source.end()).unwrap().is_empty());
            let intersections = mapping.map_range(source).unwrap();
            for mapped in intersections {
                prop_assert_eq!(mapped.source.duration(),mapped.output.duration());
                prop_assert_eq!(mapping.inverse_tick(MappedTick { segment:mapped.segment,output:mapped.output.start() }).unwrap(),Some(source.start()));
                prop_assert_eq!(mapping.inverse_tick(MappedTick { segment:mapped.segment,output:Tick(mapped.output.end().0-1) }).unwrap(),Some(Tick(source.end().0-1)));
            }
        }
    }
}
