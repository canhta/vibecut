//! Cue edits build one engine content command from the current immutable snapshot.
use super::{Options, Subtitles};
use crate::{Error, Result};
use captions::{CueText, CueTrack, NormalizedText};
use captions::{LanguageTag, NfcText};
use engine::{Command, Session};
use project::{ItemId, ItemParams};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

impl Subtitles {
    /// Commit pending inline drafts before switching tabs or executing another command.
    pub(crate) fn flush_drafts(&mut self, session: &mut Session) -> Result<bool> {
        self.refresh(session);
        if self.problem == Some(crate::catalog::Text::SubDraftConflict) {
            return Err(Error::Invalid("resolve caption draft conflict"));
        }
        let Some((_, p)) = self.caption(session) else {
            return Ok(false);
        };
        let mut cues = p.cues.clone();
        let mut changed = false;
        for cue in &mut cues.cues {
            for ((id, language), draft) in &self.drafts {
                if id != &cue.id
                    || cue
                        .texts
                        .get(language)
                        .is_some_and(|t| t.text.as_str() == draft)
                    || (draft.is_empty() && !cue.texts.contains_key(language))
                {
                    continue;
                }
                if draft.trim().is_empty() || draft.len() > 64 * 1024 {
                    return Err(Error::Invalid("caption text"));
                }
                let reference = cue
                    .texts
                    .get(language)
                    .and_then(|t| t.translation_ref.clone());
                cue.set_text(
                    language.clone(),
                    CueText {
                        text: draft.as_str().into(),
                        edited: true,
                        translation_ref: reference,
                    },
                    true,
                );
                changed = true;
            }
        }
        if changed {
            self.commit(session, cues)?;
        }
        Ok(changed)
    }
    /// Stable action dispatch shared with agents. Native file results enter import_file.
    pub fn activate(
        &mut self,
        session: &mut Session,
        options: &mut Options,
        source: Option<ItemId>,
        at: Tick,
        id: &str,
    ) -> Result<()> {
        match id {
            "subtitles.cancel" => self.cancel(),
            "subtitles.consent.approve" => self.approve_upload(true)?,
            "subtitles.consent.deny" => self.approve_upload(false)?,
            "subtitles.generate" => self.generate(
                session,
                source.ok_or(Error::Invalid("select source media"))?,
                options,
            )?,
            "subtitles.translate" => self.translate(session, options)?,
            "subtitles.split" => self.split(session, at)?,
            "subtitles.merge" => self.merge(session)?,
            "subtitles.earlier" => self.nudge(session, false)?,
            "subtitles.later" => self.nudge(session, true)?,
            "subtitles.select_all" => {
                self.selected = self
                    .caption(session)
                    .ok_or(Error::Invalid("caption item"))?
                    .1
                    .cues
                    .cues
                    .iter()
                    .map(|c| c.id.clone())
                    .collect();
            }
            "subtitles.models" => self.requested_model = self.missing_model.clone(),
            "subtitles.bilingual" => options.bilingual = !options.bilingual,
            "subtitles.force" => self.force = !self.force,
            "subtitles.dismiss" => {
                if self.problem == Some(crate::catalog::Text::SubDraftConflict) {
                    self.drafts.clear();
                    self.draft_project = Some(session.project().clone());
                }
                self.problem = None;
                self.completed = None;
            }
            row if row.starts_with("subtitles.cue.") => {
                let id = row.trim_start_matches("subtitles.cue.");
                let index = self
                    .caption(session)
                    .ok_or(Error::Invalid("caption item"))?
                    .1
                    .cues
                    .cues
                    .iter()
                    .position(|c| c.id.as_str() == id)
                    .ok_or(Error::Invalid("cue identity"))?;
                self.requested_seek = Some(self.select_cue(session, index, false, false)?);
            }
            _ => {
                return Err(Error::Unsupported(
                    "subtitle action; inject file or text input at its typed boundary",
                ));
            }
        }
        Ok(())
    }
    pub fn select_item(&mut self, session: &Session, item: ItemId) -> Result<()> {
        let previous = self.item;
        self.item = Some(item);
        if self.caption(session).is_none() {
            self.item = previous;
            return Err(Error::Invalid("caption item"));
        }
        self.selected.clear();
        self.anchor = None;
        self.page = 0;
        self.drafts.clear();
        Ok(())
    }
    /// Shift extends from the anchor; additive selection toggles one cue.
    pub fn select_cue(
        &mut self,
        session: &Session,
        index: usize,
        extend: bool,
        additive: bool,
    ) -> Result<Tick> {
        let (item, params) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let cue = params
            .cues
            .cues
            .get(index)
            .ok_or(Error::Invalid("cue index"))?;
        let at = cue
            .range
            .start()
            .checked_sub(item.source_range.start())
            .and_then(|d| item.start.checked_add(d))
            .ok_or(Error::Invalid("cue time overflow"))?;
        if extend {
            let anchor = self
                .anchor
                .as_ref()
                .and_then(|id| params.cues.cues.iter().position(|cue| &cue.id == id))
                .unwrap_or(index);
            let range = anchor.min(index)..=anchor.max(index);
            let ids: Vec<_> = params
                .cues
                .cues
                .get(range)
                .ok_or(Error::Invalid("cue selection range"))?
                .iter()
                .map(|c| c.id.clone())
                .collect();
            if !additive {
                self.selected.clear();
            }
            self.selected.extend(ids);
        } else {
            let id = cue.id.clone();
            if additive {
                if !self.selected.remove(&id) {
                    self.selected.insert(id);
                }
            } else {
                self.selected.clear();
                self.selected.insert(id);
            }
            self.anchor = Some(cue.id.clone());
        }
        Ok(at)
    }
    pub fn edit_text(
        &mut self,
        session: &mut Session,
        cue: &NfcText,
        language: LanguageTag,
        text: &str,
    ) -> Result<()> {
        if text.len() > 64 * 1024 || text.trim().is_empty() {
            return Err(Error::Invalid("caption text"));
        }
        let (_, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let mut cues = p.cues.clone();
        let cue = cues
            .cues
            .iter_mut()
            .find(|c| &c.id == cue)
            .ok_or(Error::Invalid("cue identity"))?;
        let reference = cue
            .texts
            .get(&language)
            .and_then(|t| t.translation_ref.clone());
        cue.set_text(
            language,
            CueText {
                text: text.into(),
                edited: true,
                translation_ref: reference,
            },
            true,
        );
        self.commit(session, cues)
    }
    fn commit(&mut self, session: &mut Session, cues: CueTrack) -> Result<()> {
        self.refresh(session);
        if self.problem == Some(crate::catalog::Text::SubDraftConflict) {
            return Err(Error::Invalid("resolve caption draft conflict"));
        }
        if self.busy() || session.transaction_active() {
            return Err(Error::Invalid("caption edit busy"));
        }
        session.apply(Command::SetItemParam {
            item: self.item.ok_or(Error::Invalid("caption item"))?,
            content: ItemParams::Caption(project::CaptionParams { cues }),
        })?;
        self.drafts.clear();
        self.refresh(session);
        Ok(())
    }
    /// Timing nudge is one frame. Boundaries and overlaps refuse the entire selection.
    pub fn nudge(&mut self, session: &mut Session, forward: bool) -> Result<()> {
        let (item, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let bounds = item.source_range;
        let frame = session.project().data().canvas.fps.frame_duration()?;
        let delta = if forward { frame } else { Tick(-frame.0) };
        let mut cues = p.cues.clone();
        if self.selected.is_empty() {
            return Err(Error::Invalid("select cues"));
        }
        for cue in &mut cues.cues {
            if self.selected.contains(&cue.id) {
                let start = cue
                    .range
                    .start()
                    .checked_add(delta)
                    .ok_or(Error::Invalid("cue time overflow"))?;
                let end = cue
                    .range
                    .end()
                    .checked_add(delta)
                    .ok_or(Error::Invalid("cue time overflow"))?;
                if start < bounds.start() || end > bounds.end() {
                    return Err(Error::Invalid("cue outside item"));
                }
                cue.range = SourceRange::new(start, end)?;
            }
        }
        if cues.cues.windows(2).any(|p| {
            p.first()
                .zip(p.get(1))
                .is_some_and(|(a, b)| a.range.end() > b.range.start())
        }) {
            return Err(Error::Invalid("cue overlap"));
        }
        self.commit(session, cues)
    }
    /// Explicit split at the preview playhead. Text divides on whole graphemes;
    /// timing is user-chosen, never invented source word alignment.
    pub fn split(&mut self, session: &mut Session, at: Tick) -> Result<()> {
        if self.selected.len() != 1 {
            return Err(Error::Invalid("select one cue"));
        }
        let (item, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let at = at
            .checked_sub(item.start)
            .and_then(|d| item.source_range.start().checked_add(d))
            .ok_or(Error::Invalid("cue time overflow"))?;
        let mut cues = p.cues.clone();
        let index = cues
            .cues
            .iter()
            .position(|c| self.selected.contains(&c.id))
            .ok_or(Error::Invalid("cue identity"))?;
        let left = cues
            .cues
            .get_mut(index)
            .ok_or(Error::Invalid("cue index"))?;
        if at <= left.range.start() || at >= left.range.end() {
            return Err(Error::Invalid("seek inside selected cue"));
        }
        let mut right = left.clone();
        for (language, text) in &mut left.texts {
            let normalized = NormalizedText::new(text.text.as_str())?;
            let cut = normalized
                .spans()
                .get(normalized.spans().len() / 2)
                .filter(|_| normalized.spans().len() >= 2)
                .map(|s| s.normalized.start)
                .ok_or(Error::Invalid("text too short to split"))?;
            let a = text
                .text
                .as_str()
                .get(..cut)
                .ok_or(Error::Invalid("text split"))?
                .to_owned();
            let b = text
                .text
                .as_str()
                .get(cut..)
                .ok_or(Error::Invalid("text split"))?
                .to_owned();
            if a.trim().is_empty() || b.trim().is_empty() {
                return Err(Error::Invalid("text too short to split"));
            }
            text.text = a.trim().into();
            text.edited = true;
            let target = right
                .texts
                .get_mut(language)
                .ok_or(Error::Invalid("cue language"))?;
            target.text = b.trim().into();
            target.edited = true;
        }
        left.range = SourceRange::new(left.range.start(), at)?;
        right.range = SourceRange::new(at, right.range.end())?;
        let original = right.id.as_str().to_owned();
        let mut ordinal = 1usize;
        loop {
            let id: NfcText = format!("{original}~{ordinal}").into();
            if !cues.cues.iter().any(|c| c.id == id) {
                right.id = id;
                break;
            }
            ordinal = ordinal
                .checked_add(1)
                .ok_or(Error::Invalid("cue identity overflow"))?;
        }
        cues.cues.insert(index + 1, right);
        self.commit(session, cues)
    }
    /// Merge a contiguous selection only; retain independent language provenance.
    pub fn merge(&mut self, session: &mut Session) -> Result<()> {
        let (_, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let mut cues = p.cues.clone();
        let indices: Vec<_> = cues
            .cues
            .iter()
            .enumerate()
            .filter_map(|(i, c)| self.selected.contains(&c.id).then_some(i))
            .collect();
        let first = *indices
            .first()
            .ok_or(Error::Invalid("select adjacent cues"))?;
        let last = *indices
            .last()
            .ok_or(Error::Invalid("select adjacent cues"))?;
        if indices.len() < 2 || last - first + 1 != indices.len() {
            return Err(Error::Invalid("select adjacent cues"));
        }
        let selected = cues
            .cues
            .get(first..=last)
            .ok_or(Error::Invalid("cue range"))?;
        let mut merged = selected.first().ok_or(Error::Invalid("cue range"))?.clone();
        if selected
            .iter()
            .any(|c| c.source_lang != merged.source_lang || c.texts.keys().ne(merged.texts.keys()))
        {
            return Err(Error::Invalid("merge needs matching languages"));
        }
        merged.range = SourceRange::new(
            merged.range.start(),
            selected
                .last()
                .ok_or(Error::Invalid("cue range"))?
                .range
                .end(),
        )?;
        for (language, text) in &mut merged.texts {
            let values: Vec<_> = selected
                .iter()
                .filter_map(|c| c.texts.get(language))
                .collect();
            let combined = values
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if combined.len() > 64 * 1024 {
                return Err(Error::Invalid("merged text too large"));
            }
            text.text = combined.into();
            text.edited = true;
            if values
                .iter()
                .any(|v| v.translation_ref != text.translation_ref)
            {
                text.translation_ref = None;
            }
        }
        cues.cues.splice(first..=last, [merged]);
        self.commit(session, cues)
    }
    pub fn cps_warning(
        cue: &captions::Cue,
        language: &LanguageTag,
        profiles: &captions::LanguageRegistry,
    ) -> Result<bool> {
        let Some(text) = cue.texts.get(language) else {
            return Ok(false);
        };
        let count = NormalizedText::new(text.text.as_str())?.spans().len() as u128;
        Ok(count * TICKS_PER_SECOND as u128
            > cue.range.duration().0 as u128
                * u128::from(profiles.profile(language).graphemes_per_second))
    }
}
