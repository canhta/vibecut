//! Prepares the captions workflow.
use crate::editor::{Editor, Result};

impl Editor {
    /// Initialize the panel without any download or provider construction.
    pub fn prepare_subtitles(&mut self) -> Result<()> {
        if self.subtitles.is_none() {
            self.subtitles = Some(crate::subtitles::Subtitles::new(
                self.playback.budget().clone(),
            )?);
        }
        Ok(())
    }
}
