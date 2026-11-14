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
    /// Validate the request with no IO, then submit one synthesis job onto `runner` and
    /// return it. A refusal (unsupported language or voice, missing key, unusable cue) is
    /// typed and leaves the runner untouched. The job asks the consent prompt of `access`
    /// before any text leaves, so a caller whose prompt waits for a person never blocks
    /// here; a denial ends the job as a permission failure with nothing sent. The job
    /// output is an encoded take, read with `JobRunner::output_bytes` and `decode_take`.
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
        let prompt = access.consent();
        let prepared = self.prepare(&request)?;
        let spec = JobSpec {
            kind: JobKind::Synthesize,
            class: ResourceClass::Network,
            input,
            bytes: JobBytes {
                result: MAX_TAKE_BYTES,
                ..Default::default()
            },
        };
        let (id, _) = runner.submit(
            spec,
            move |context| -> std::result::Result<Vec<u8>, crate::providers::Error> {
                context.check_cancelled().map_err(|_| {
                    crate::providers::Error::Runtime(crate::RuntimeError::Cancelled)
                })?;
                let consent = prepared.approve(prompt.as_ref())?;
                let take = prepared.synthesize(&consent, context)?;
                Ok(encode_take(&take)?)
            },
        )?;
        Ok(id)
    }
}

impl CaptionAccess {
    /// Lend the narration workflow. It asks the consent prompt of this access, so no
    /// second consent path exists; the access needs no other resource (see
    /// [`CaptionAccess::consent_only`]).
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
