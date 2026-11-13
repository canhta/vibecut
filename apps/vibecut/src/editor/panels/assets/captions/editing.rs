//! Cue edits build one engine content command from the current immutable snapshot.
use super::{Options, Subtitles};
use crate::{Error, Result};
use captions::{CueText, CueTrack, NormalizedText};
use captions::{LanguageTag, NfcText};
use engine::{Command, Session};
use project::{ItemId, ItemParams};
use std::sync::Arc;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

impl Subtitles {
    pub(crate) fn bank_available(&self, session: &Session, save: bool) -> bool {
        if !self.edit_ready(session) {
            return false;
        }
        if !save {
            return !session.project().data().caption_presets.is_empty();
        }
        self.style_available(session)
            && session.project().data().caption_presets.len() < project::MAX_CAPTION_PRESETS
            && self.caption(session).is_some_and(|(_, p)| {
                if p.preset.is_some() {
                    return true;
                }
                let first = p.cues.cues.first().and_then(|cue| p.cues.style_of(cue));
                first.is_some() && p.cues.cues.iter().all(|cue| p.cues.style_of(cue) == first)
            })
    }

    fn open_bank(&mut self, session: &Session, at: Tick, save: bool) -> Result<()> {
        if !self.bank_available(session, save) {
            return Err(Error::Invalid("caption preset unavailable"));
        }
        let preset = (!save)
            .then(|| session.project().data().caption_presets.first())
            .flatten();
        self.bank = Some(super::BankEdit {
            confirm_pending: false,
            snapshot: session.project().clone(),
            item: self.item,
            at,
            save,
            preset: preset.map(|p| p.id),
            name: preset.map_or(String::new(), |p| p.name.as_str().into()),
            focused: false,
        });
        Ok(())
    }

    pub(super) fn reconcile_bank(
        &mut self,
        session: &Session,
        at: Tick,
        ready: bool,
        continuous: bool,
    ) {
        if let Some(edit) = self.bank.take()
            && ready
            && continuous
            && self.edit_ready(session)
            && Arc::ptr_eq(&edit.snapshot, session.project())
            && edit.at == at
            && edit.item == self.item
        {
            self.bank = Some(edit);
        }
    }

    pub(super) fn confirm_bank(
        &mut self,
        session: &mut Session,
        at: Tick,
        delete: bool,
    ) -> Result<()> {
        let edit = self
            .bank
            .take()
            .ok_or(Error::Invalid("caption preset editor"))?;
        if !self.edit_ready(session)
            || !Arc::ptr_eq(&edit.snapshot, session.project())
            || edit.at != at
            || edit.item != self.item
        {
            return Err(Error::Invalid("caption preset changed"));
        }
        let command = if edit.save {
            Command::SaveCaptionPreset {
                item: edit.item.ok_or(Error::Invalid("caption item"))?,
                name: edit.name.as_str().into(),
            }
        } else if delete {
            Command::DeleteCaptionPreset {
                preset: edit.preset.ok_or(Error::Invalid("caption preset"))?,
            }
        } else {
            Command::RenameCaptionPreset {
                preset: edit.preset.ok_or(Error::Invalid("caption preset"))?,
                name: edit.name.as_str().into(),
            }
        };
        match session.apply(command) {
            Ok(_) => {
                self.refresh(session);
                Ok(())
            }
            Err(error) => {
                self.bank = Some(edit);
                Err(error.into())
            }
        }
    }

    pub(super) fn open_style_edit(&mut self, session: &Session, at: Tick) -> Result<()> {
        if !self.edit_ready(session) || self.bank.is_some() {
            return Err(Error::Invalid("caption style busy"));
        }
        let (item, params) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        if !self.style_available(session) {
            return Err(Error::Invalid("caption style unavailable"));
        }
        let preset = params
            .preset
            .ok_or(Error::Invalid("caption preset selection"))?;
        let base = session
            .project()
            .data()
            .caption_presets
            .iter()
            .find(|value| value.id == preset)
            .ok_or(Error::Invalid("caption preset"))?
            .style
            .clone();
        let mut draft = base.clone();
        params.overrides.apply(&mut draft);
        self.style_edit = Some(super::StyleEdit {
            snapshot: session.project().clone(),
            item: item.id,
            at,
            preset,
            base,
            draft,
        });
        Ok(())
    }

    pub(super) fn reconcile_style_edit(
        &mut self,
        session: &Session,
        at: Tick,
        ready: bool,
        continuous: bool,
    ) {
        if let Some(edit) = self.style_edit.take()
            && ready
            && continuous
            && self.edit_ready(session)
            && Arc::ptr_eq(&edit.snapshot, session.project())
            && edit.at == at
            && self.item == Some(edit.item)
            && self
                .caption(session)
                .is_some_and(|(_, params)| params.preset == Some(edit.preset))
        {
            self.style_edit = Some(edit);
        }
    }

    pub(super) fn confirm_style_edit(
        &mut self,
        session: &mut Session,
        at: Tick,
        reset: bool,
    ) -> Result<()> {
        let edit = self
            .style_edit
            .take()
            .ok_or(Error::Invalid("caption style editor"))?;
        if !self.edit_ready(session)
            || !Arc::ptr_eq(&edit.snapshot, session.project())
            || edit.at != at
            || self.item != Some(edit.item)
        {
            return Err(Error::Invalid("caption style changed"));
        }
        let overrides = if reset {
            captions::CaptionOverrides::default()
        } else {
            overrides_from(&edit.base, &edit.draft)
        };
        session.apply(Command::ApplyCaptionStyle {
            item: edit.item,
            style: engine::CaptionStyleApply::Saved {
                preset: edit.preset,
                overrides,
            },
        })?;
        self.refresh(session);
        Ok(())
    }

    pub(crate) fn overrides_available(&self, session: &Session) -> bool {
        self.style_available(session)
            && self
                .caption(session)
                .is_some_and(|(_, params)| params.preset.is_some())
    }

    /// Style actions never implicitly commit a focused or changed caption draft.
    pub(crate) fn style_available(&self, session: &Session) -> bool {
        self.edit_ready(session)
            && self.caption(session).is_some_and(|(item, params)| {
                !params.cues.cues.is_empty()
                    && session
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .any(|track| {
                            !track.locked && track.items.iter().any(|value| value.id == item.id)
                        })
            })
    }

    /// Unrelated edits must refuse before activation flushes a caption draft.
    pub(crate) fn edit_ready(&self, session: &Session) -> bool {
        !self.busy()
            && self.bank.is_none()
            && self.style_edit.is_none()
            && !session.transaction_active()
            && self.text_focus.is_none()
            && !self.drafts.iter().any(|((cue, language), draft)| {
                super::draft_text(session.project(), self.item, cue, language)
                    .map_or(!draft.is_empty(), |original| original != draft)
            })
    }

    /// Commit pending inline drafts before switching tabs or executing another command.
    pub(crate) fn flush_drafts(&mut self, session: &mut Session) -> Result<bool> {
        self.refresh(session);
        if self.problem == Some(crate::i18n::Text::SubDraftConflict) {
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
            "subtitles.style.save" | "subtitles.style.manage" => {
                self.open_bank(session, at, id == "subtitles.style.save")?;
            }
            "subtitles.style.overrides" => self.open_style_edit(session, at)?,
            "subtitles.style.word_highlight" => {
                if !self.style_available(session) {
                    return Err(Error::Invalid("caption style unavailable"));
                }
                let (item, params) = self
                    .caption(session)
                    .ok_or(Error::Invalid("caption item"))?;
                session.apply(Command::SetCaptionWordHighlight {
                    item: item.id,
                    enabled: !params.word_highlight,
                })?;
                self.refresh(session);
            }
            "subtitles.style.outline" | "subtitles.style.box" => {
                if !self.style_available(session) {
                    return Err(Error::Invalid("caption style unavailable"));
                }
                session.apply(Command::ApplyCaptionStyle {
                    item: self.item.ok_or(Error::Invalid("caption item"))?,
                    style: engine::CaptionStyleApply::Preset {
                        preset: if id == "subtitles.style.box" {
                            project::CaptionLook::LabelBlack
                        } else {
                            project::CaptionLook::OutlineBottom
                        },
                    },
                })?;
                self.refresh(session);
            }
            _ if id.starts_with("subtitles.style.look.") => {
                if !self.style_available(session) {
                    return Err(Error::Invalid("caption style unavailable"));
                }
                let word = id.trim_start_matches("subtitles.style.look.");
                let preset: project::CaptionLook =
                    serde_json::from_value(serde_json::Value::String(word.to_owned()))
                        .map_err(|_| Error::Invalid("caption look"))?;
                session.apply(Command::ApplyCaptionStyle {
                    item: self.item.ok_or(Error::Invalid("caption item"))?,
                    style: engine::CaptionStyleApply::Preset { preset },
                })?;
                self.refresh(session);
            }
            "subtitles.cancel" => self.cancel(),
            "subtitles.consent.unknown_price" => {
                let active = self
                    .active
                    .as_ref()
                    .ok_or(Error::Invalid("no subtitle job"))?;
                let mut state = active.events.state.lock().map_err(|_| Error::Poisoned)?;
                let upload = state
                    .upload
                    .as_mut()
                    .ok_or(Error::Invalid("no upload request"))?;
                upload.price_acknowledged = !upload.price_acknowledged;
            }
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
                if self.problem == Some(crate::i18n::Text::SubDraftConflict) {
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
    pub(super) fn commit(&mut self, session: &mut Session, cues: CueTrack) -> Result<()> {
        self.refresh(session);
        if self.problem == Some(crate::i18n::Text::SubDraftConflict) {
            return Err(Error::Invalid("resolve caption draft conflict"));
        }
        if self.busy() || session.transaction_active() {
            return Err(Error::Invalid("caption edit busy"));
        }
        let (word_highlight, preset, overrides) = {
            let params = &self
                .caption(session)
                .ok_or(Error::Invalid("caption item"))?
                .1;
            (
                params.word_highlight,
                params.preset,
                params.overrides.clone(),
            )
        };
        session.apply(Command::SetItemParam {
            item: self.item.ok_or(Error::Invalid("caption item"))?,
            content: ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight,
                preset,
                overrides,
                cues,
            }),
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
    /// Explicit split at the preview playhead. Each language divides on its
    /// nearest word boundary to that time; timing is user-chosen, never invented
    /// source word alignment. Refuses a cut that would leave an empty cue.
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
        let start = left.range.start();
        let end = left.range.end();
        if at <= start || at >= end {
            return Err(Error::Invalid("seek inside selected cue"));
        }
        // Native word timings win when the cue carries them; otherwise the
        // playhead stays the user-chosen cut.
        let at = snap_word_time(start, end, at, &left.word_ranges());
        let mut right = left.clone();
        let keys: Vec<_> = left.texts.keys().cloned().collect();
        for language in keys {
            let text = left
                .texts
                .get(&language)
                .ok_or(Error::Invalid("cue language"))?
                .text
                .as_str();
            let cut = cut_for_fraction(text, (at - start).0 as u128, (end - start).0 as u128)
                .ok_or(Error::Invalid("text too short to split"))?;
            let (a, b) = divide_text(text, cut)?;
            left.texts
                .get_mut(&language)
                .ok_or(Error::Invalid("cue language"))?
                .text = a.into();
            left.texts
                .get_mut(&language)
                .ok_or(Error::Invalid("cue language"))?
                .edited = true;
            let target = right
                .texts
                .get_mut(&language)
                .ok_or(Error::Invalid("cue language"))?;
            target.text = b.into();
            target.edited = true;
        }
        left.range = SourceRange::new(start, at)?;
        right.range = SourceRange::new(at, end)?;
        self.finish_split(session, cues, index, right)
    }
    /// Enter in a cue field: divide every language at its word boundary nearest
    /// the caret and the cue at the matching fraction of its span. One undo step.
    /// `caret` is a UTF-8 byte offset into `text`, the field's current draft.
    pub fn split_text(
        &mut self,
        session: &mut Session,
        cue: &NfcText,
        language: LanguageTag,
        text: &str,
        caret: usize,
    ) -> Result<Tick> {
        if text.len() > 64 * 1024 || text.trim().is_empty() {
            return Err(Error::Invalid("caption text"));
        }
        let (_, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let mut cues = p.cues.clone();
        let index = cues
            .cues
            .iter()
            .position(|c| &c.id == cue)
            .ok_or(Error::Invalid("cue identity"))?;
        let (_, before, total) =
            caret_word_cut(text, caret).ok_or(Error::Invalid("text too short to split"))?;
        let source = cues
            .cues
            .get_mut(index)
            .ok_or(Error::Invalid("cue index"))?;
        // Measured before the draft replaces the stored text the proof covers.
        let words = source.word_ranges();
        let reference = source
            .texts
            .get(&language)
            .and_then(|t| t.translation_ref.clone());
        source.set_text(
            language.clone(),
            CueText {
                text: text.into(),
                edited: true,
                translation_ref: reference,
            },
            true,
        );
        let start = source.range.start();
        let end = source.range.end();
        let exact_at = (language == source.source_lang)
            .then(|| exact_word_time(text, caret, &words, start, end))
            .flatten();
        let mut right = source.clone();
        let keys: Vec<_> = source.texts.keys().cloned().collect();
        for key in keys {
            let value = source
                .texts
                .get(&key)
                .ok_or(Error::Invalid("cue language"))?
                .text
                .as_str();
            let cut = if key == language {
                text.get(..caret)
                    .and_then(|_| caret_word_cut(text, caret))
                    .map(|(byte, _, _)| byte)
            } else {
                cut_for_fraction(value, u128::from(before), u128::from(total))
            }
            .ok_or(Error::Invalid("text too short to split"))?;
            let (a, b) = divide_text(value, cut)?;
            let left = source
                .texts
                .get_mut(&key)
                .ok_or(Error::Invalid("cue language"))?;
            left.text = a.into();
            left.edited = true;
            let target = right
                .texts
                .get_mut(&key)
                .ok_or(Error::Invalid("cue language"))?;
            target.text = b.into();
            target.edited = true;
        }
        let span = (end - start).0 as u128;
        if span < 2 {
            return Err(Error::Invalid("cue too short to split"));
        }
        let delta =
            ((span * u128::from(before) + u128::from(total) / 2) / u128::from(total)) as i64;
        let delta = delta.clamp(1, (span as i64) - 1);
        let at = start
            .checked_add(Tick(delta))
            .ok_or(Error::Invalid("cue time overflow"))?;
        let at = exact_at.unwrap_or_else(|| snap_word_time(start, end, at, &words));
        let source = cues
            .cues
            .get_mut(index)
            .ok_or(Error::Invalid("cue index"))?;
        source.range = SourceRange::new(start, at)?;
        right.range = SourceRange::new(at, end)?;
        self.finish_split(session, cues, index, right)?;
        self.requested_seek = Some(at);
        Ok(at)
    }
    /// Join the selected cue's text and span with its previous neighbour.
    pub fn merge_up(&mut self, session: &mut Session) -> Result<()> {
        self.merge_neighbour(session, false)
    }
    /// Join the selected cue's text and span with its next neighbour.
    pub fn merge_down(&mut self, session: &mut Session) -> Result<()> {
        self.merge_neighbour(session, true)
    }
    fn merge_neighbour(&mut self, session: &mut Session, down: bool) -> Result<()> {
        if self.selected.len() != 1 {
            return Err(Error::Invalid("select one cue"));
        }
        let (_, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let index = p
            .cues
            .cues
            .iter()
            .position(|c| self.selected.contains(&c.id))
            .ok_or(Error::Invalid("cue identity"))?;
        let neighbour = if down {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        }
        .ok_or(Error::Invalid("no neighbouring cue"))?;
        let id = p
            .cues
            .cues
            .get(neighbour)
            .ok_or(Error::Invalid("no neighbouring cue"))?
            .id
            .clone();
        self.selected.insert(id);
        self.flush_drafts(session)?;
        self.merge(session)
    }
    fn finish_split(
        &mut self,
        session: &mut Session,
        mut cues: CueTrack,
        index: usize,
        mut right: captions::Cue,
    ) -> Result<()> {
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
        let id = right.id.clone();
        let at = right.range.start();
        cues.cues.insert(index + 1, right);
        self.commit(session, cues)?;
        self.selected.clear();
        self.selected.insert(id.clone());
        self.anchor = Some(id);
        self.requested_seek = Some(at);
        Ok(())
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
        merged.word_timing =
            captions::CueWordTiming::joined(&selected.iter().collect::<Vec<_>>(), "\n")?;
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

/// Sparse patch from a full edited draft; equal fields keep the preset value.
fn overrides_from(
    base: &captions::CaptionStyle,
    draft: &captions::CaptionStyle,
) -> captions::CaptionOverrides {
    captions::CaptionOverrides {
        font: (draft.font != base.font).then(|| draft.font.clone()),
        font_style: (draft.font_style != base.font_style).then(|| draft.font_style.clone()),
        size: (draft.size != base.size).then_some(draft.size),
        color: (draft.color != base.color).then_some(draft.color),
        outline: (draft.outline != base.outline).then(|| draft.outline.clone()),
        background: (draft.background != base.background).then(|| draft.background.clone()),
        position: (draft.position != base.position).then_some(draft.position),
        safe_area_percent: (draft.safe_area_percent != base.safe_area_percent)
            .then_some(draft.safe_area_percent),
        max_lines: (draft.max_lines != base.max_lines).then_some(draft.max_lines),
        word_highlight_color: (draft.word_highlight_color != base.word_highlight_color)
            .then_some(draft.word_highlight_color),
    }
}

/// Exact native end of the word before the caret when word timing and the
/// text's word count agree; otherwise `None` so the proportional cut applies.
fn exact_word_time(
    text: &str,
    caret: usize,
    words: &[SourceRange],
    start: Tick,
    end: Tick,
) -> Option<Tick> {
    let (cut, _, _) = caret_word_cut(text, caret)?;
    let index = words_before(text, cut);
    if words.len() != word_count(text) {
        return None;
    }
    let at = words.get(index.checked_sub(1)?)?.end();
    (at > start && at < end && index < words.len()).then_some(at)
}

fn word_count(text: &str) -> usize {
    if text.chars().any(char::is_whitespace) {
        text.split_whitespace().count()
    } else {
        text.chars().filter(|c| !c.is_whitespace()).count()
    }
}

fn words_before(text: &str, cut: usize) -> usize {
    let left = text.get(..cut).unwrap_or_default();
    if text.chars().any(char::is_whitespace) {
        left.split_whitespace().count()
    } else {
        left.chars().filter(|c| !c.is_whitespace()).count()
    }
}

/// Nearest native word edge to `at`, kept strictly inside the cue. Empty
/// word timing keeps the caller's exact tick.
fn snap_word_time(start: Tick, end: Tick, at: Tick, words: &[SourceRange]) -> Tick {
    words
        .iter()
        .flat_map(|word| [word.start(), word.end()])
        .filter(|tick| *tick > start && *tick < end)
        .min_by_key(|tick| tick.0.abs_diff(at.0))
        .unwrap_or(at)
}

/// Split raw text at a grapheme boundary, dropping only surrounding whitespace.
fn divide_text(text: &str, cut: usize) -> Result<(String, String)> {
    if cut == 0 || cut >= text.len() || !text.is_char_boundary(cut) {
        return Err(Error::Invalid("text too short to split"));
    }
    let a = text
        .get(..cut)
        .ok_or(Error::Invalid("text split"))?
        .trim()
        .to_owned();
    let b = text
        .get(cut..)
        .ok_or(Error::Invalid("text split"))?
        .trim()
        .to_owned();
    if a.is_empty() || b.is_empty() {
        return Err(Error::Invalid("text too short to split"));
    }
    Ok((a, b))
}

fn is_space(grapheme: &str) -> bool {
    grapheme.chars().all(char::is_whitespace)
}

/// Scripts whose words are not separated by spaces, so any grapheme boundary
/// is a legal word boundary.
fn unspaced_script(grapheme: &str) -> bool {
    grapheme.chars().any(|c| {
        matches!(c as u32,
            0x0E00..=0x0EFF // Thai, Lao
            | 0x1000..=0x109F // Myanmar
            | 0x1780..=0x17FF // Khmer
            | 0x3040..=0x30FF // Hiragana, Katakana
            | 0x3400..=0x4DBF // CJK Extension A
            | 0x4E00..=0x9FFF // CJK Unified Ideographs
            | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        )
    })
}

/// Every legal cut with the count of non-whitespace graphemes left of it.
/// Text with word separators cuts only between words; unspaced scripts (zh)
/// cut on any grapheme boundary.
fn word_cuts(text: &str) -> Option<(Vec<(usize, u32)>, u32)> {
    let normalized = NormalizedText::new(text).ok()?;
    let spans = normalized.spans();
    let mut cuts = Vec::new();
    let mut units = 0u32;
    for (index, span) in spans.iter().enumerate() {
        if index > 0 {
            let start = span.raw.start;
            let previous = spans
                .get(index - 1)
                .and_then(|s| text.get(s.raw.clone()))
                .unwrap_or_default();
            let current = text.get(span.raw.clone()).unwrap_or_default();
            let left = text.get(..start).unwrap_or_default();
            let right = text.get(start..).unwrap_or_default();
            if !left.trim().is_empty()
                && !right.trim().is_empty()
                && (unspaced_script(previous)
                    || unspaced_script(current)
                    || is_space(previous)
                    || is_space(current))
            {
                cuts.push((start, units));
            }
        }
        if !is_space(text.get(span.raw.clone()).unwrap_or_default()) {
            units = units.saturating_add(1);
        }
    }
    (!cuts.is_empty() && units > 0).then_some((cuts, units))
}

/// Cut at the word boundary nearest a UTF-8 byte caret; returns the byte offset
/// and the non-whitespace unit counts before and after the whole text.
fn caret_word_cut(text: &str, caret: usize) -> Option<(usize, u32, u32)> {
    let (cuts, total) = word_cuts(text)?;
    let caret = caret.min(text.len());
    let (byte, before) = cuts
        .iter()
        .copied()
        .min_by_key(|(byte, _)| (byte.abs_diff(caret), *byte))?;
    Some((byte, before, total))
}

/// Nearest word boundary to `numerator / denominator` of the text's units.
fn cut_for_fraction(text: &str, numerator: u128, denominator: u128) -> Option<usize> {
    if denominator == 0 {
        return None;
    }
    let (cuts, total) = word_cuts(text)?;
    let target = ((u128::from(total) * numerator + denominator / 2) / denominator) as u32;
    cuts.iter()
        .copied()
        .min_by_key(|(byte, before)| (before.abs_diff(target), *byte))
        .map(|(byte, _)| byte)
}
