//! The narration workflow a composition root lends to `dub.generate`.
use super::Result;
use crate::cache::CacheKey;
use crate::dubbing::{MAX_TAKE_BYTES, Narrator, encode_take};
use crate::jobs::{JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass};
use crate::subtitles::CaptionAccess;
use std::sync::Arc;

pub use crate::dubbing::Request as DubGenerateRequest;

/// The narration workflow, implemented by [`Narrator`] or a test double and lent through
/// [`CaptionAccess::with_dubbing`]. Admission, grants, receipts and revision checks run
/// before it is invoked.
pub trait DubWorkflow: Send + Sync {
    /// Ask for consent through `access`, then submit one synthesis job onto `runner` and
    /// return it. A refusal (unsupported language or voice, cost bound, denied consent) is
    /// typed and leaves the runner untouched. The job output is an encoded take.
    fn generate(
        &self,
        runner: &mut JobRunner,
        access: &CaptionAccess,
        request: DubGenerateRequest,
        input: CacheKey,
    ) -> Result<JobId>;
}

impl DubWorkflow for Narrator {
    fn generate(
        &self,
        runner: &mut JobRunner,
        access: &CaptionAccess,
        request: DubGenerateRequest,
        input: CacheKey,
    ) -> Result<JobId> {
        let (_, prompt) = access.share()?;
        let prepared = self.prepare(&request)?;
        let consent = prepared.approve(prompt.as_ref())?;
        let spec = JobSpec {
            kind: JobKind::Synthesize,
            class: ResourceClass::Network,
            input,
            bytes: JobBytes {
                result: MAX_TAKE_BYTES,
                ..Default::default()
            },
        };
        let (id, _) = runner.submit(spec, move |context| {
            context
                .check_cancelled()
                .map_err(|_| ai::Error::Cancelled)?;
            let take = prepared.synthesize(&consent, context)?;
            encode_take(&take)
        })?;
        Ok(id)
    }
}

impl CaptionAccess {
    /// Lend the narration workflow. It asks the consent prompt of this access, so no
    /// second consent path exists.
    pub fn with_dubbing(mut self, dubbing: Arc<dyn DubWorkflow>) -> Self {
        self.dubbing = Some(Arc::new(dubbing));
        self
    }
    /// The lent narration workflow, absent unless the composition root supplied one.
    pub fn dubbing(&self) -> Option<&Arc<dyn DubWorkflow>> {
        self.dubbing
            .as_ref()
            .and_then(|lent| lent.downcast_ref::<Arc<dyn DubWorkflow>>())
    }
}
