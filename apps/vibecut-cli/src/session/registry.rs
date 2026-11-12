//! Runs one registry command for a standalone CLI adapter.
use crate::args::CliError;
use engine::control::{
    API_VERSION, Caller, ControlResult, FontAccess, MediaAccess, Policy, Request, SoundAccess,
    VisionAccess, registry_hash,
};
use engine::jobs::JobRunner;
use engine::subtitles::CaptionAccess;
use engine::{Command, Session};
use serde_json::Value;
use std::sync::Mutex;

/// Access objects a composition root lends to one request.
#[derive(Clone, Copy, Default)]
pub(crate) struct Lent<'a> {
    pub(crate) fonts: Option<&'a FontAccess<'a>>,
    pub(crate) sounds: Option<&'a SoundAccess<'a>>,
    pub(crate) media: Option<&'a Mutex<MediaAccess>>,
    pub(crate) captions: Option<&'a CaptionAccess>,
    /// Narrower trusted admission than the default allow-all of a one-shot session.
    pub(crate) policy: Option<&'a dyn Policy>,
    pub(crate) vision: Option<&'a VisionAccess>,
}

/// The adapter is the trusted composition root of its own one-shot session.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// A session that owns no project file: adapters for process libraries still
/// dispatch through the engine registry like every other command.
pub(crate) fn session() -> Result<Session, CliError> {
    Ok(Session::new(
        project::Project::empty(project::ProjectId(0), String::new().into())?,
        1,
    )?)
}

/// Run one command through control admission and return its typed result. The
/// in-process entry keeps a `SoundError`/`FontError` cause, so the adapter's
/// documented exit codes survive the registry round trip.
pub(crate) fn run(
    session: &mut Session,
    runner: &mut JobRunner,
    lent: Lent<'_>,
    command: &str,
    params: Value,
    effectful: bool,
) -> Result<ControlResult, CliError> {
    let request = Request {
        api_version: API_VERSION,
        registry_hash: Some(registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: command.into(),
        idempotency_key: effectful.then(|| command.into()),
        base_rev: effectful.then(|| session.revision()),
        command: command.into(),
        params,
    };
    let caller = Caller {
        principal: "vibecut-cli",
        external: false,
        policy: lent.policy.unwrap_or(&Trusted),
        fonts: lent.fonts,
        sounds: lent.sounds,
        media: lent.media,
        chapters: None,
        captions: lent.captions,
        vision: lent.vision,
    };
    session
        .control_typed(request, caller, runner)
        .map_err(CliError::from)
}
