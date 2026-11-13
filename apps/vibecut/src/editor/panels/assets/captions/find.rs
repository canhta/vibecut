//! Find-replace over the cue list: query state, the preview and the one batch commit.
use super::Subtitles;
use super::view::Controls;
use crate::{
    Error, Result, egui,
    i18n::{self, Locale, Text},
    kit::Icon,
};
use captions::{CueOutcome, FindOptions, FindPlan, NfcText};
use engine::Session;
use project::ItemId;
use std::collections::BTreeSet;

/// The section's inputs and the preview they last produced. Replace writes only a preview whose
/// inputs and recomputed plan both still match.
#[derive(Default)]
pub(super) struct Find {
    query: String,
    replacement: String,
    options: FindOptions,
    preview: Option<Preview>,
    /// Matches the last replace wrote; the next preview clears it.
    done: Option<usize>,
}

/// What a preview was computed from.
#[derive(Clone, PartialEq, Eq)]
struct Request {
    item: ItemId,
    /// The selected cues, or `None` for every cue of the item.
    scope: Option<BTreeSet<NfcText>>,
    query: String,
    replacement: String,
    options: FindOptions,
}

struct Preview {
    request: Request,
    plan: FindPlan,
}

/// The counts a preview shows, copied out so the section can draw them while it edits its inputs.
#[derive(Clone, Copy)]
struct Summary {
    matches: usize,
    cues: usize,
    replaced: usize,
    protected: usize,
    emptied: usize,
    too_long: usize,
}

impl Summary {
    fn of(plan: &FindPlan) -> Self {
        Self {
            matches: plan.matches,
            cues: plan.cues.len(),
            replaced: plan.replaced_matches(),
            protected: plan.cues_with(|o| matches!(o, CueOutcome::Protected)),
            emptied: plan.cues_with(|o| matches!(o, CueOutcome::Emptied)),
            too_long: plan.cues_with(|o| matches!(o, CueOutcome::TooLong)),
        }
    }
}

impl Subtitles {
    /// The current request: the caption item, the selection as scope, and the inputs.
    fn find_request(&self) -> Option<Request> {
        Some(Request {
            item: self.item?,
            scope: (!self.selected.is_empty()).then(|| self.selected.clone()),
            query: self.find.query.clone(),
            replacement: self.find.replacement.clone(),
            options: self.find.options,
        })
    }

    /// The plan of the last preview, while its inputs are still the current ones.
    pub fn find_plan(&self) -> Option<&FindPlan> {
        let request = self.find_request()?;
        self.find
            .preview
            .as_ref()
            .filter(|preview| preview.request == request)
            .map(|preview| &preview.plan)
    }

    /// Previews the current query over the scope. Writes nothing.
    pub fn find_preview(&mut self, session: &Session) -> Result<()> {
        let request = self.find_request().ok_or(Error::Invalid("caption item"))?;
        let (_, params) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let plan = captions::find_plan(
            &params.cues,
            request.scope.as_ref(),
            &request.query,
            &request.replacement,
            request.options,
        )?;
        self.find.done = None;
        self.find.preview = Some(Preview { request, plan });
        Ok(())
    }

    /// Writes the previewed replacement as one batch edit: one undo step, timings unchanged.
    /// Refuses unless the preview belongs to the current inputs and the plan recomputed now is
    /// the plan that was previewed.
    pub fn find_replace(&mut self, session: &mut Session) -> Result<usize> {
        let request = self.find_request().ok_or(Error::Invalid("caption item"))?;
        let preview = self
            .find
            .preview
            .take()
            .filter(|preview| preview.request == request)
            .ok_or(Error::Invalid("find preview"))?;
        let mut cues = self
            .caption(session)
            .map(|(_, params)| params.cues.clone())
            .ok_or(Error::Invalid("caption item"))?;
        let plan = captions::find_replace(
            &mut cues,
            request.scope.as_ref(),
            &request.query,
            &request.replacement,
            request.options,
        )?;
        if plan != preview.plan {
            return Err(Error::Invalid("find preview changed"));
        }
        let count = plan.replaced_matches();
        self.commit(session, cues)?;
        self.find.done = Some(count);
        Ok(count)
    }

    /// The find-replace section above the cue list: the query, the replacement, the options, the
    /// scope, the preview counts and the batch Replace that only a current preview enables.
    pub(super) fn find_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        c: &mut Controls,
        locale: Locale,
    ) {
        let busy = self.busy();
        let selected = self.selected.len();
        let summary = self.find_plan().map(Summary::of);
        let done = self.find.done;
        let mut preview = false;
        let mut replace = false;
        ui.add_enabled_ui(!busy, |ui| {
            ui.label(c.tr(Text::SubFindTitle));
            let label = ui.label(c.tr(Text::SubFind));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.find.query)
                    .id_salt("subtitles.find.query")
                    .hint_text(c.tr(Text::SubFind))
                    .char_limit(128)
                    .desired_width(ui.available_width()),
            );
            if r.has_focus() {
                self.text_focus = Some(r.id);
            }
            let r = r.labelled_by(label.id);
            c.remember("subtitles.find.query", &r);
            let label = ui.label(c.tr(Text::SubFindWith));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.find.replacement)
                    .id_salt("subtitles.find.replacement")
                    .hint_text(c.tr(Text::SubFindWith))
                    .char_limit(128)
                    .desired_width(ui.available_width()),
            );
            if r.has_focus() {
                self.text_focus = Some(r.id);
            }
            let r = r.labelled_by(label.id);
            c.remember("subtitles.find.replacement", &r);
            let r = crate::kit::toggle(
                ui,
                "subtitles.find.case",
                &mut self.find.options.case_sensitive,
                c.tr(Text::SubFindCase),
            );
            c.remember("subtitles.find.case", &r);
            let r = crate::kit::toggle(
                ui,
                "subtitles.find.whole",
                &mut self.find.options.whole_word,
                c.tr(Text::SubFindWhole),
            );
            c.remember("subtitles.find.whole", &r);
            let scope = if selected == 0 {
                c.tr(Text::SubFindScopeAll).to_owned()
            } else {
                i18n::format(
                    locale,
                    Text::SubFindScope,
                    &[("count", &i18n::number(locale, selected as u64))],
                )
            };
            ui.label(scope);
            preview = c.button(
                ui,
                "subtitles.find.preview",
                Text::SubFindPreview,
                Icon::MagnifyingGlass,
                !self.find.query.is_empty(),
            );
            match summary {
                Some(s) if s.matches == 0 => {
                    ui.label(c.tr(Text::SubFindNone));
                }
                Some(s) => {
                    ui.label(i18n::format(
                        locale,
                        Text::SubFindCount,
                        &[
                            ("matches", &i18n::number(locale, s.matches as u64)),
                            ("cues", &i18n::number(locale, s.cues as u64)),
                        ],
                    ));
                    for (count, text) in [
                        (s.protected, Text::SubFindProtected),
                        (s.emptied, Text::SubFindEmptied),
                        (s.too_long, Text::SubFindTooLong),
                    ] {
                        if count > 0 {
                            ui.label(i18n::format(
                                locale,
                                text,
                                &[("count", &i18n::number(locale, count as u64))],
                            ));
                        }
                    }
                }
                None => {}
            }
            if let Some(count) = done {
                ui.label(i18n::format(
                    locale,
                    Text::SubFindDone,
                    &[("count", &i18n::number(locale, count as u64))],
                ));
            }
            replace = c.button(
                ui,
                "subtitles.find.replace",
                Text::SubFindReplaceAll,
                Icon::PencilSimple,
                summary.is_some_and(|s| s.replaced > 0),
            );
        });
        if preview {
            let result = self
                .flush_drafts(session)
                .and_then(|_| self.find_preview(session));
            self.ui_result(result);
        }
        if replace {
            let result = self
                .flush_drafts(session)
                .and_then(|_| self.find_replace(session));
            self.ui_result(result.map(|_| ()));
        }
    }
}
