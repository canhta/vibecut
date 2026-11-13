//! Answers transcript text queries from a per-asset index kept in the artifact cache.
mod folding;
mod index;
use super::{Error, Result};
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::{EditError, RuntimeError};
use media::AssetId;
use project::{ItemParams, ProjectData};
use std::collections::BTreeMap;
use time::Tick;
pub use folding::fold;
/// Longest accepted query, in UTF-8 bytes before folding.
pub const MAX_QUERY_BYTES: usize = 256;
/// Most hits one query returns.
pub const MAX_HITS: usize = 200;
/// Hit count when a caller names no limit.
pub const DEFAULT_HITS: usize = 50;
const SNIPPET_CHARS: usize = 160;
/// One source-timed transcript line of an asset, in its source-language text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: Tick,
    pub end: Tick,
    pub text: String,
}
/// The transcript of one source asset as the project holds it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub asset: AssetId,
    /// Content hash of the media the transcript was made from.
    pub content: [u8; 32],
    /// Media duration when the probe knows it; used to report partial coverage.
    pub duration: Option<Tick>,
    /// Ascending by start, without duplicates.
    pub segments: Vec<Segment>,
}
/// One matching transcript line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub asset: AssetId,
    pub start: Tick,
    pub end: Tick,
    pub snippet: String,
}
/// Query result. `partial` means the transcript covers less than its media for some
/// asset; `stale` means an index from older media or transcript was replaced; `rebuilt`
/// counts indexes built during this query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub hits: Vec<Hit>,
    pub partial: bool,
    pub stale: bool,
    pub rebuilt: usize,
}
/// Collects the transcript of every asset that caption items were made from. Captions
/// without a source asset, assets missing from the project and empty transcripts are skipped.
pub fn sources(data: &ProjectData) -> Vec<Source> {
    let mut by_asset: BTreeMap<AssetId, Vec<Segment>> = BTreeMap::new();
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            let (Some(asset), ItemParams::Caption(params)) = (item.source_asset, &item.content)
            else {
                continue;
            };
            let segments = by_asset.entry(asset).or_default();
            for cue in &params.cues.cues {
                if let Some(text) = cue.source_text() {
                    segments.push(Segment {
                        start: cue.range.start(),
                        end: cue.range.end(),
                        text: text.text.as_str().to_owned(),
                    });
                }
            }
        }
    }
    let mut out = Vec::new();
    for (asset, mut segments) in by_asset {
        segments.sort_by(|a, b| {
            a.start
                .0
                .cmp(&b.start.0)
                .then(a.end.0.cmp(&b.end.0))
                .then_with(|| a.text.cmp(&b.text))
        });
        segments.dedup_by(|a, b| a.start.0 == b.start.0 && a.end.0 == b.end.0 && a.text == b.text);
        let Some(media) = data.assets.iter().find(|candidate| candidate.id() == asset) else {
            continue;
        };
        if segments.is_empty() {
            continue;
        }
        out.push(Source {
            asset,
            content: media.content_hash().0,
            duration: media.probe().duration,
            segments,
        });
    }
    out
}
/// Searches every source, reusing or rebuilding each asset's index in `cache`.
/// The query matches folded text, so case, diacritics and compatibility forms do not
/// matter. Results follow source order, then time, and stop at `limit` hits.
pub fn query(
    cache: &mut ArtifactCache,
    sources: &[Source],
    needle: &str,
    limit: usize,
) -> Result<Answer> {
    if needle.trim().is_empty() || needle.len() > MAX_QUERY_BYTES {
        return Err(Error::Edit(EditError::Invalid(
            "search query must be 1 to 256 bytes",
        )));
    }
    if !(1..=MAX_HITS).contains(&limit) {
        return Err(Error::Edit(EditError::Invalid(
            "search limit must be 1 to 200",
        )));
    }
    let folded_needle = fold(needle.trim());
    let mut answer = Answer {
        hits: Vec::new(),
        partial: false,
        stale: false,
        rebuilt: 0,
    };
    for source in sources {
        let fingerprint = index::fingerprint(source.content, &source.segments);
        let index = load_or_build(cache, source, fingerprint, &mut answer)?;
        let covered = source
            .segments
            .iter()
            .map(|segment| segment.end.0)
            .max()
            .unwrap_or(0);
        if source.duration.as_ref().is_some_and(|duration| covered < duration.0) {
            answer.partial = true;
        }
        if folded_needle.is_empty() {
            continue;
        }
        for entry in &index.entries {
            if answer.hits.len() >= limit {
                break;
            }
            if entry.folded.contains(&folded_needle) {
                answer.hits.push(Hit {
                    asset: source.asset,
                    start: Tick(entry.start),
                    end: Tick(entry.end),
                    snippet: entry.text.chars().take(SNIPPET_CHARS).collect(),
                });
            }
        }
    }
    Ok(answer)
}
/// Returns the index for `source` when the cached entry matches the current media and
/// transcript; otherwise builds it, flags a replaced entry as stale and stores it. A
/// refused store still answers the query from memory.
fn load_or_build(
    cache: &mut ArtifactCache,
    source: &Source,
    fingerprint: [u8; 32],
    answer: &mut Answer,
) -> Result<index::Index> {
    let key = asset_key(source.asset)?;
    let cached = match cache.get(key) {
        Ok(Some(lease)) => index::decode(lease.bytes()),
        Ok(None) | Err(RuntimeError::CacheCorrupt) => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(cached) = cached {
        if cached.matches(source.content, fingerprint) {
            return Ok(cached);
        }
        answer.stale = true;
    }
    let built = index::build(source.content, fingerprint, &source.segments);
    answer.rebuilt += 1;
    let bytes = index::encode(&built)?;
    match cache.put(key, &bytes) {
        Ok(()) | Err(RuntimeError::ArtifactTooLarge | RuntimeError::BudgetFull) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(built)
}
fn asset_key(asset: AssetId) -> Result<CacheKey> {
    let settings = format!("asset={}", asset.0);
    Ok(CacheKey::new(KeyInputs {
        kind: ArtifactKind::TranscriptIndex,
        source: [0; 32],
        provider: "vibecut",
        model: "transcript-search",
        version: index::VERSION,
        settings: settings.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })?)
}
