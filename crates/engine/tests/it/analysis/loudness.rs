#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::jobs::*;
use engine::loudness::{LoudnessResult, admit, decode, submit_loudness};
use engine::{ByteBudget, Session, control::Error as EngineError};
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
        testkit::source::asset(1, std::env::temp_dir().join("loudness.wav"))
            .format("wav")
            .duration(seconds)
            .content_hash(media::ContentHash::from_bytes(&[5]))
            .stream(
                testkit::source::audio()
                    .codec("pcm_f32le")
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(seconds)
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![
            testkit::item::audio(project::ItemId(1))
                .asset(media::AssetId(1))
                .source(TimeRange::new(Tick::ZERO, seconds).unwrap())
                .build(),
        ],
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

fn finish(runner: &mut JobRunner, job: JobId) -> Result<LoudnessResult, EngineError> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        runner.tick().unwrap();
        let snapshot = runner.snapshot(job).unwrap();
        match snapshot.state {
            JobState::Cancelled => {
                return Err(EngineError::Runtime(engine::RuntimeError::Cancelled));
            }
            JobState::Failed => {
                return Err(EngineError::Runtime(engine::RuntimeError::Failed(
                    snapshot.error.unwrap(),
                )));
            }
            JobState::Completed => return Ok(decode(runner.output_bytes(job).unwrap())?),
            _ => {}
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn submit(
    runner: &mut JobRunner,
    session: &Session,
    tone: (f32, &Arc<AtomicUsize>, bool),
) -> Result<JobId, EngineError> {
    let (level, reads, fail) = tone;
    let reads = reads.clone();
    Ok(submit_loudness(
        runner,
        session.project().clone(),
        move || {
            Box::new(Tone {
                level,
                reads: reads.clone(),
                fail,
            })
        },
    )?)
}

#[test]
fn normalize_measures_applies_and_gates_export_on_unchanged_audio() {
    let budget = ByteBudget::new(256 << 20).unwrap();
    let mut runner = runner(&budget);
    let reads = Arc::new(AtomicUsize::new(0));
    let mut session = session();
    admit(session.project()).unwrap();
    session
        .apply(engine::Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -14.0,
            true_peak_db: -1.0,
        })
        .unwrap();
    // Targets alone are not exportable: gains must be measured first.
    assert!(matches!(
        admit(session.project()),
        Err(engine::RuntimeError::LoudnessStale)
    ));

    let job = submit(&mut runner, &session, (0.1, &reads, false)).unwrap();
    let result = finish(&mut runner, job).unwrap();
    assert!(
        (f64::from(result.gains.master_db) - 6.0).abs() < 0.05,
        "-20 LUFS to -14"
    );
    session
        .apply(engine::Command::ApplyLoudness {
            gains: result.gains.clone(),
        })
        .unwrap();
    admit(session.project()).unwrap();

    // The master fader trims after normalization and needs no remeasure; an
    // audible edit or a new target invalidates the gains, and undo restores them.
    session
        .apply(engine::Command::SetMaster {
            volume: -3.0,
            muted: false,
        })
        .unwrap();
    admit(session.project()).unwrap();
    session
        .apply(engine::Command::SetItemField {
            item: project::ItemId(1),
            patch: engine::ItemParamPatch::AudioGain(-3.0),
        })
        .unwrap();
    assert!(matches!(
        admit(session.project()),
        Err(engine::RuntimeError::LoudnessStale)
    ));
    session.undo().unwrap();
    admit(session.project()).unwrap();
    session
        .apply(engine::Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -16.0,
            true_peak_db: -1.0,
        })
        .unwrap();
    assert!(matches!(
        admit(session.project()),
        Err(engine::RuntimeError::LoudnessStale)
    ));
}

#[test]
fn measurement_job_refuses_unset_targets_fails_typed_and_cancels() {
    let budget = ByteBudget::new(256 << 20).unwrap();
    let mut runner = runner(&budget);
    let reads = Arc::new(AtomicUsize::new(0));
    let mut session = session();
    assert!(submit(&mut runner, &session, (0.1, &reads, false)).is_err());
    session
        .apply(engine::Command::SetLoudness {
            scope: project::LoudnessScope::Clip,
            target_lufs: -14.0,
            true_peak_db: -1.0,
        })
        .unwrap();

    let failing = submit(&mut runner, &session, (0.1, &reads, true)).unwrap();
    assert!(matches!(
        finish(&mut runner, failing),
        Err(EngineError::Runtime(engine::RuntimeError::Failed(error))) if matches!((error.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<engine::AnalysisError>(), Some(engine::AnalysisError::Render(render::Error::Invalid(_))))
    ));

    let cancelled = submit(&mut runner, &session, (0.1, &reads, false)).unwrap();
    runner.cancel_job(cancelled).unwrap();
    assert!(matches!(
        finish(&mut runner, cancelled),
        Err(EngineError::Runtime(engine::RuntimeError::Cancelled))
    ));
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
