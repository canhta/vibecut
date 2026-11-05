#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::cache::ArtifactCache;
use engine::jobs::*;
use engine::loudness::{LoudnessJob, LoudnessResult, admit, submit_loudness};
use engine::{ByteBudget, EditError, Session};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{Tick, TimeRange};

/// A 997 Hz stereo tone at `level`; `fail` makes the first read error.
struct Tone {
    level: f32,
    reads: Arc<AtomicUsize>,
    fail: bool,
}
impl render::SourceProvider for Tone {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(render::Error::Invalid("source audio block"));
        }
        let mut out = render::AudioBlock::new(rate, 2, frames, render::MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                let at = (start + n as i64) as f64;
                *sample = self.level
                    * (2.0 * std::f64::consts::PI * 997.0 * at / f64::from(rate)).sin() as f32;
            }
        }
        Ok(out)
    }
}

fn session() -> Session {
    let mut data = project::Project::empty(project::ProjectId(1), "".into())
        .unwrap()
        .into_data();
    let seconds = Tick(4 * time::TICKS_PER_SECOND);
    data.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(1),
            path: std::env::temp_dir().join("loudness.wav"),
            content_hash: media::ContentHash::from_bytes(&[5]),
            probe: media::SourceProbe {
                format: "wav".into(),
                duration: Some(seconds),
                streams: vec![media::SourceStream {
                    index: 0,
                    kind: media::StreamKind::Audio,
                    codec: "pcm_f32le".into(),
                    time_base: media::TimeBase::new(1, 48000).unwrap(),
                    start: None,
                    duration: Some(seconds),
                    frame_rate: None,
                    frame_count: None,
                    variable_frame_rate: None,
                    video: None,
                    audio: Some(media::AudioInfo {
                        sample_rate: 48000,
                        channels: 2,
                        initial_padding: 0,
                        trailing_padding: 0,
                    }),
                }],
            },
        })
        .unwrap(),
    );
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![project::Item {
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, seconds).unwrap(),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            content: project::ItemParams::Audio(project::AudioParams {
                volume: 0.0,
                muted: false,
                source: Default::default(),
                fades: Default::default(),
            }),
        }],
    });
    Session::new(project::Project::new(data).unwrap(), 1).unwrap()
}

fn runner(budget: &ByteBudget) -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 2,
            retained: 2,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(10),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}

fn finish(
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    job: &LoudnessJob,
) -> Result<LoudnessResult, EditError> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        runner.tick().unwrap();
        if let Some(result) = job.finish(runner, cache)? {
            return Ok(result);
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn submit(
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    session: &Session,
    tone: (f32, &Arc<AtomicUsize>, bool),
) -> Result<LoudnessJob, EditError> {
    let (level, reads, fail) = tone;
    let reads = reads.clone();
    submit_loudness(runner, cache, session.project().clone(), move || {
        Box::new(Tone {
            level,
            reads: reads.clone(),
            fail,
        })
    })
}

#[test]
fn normalize_measures_applies_and_gates_export_on_unchanged_audio() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(256 << 20).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    let mut runner = runner(&budget);
    let reads = Arc::new(AtomicUsize::new(0));
    let mut session = session();
    session
        .execute(
            "audio.set_loudness",
            json!({"scope":"master","target_lufs":-14.0,"true_peak_db":-1.0}),
        )
        .unwrap();
    // Targets alone are not exportable: gains must be measured first.
    assert!(matches!(
        admit(session.project()),
        Err(EditError::LoudnessStale)
    ));

    let job = submit(&mut runner, &mut cache, &session, (0.1, &reads, false)).unwrap();
    let result = finish(&mut runner, &mut cache, &job).unwrap();
    assert!(
        (f64::from(result.gains.master_db) - 6.0).abs() < 0.05,
        "-20 LUFS to -14"
    );
    session
        .execute("audio.apply_loudness", json!({"gains": result.gains}))
        .unwrap();
    admit(session.project()).unwrap();

    // Identical content and targets reuse the cached measurement without decoding.
    let before = reads.load(Ordering::SeqCst);
    assert!(before > 0);
    let again = submit(&mut runner, &mut cache, &session, (0.1, &reads, false)).unwrap();
    assert_eq!(finish(&mut runner, &mut cache, &again).unwrap(), result);
    assert_eq!(reads.load(Ordering::SeqCst), before);

    // The master fader trims after normalization and needs no remeasure; an
    // audible edit or a new target invalidates the gains, and undo restores them.
    session
        .execute("audio.set_master", json!({"volume":-3.0,"muted":false}))
        .unwrap();
    admit(session.project()).unwrap();
    session
        .execute(
            "timeline.set_item_param",
            json!({"item":1,"patch":{"field":"volume","value":-3.0}}),
        )
        .unwrap();
    assert!(matches!(
        admit(session.project()),
        Err(EditError::LoudnessStale)
    ));
    session.undo().unwrap();
    admit(session.project()).unwrap();
    session
        .execute(
            "audio.set_loudness",
            json!({"scope":"master","target_lufs":-16.0,"true_peak_db":-1.0}),
        )
        .unwrap();
    assert!(matches!(
        admit(session.project()),
        Err(EditError::LoudnessStale)
    ));
}

#[test]
fn measurement_job_refuses_unset_targets_fails_typed_and_cancels() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(256 << 20).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    let mut runner = runner(&budget);
    let reads = Arc::new(AtomicUsize::new(0));
    let mut session = session();
    assert!(submit(&mut runner, &mut cache, &session, (0.1, &reads, false)).is_err());
    session
        .execute(
            "audio.set_loudness",
            json!({"scope":"clip","target_lufs":-14.0,"true_peak_db":-1.0}),
        )
        .unwrap();

    let failing = submit(&mut runner, &mut cache, &session, (0.1, &reads, true)).unwrap();
    assert!(matches!(
        finish(&mut runner, &mut cache, &failing),
        Err(EditError::JobFailure(error)) if matches!(*error, EditError::Render(render::Error::Invalid(_)))
    ));

    let cancelled = submit(&mut runner, &mut cache, &session, (0.1, &reads, false)).unwrap();
    cancelled.cancel.cancel();
    assert!(matches!(
        finish(&mut runner, &mut cache, &cancelled),
        Err(EditError::Cancelled)
    ));
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
