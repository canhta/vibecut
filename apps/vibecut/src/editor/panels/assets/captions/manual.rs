//! Manual caption entry: an empty caption item at the playhead and cues typed into it.
use super::{Options, Subtitles};
use crate::{Error, Result};
use captions::{Cue, CueText, CueWordTiming, LanguageTag};
use engine::{Command, CreatedId, Session};
use std::collections::BTreeMap;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// Length of a new empty caption item. The reference does not measure it: open in the editor
/// design file.
const ITEM_SECONDS: i64 = 5;
/// Length of a new cue before it is clipped by the item end or the next cue: open as above.
const CUE_SECONDS: i64 = 2;

impl Subtitles {
    /// New caption item at the playhead: one engine command, one undo step. A later item goes
    /// on the selected item's track; the first one allocates a text track.
    pub fn add_manual(&mut self, session: &mut Session, at: Tick) -> Result<()> {
        self.refresh(session);
        if self.busy() || session.transaction_active() {
            return Err(Error::Invalid("caption edit busy"));
        }
        let id = media::AssetId(
            session
                .project()
                .data()
                .assets
                .iter()
                .map(|a| a.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(Error::Invalid("asset identity overflow"))?,
        );
        let duration = Tick(ITEM_SECONDS * TICKS_PER_SECOND);
        let asset = media::SourceAsset::try_from(media::AssetData {
            ephemeral: true,
            id,
            path: std::env::temp_dir().join("manual.captions"),
            content_hash: media::ContentHash::from_bytes(b"manual"),
            probe: media::SourceProbe {
                format: "manual".into(),
                duration: Some(duration),
                streams: Vec::new(),
            },
        })
        .map_err(|_| Error::Invalid("manual caption asset"))?;
        let track = self.item.and_then(|item| {
            session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .find(|track| track.items.iter().any(|i| i.id == item))
                .map(|track| track.id)
        });
        let outcome = session.apply(Command::CaptionsAddEmpty {
            asset,
            start: at,
            duration,
            track,
            name: None,
        })?;
        let created = outcome.created.iter().find_map(|id| match id {
            CreatedId::Item(id) => Some(*id),
            _ => None,
        });
        if let Some(item) = created {
            self.select_item(session, item)?;
            // Placement can move the item off the playhead; cues are added at the playhead.
            self.requested_seek = self.caption(session).map(|(item, _)| item.start);
        }
        self.refresh(session);
        Ok(())
    }

    /// Add one cue holding `text` at the playhead of the selected caption item. The cue never
    /// overlaps a neighbour and stays inside the item. One undo step.
    pub fn add_cue(
        &mut self,
        session: &mut Session,
        options: &Options,
        at: Tick,
        text: &str,
    ) -> Result<()> {
        let text = text.trim();
        if text.is_empty() || text.len() > 64 * 1024 {
            return Err(Error::Invalid("caption text"));
        }
        let (item, params) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let first = item.source_range.start();
        let limit = item.source_range.end();
        let start = at
            .checked_sub(item.start)
            .and_then(|d| first.checked_add(d))
            .ok_or(Error::Invalid("cue time overflow"))?;
        if start < first || start >= limit {
            return Err(Error::Invalid("seek inside caption item"));
        }
        let mut cues = params.cues.clone();
        if cues
            .cues
            .iter()
            .any(|c| c.range.start() <= start && start < c.range.end())
        {
            return Err(Error::Invalid("seek outside existing cues"));
        }
        let mut end = start
            .checked_add(Tick(CUE_SECONDS * TICKS_PER_SECOND))
            .ok_or(Error::Invalid("cue time overflow"))?
            .min(limit);
        if let Some(next) = cues
            .cues
            .iter()
            .map(|c| c.range.start())
            .filter(|s| *s > start)
            .min()
        {
            end = end.min(next);
        }
        let language = LanguageTag::parse(&options.source)
            .ok()
            .or_else(|| cues.cues.first().map(|c| c.source_lang.clone()))
            .map_or_else(|| LanguageTag::parse("en"), Ok)?;
        let mut ordinal = cues.cues.len().saturating_add(1);
        let id = loop {
            let id: captions::NfcText = format!("manual-{ordinal}").into();
            if !cues.cues.iter().any(|c| c.id == id) {
                break id;
            }
            ordinal = ordinal
                .checked_add(1)
                .ok_or(Error::Invalid("cue identity overflow"))?;
        };
        let index = cues
            .cues
            .iter()
            .position(|c| c.range.start() > start)
            .unwrap_or(cues.cues.len());
        cues.cues.insert(
            index,
            Cue {
                word_timing: CueWordTiming::Unverified,
                id: id.clone(),
                range: SourceRange::new(start, end)?,
                source_lang: language.clone(),
                texts: BTreeMap::from([(
                    language,
                    CueText {
                        text: text.into(),
                        edited: true,
                        translation_ref: None,
                    },
                )]),
                style: None,
            },
        );
        self.commit(session, cues)?;
        self.selected.clear();
        self.selected.insert(id.clone());
        self.anchor = Some(id);
        Ok(())
    }
}
