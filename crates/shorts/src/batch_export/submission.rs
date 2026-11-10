//! Schedules accepted short exports.
use super::{
    ArtifactKind, CacheKey, CancelToken, ClipBatch, ClipJob, EditError, Error, JobBytes, JobId,
    JobKind, JobRunner, JobSpec, JobStage, JobState, KeyInputs, Options, PreparedClip, Project,
    ResourceClass, Tick, TimeRange, Write, extension, mark_subtitle, mark_video,
    reconcile_published, subtitle_file,
};
impl ClipBatch {
    /// Submit one prepared short that has no job yet. An out-of-range or already
    /// submitted index is refused before the runner is touched, so callers that
    /// drain as they submit never leave an orphan admission behind.
    pub fn submit_one(&mut self, runner: &mut JobRunner, index: usize) -> Result<JobId, Error> {
        if self.jobs.get(index).is_some_and(Option::is_some) {
            return Err(EditError::Invalid("clip already submitted").into());
        }
        let clip = self
            .clips
            .get(index)
            .ok_or(EditError::Invalid("clip index"))?;
        let (id, cancel) = submit_clip(runner, clip)?;
        if let Some(slot) = self.jobs.get_mut(index) {
            *slot = Some(ClipJob { id, cancel });
        }
        Ok(id)
    }
}

impl ClipBatch {
    /// Submit every short that has no job yet. A repeated call submits nothing and
    /// returns an empty list. If submission fails midway, earlier jobs stay tracked
    /// and cancellable; nothing is published until a job runs.
    pub fn submit(&mut self, runner: &mut JobRunner) -> Result<Vec<JobId>, Error> {
        let mut submitted = Vec::new();
        submitted
            .try_reserve_exact(self.clips.len())
            .map_err(|_| EditError::Limit)?;
        for index in 0..self.clips.len() {
            if self.jobs.get(index).is_some_and(Option::is_some) {
                continue;
            }
            submitted.push(self.submit_one(runner, index)?);
        }
        Ok(submitted)
    }
}

impl ClipBatch {
    /// Resubmit a failed or cancelled short with its prepared inputs. Completed,
    /// running and unknown jobs are refused.
    pub fn retry(&mut self, runner: &mut JobRunner, index: usize) -> Result<JobId, Error> {
        let clip = self
            .clips
            .get(index)
            .ok_or(EditError::Invalid("clip index"))?;
        let current = self
            .jobs
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(EditError::Invalid("clip not submitted"))?;
        match runner.snapshot(current.id).map(|snapshot| snapshot.state) {
            Some(JobState::Failed | JobState::Cancelled) => {}
            _ => return Err(EditError::Invalid("clip is not retryable").into()),
        }
        let (id, cancel) = submit_clip(runner, clip)?;
        if let Some(slot) = self.jobs.get_mut(index) {
            *slot = Some(ClipJob { id, cancel });
        }
        Ok(id)
    }
}

impl ClipBatch {
    /// Cancel one short. Cancelling an already finished job has no effect.
    pub fn cancel(&self, index: usize) -> Result<(), Error> {
        let job = self
            .jobs
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(EditError::Invalid("clip not submitted"))?;
        job.cancel.cancel();
        Ok(())
    }
}

impl ClipBatch {
    pub fn cancel_all(&self) {
        for job in self.jobs.iter().flatten() {
            job.cancel.cancel();
        }
    }
}

pub(super) fn job_key(
    project: &Project,
    duration: Tick,
    settings: &render::ExportSettings,
    crop: Option<project::Crop>,
    options: &Options<'_>,
) -> Result<CacheKey, EditError> {
    let encoded = project::encode(project, false)?;
    let source = media::ContentHash::from_bytes(&encoded).0;
    let range = media::ContentHash::from_bytes(&duration.0.to_le_bytes()).0;
    let languages = options
        .languages
        .iter()
        .map(captions::LanguageTag::as_str)
        .collect::<Vec<_>>()
        .join(",");
    let formats = options
        .formats
        .iter()
        .map(|format| extension(*format))
        .collect::<Vec<_>>()
        .join(",");
    let crop = crop.map_or_else(String::new, |crop| {
        format!("{}:{}:{}:{}", crop.x, crop.y, crop.width, crop.height)
    });
    let settings_key = format!(
        "{}x{}@{}:{}:{}:{}:{}:{}:{}:{}:fade{}",
        settings.render.width,
        settings.render.height,
        settings.fps.num(),
        settings.fps.den(),
        settings.bit_rate,
        settings.quality,
        settings.include_audio,
        languages,
        formats,
        crop,
        settings.audio_cut_fade_ms,
    );
    // Export files are user artifacts, not cache entries; this key identifies the job.
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source,
        provider: "vibecut-engine",
        model: "clip-batch-export",
        version: "1",
        settings: settings_key.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: range,
        translation: [0; 32],
    })
}

fn submit_clip(
    runner: &mut JobRunner,
    clip: &PreparedClip,
) -> Result<(JobId, CancelToken), EditError> {
    let project = clip.project.clone();
    let settings = clip.settings.clone();
    let paths = clip.paths.clone();
    let caption_track = clip.caption_track;
    let languages = clip.languages.clone();
    let overwrite = clip.overwrite;
    let max_output_bytes = clip.max_output_bytes;
    let published = clip.published.clone();
    let sources = clip.sources.clone();
    let range = TimeRange::new(Tick::ZERO, clip.duration)?;
    runner.submit(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::GpuEncoder,
            input: clip.input,
            bytes: JobBytes {
                export: clip.sidecar_bytes,
                ..Default::default()
            },
        },
        move |context| {
            context.progress(JobStage::Admit, 0.0)?;
            let policy = if overwrite {
                project::atomic::Overwrite::Replace
            } else {
                project::atomic::Overwrite::Refuse
            };
            // Resume only missing artifacts; an artifact this short already
            // published is never overwritten again.
            let (video_done, subtitles_done) = reconcile_published(&published, &paths)?;
            // Refuse aliases and occupied destinations before staging or encoding.
            // Validate against the original snapshot, not the reduced derived
            // timeline, so every project source stays protected.
            for (index, subtitle) in paths.subtitles.iter().enumerate() {
                if subtitles_done.get(index).copied().unwrap_or(false) {
                    continue;
                }
                render::validate_output_path(&sources, &subtitle.path, context)?;
            }
            if !video_done {
                render::validate_output_path(&sources, &paths.video, context)?;
            }
            // Stage unpublished sidecars first: a later MP4 refusal leaves no
            // subtitle artifact.
            let mut staged = Vec::new();
            staged
                .try_reserve_exact(paths.subtitles.len())
                .map_err(|_| EditError::Limit)?;
            for (index, subtitle) in paths.subtitles.iter().enumerate() {
                if subtitles_done.get(index).copied().unwrap_or(false) {
                    continue;
                }
                context.check_cancelled()?;
                let track = caption_track.ok_or(EditError::Invalid("clip subtitle track"))?;
                let export = subtitle_file(&project, track, subtitle.format, &languages)?;
                let stage =
                    project::atomic::StreamingStage::new(&subtitle.path, policy, max_output_bytes)?;
                {
                    let mut writer = stage.writer()?;
                    for chunk in export.text.as_bytes().chunks(65536) {
                        context.check_cancelled()?;
                        writer.write_all(chunk)?;
                    }
                }
                staged.push((index, stage.finalize()?));
            }
            // The MP4 is the primary artifact and publishes through render's fence.
            if !video_done {
                render::export(&project, range, &settings, &paths.video, context)?;
                mark_video(&published)?;
            }
            for (index, stage) in staged {
                // A retry may skip render and its publication fence entirely.
                context.commit_publication(|| {
                    stage.publish()?;
                    mark_subtitle(&published, index)
                })?;
            }
            Ok(Vec::new())
        },
    )
}
