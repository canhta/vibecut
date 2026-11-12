//! The clip workflow the composition root lends to the registry.
use crate::suggestion::{submit_export, submit_suggest};
use engine::Session;
use engine::cache::CacheKey;
use engine::control::{
    ClipExportShort, ClipWorkflow, ClipsExportRequest, ClipsSuggestRequest, Error as EngineError,
    Result,
};
use engine::jobs::{JobId, JobRunner};
use engine::providers::Credentials;
use engine::subtitles::CaptionAccess;

/// Implements `clips.suggest` and `clips.export` over the process providers, consent and
/// checkpoint cache of the [`CaptionAccess`] it is lent through. It holds the one thing a
/// command may not carry: the credentials remote suggestion needs. Lend it with
/// `CaptionAccess::with_clips(Arc::new(ClipJobs::new(credentials)))`.
pub struct ClipJobs {
    credentials: Credentials,
}
impl ClipJobs {
    pub fn new(credentials: Credentials) -> Self {
        Self { credentials }
    }
}
impl ClipWorkflow for ClipJobs {
    fn suggest(
        &self,
        runner: &mut JobRunner,
        access: &CaptionAccess,
        session: &Session,
        request: ClipsSuggestRequest,
        input: CacheKey,
    ) -> Result<JobId> {
        submit_suggest(&self.credentials, runner, access, session, request, input)
    }

    fn export(
        &self,
        runner: &mut JobRunner,
        project: &project::Project,
        request: ClipsExportRequest,
    ) -> Result<Vec<ClipExportShort>> {
        submit_export(runner, project, request).map_err(EngineError::from)
    }
}
