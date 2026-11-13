//! Composes headless engine work under CLI admission settings.
mod control;
mod export;
mod job;
mod leased;
mod libraries;
mod media_cache;
mod paths;
mod registry;
mod runtime;

pub(crate) use control::{control, save_control_state};
#[cfg(test)]
use export::export_project_with;
pub(crate) use export::{Aspect, export_job_key, export_project, export_settings};
pub(crate) use job::{run_job, run_job_with};
pub(crate) use leased::{Edit, History, WritePolicy, run_leased};
pub(crate) use libraries::{
    prepare_project_libraries, sticker_lend, with_fonts, with_music, with_sounds, with_stickers,
};
pub(crate) use media_cache::{thumbnail_cache, waveform_cache};
pub(crate) use paths::{resolve_target, same_target};
pub(crate) use registry::{Lent, run as run_command, session as standalone_session};
pub(crate) use runtime::{
    caption_access, cli_runner, cli_runner_sized, cli_runtime, vision_access,
};

use crate::args::CliError;
use engine::ByteBudget;

/// Per-process CLI admission settings; each job keeps its existing independent budget.
pub(crate) struct Context {
    memory_limit: u64,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            memory_limit: 2 << 30,
        }
    }
}
impl Context {
    pub(crate) fn budget(&self) -> Result<ByteBudget, CliError> {
        Ok(ByteBudget::new(self.memory_limit).map_err(engine::control::Error::from)?)
    }
}
#[cfg(test)]
mod export_geometry {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use project::Project;
    use project::ProjectId;
    use project::atomic::Overwrite;
    #[test]
    fn portrait_and_landscape_override_only_output_geometry() {
        let project = Project::empty(ProjectId(0), "".into()).unwrap();
        let portrait = export_settings(&project, Some(Aspect::Portrait), false, false);
        assert_eq!(
            (portrait.render.width, portrait.render.height),
            (1080, 1920)
        );
        let landscape = export_settings(&project, Some(Aspect::Landscape), false, false);
        assert_eq!(
            (landscape.render.width, landscape.render.height),
            (1920, 1080)
        );
        assert_eq!(landscape.fps, project.data().canvas.fps);
        assert!(!landscape.allow_degraded);
        assert!(matches!(landscape.overwrite, Overwrite::Refuse));
    }
}

#[cfg(test)]
mod source_loss;
