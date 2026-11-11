#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::*;
use engine::{ByteBudget, ByteOwner, Command, EditError, Failure, RuntimeError as B, Session};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
#[derive(Default)]
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}
impl TestClock {
    fn advance(&self, millis: u64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}
fn key(n: u8) -> CacheKey {
    CacheKey::new(inputs(n)).unwrap()
}
fn inputs(n: u8) -> KeyInputs<'static> {
    KeyInputs {
        kind: ArtifactKind::AsrChunk,
        source: [n; 32],
        provider: "local",
        model: "test",
        version: "1",
        settings: b"{}",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    }
}
fn config() -> RunnerConfig {
    RunnerConfig {
        concurrency: [1, 1, 1],
        queue: 2,
        retained: 2,
        network_attempts: 3,
        backoff: Duration::from_millis(100),
        progress_interval: Duration::from_millis(10),
    }
}
fn runner(budget: &ByteBudget) -> (JobRunner, Arc<TestClock>) {
    let clock = Arc::new(TestClock::default());
    (
        JobRunner::new(config(), budget.clone(), clock.clone()).unwrap(),
        clock,
    )
}
fn spec(class: ResourceClass) -> JobSpec {
    JobSpec {
        kind: JobKind::Transcribe,
        class,
        input: key(1),
        bytes: JobBytes {
            decoded_frames: 100,
            result: 10,
            ..Default::default()
        },
    }
}
fn pump(r: &mut JobRunner, id: JobId) -> JobSnapshot {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        r.tick().unwrap();
        let s = r.snapshot(id).unwrap();
        if matches!(
            s.state,
            JobState::Completed | JobState::Cancelled | JobState::Failed | JobState::Backoff
        ) {
            return s;
        }
        assert!(Instant::now() < deadline, "job timed out");
        std::thread::yield_now();
    }
}
#[test]
fn cancellation_panics_failure_and_oversized_results_never_promote() {
    let budget = ByteBudget::new(1000).unwrap();
    let (mut r, _) = runner(&budget);
    let (started, start) = mpsc::sync_channel(1);
    let (id, cancel) = r
        .submit(spec(ResourceClass::Cpu), move |ctx| {
            started.send(()).unwrap();
            while !ctx.cancelled() {
                std::thread::yield_now();
            }
            // Hostile domain returns a partial success after observing cancellation.
            Ok(vec![1, 2])
        })
        .unwrap();
    r.tick().unwrap();
    start.recv_timeout(Duration::from_secs(10)).unwrap();
    cancel.cancel();
    assert_eq!(pump(&mut r, id).state, JobState::Cancelled);
    assert!(r.take_output(id).is_none());
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), |_| panic!("worker failure"))
        .unwrap();
    assert!(matches!(
        pump(&mut r, id).error.as_deref().and_then(|error| (error
            as &(dyn std::error::Error + 'static))
            .downcast_ref::<EditError>()),
        Some(EditError::Runtime(engine::RuntimeError::WorkerPanicked))
    ));
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), |_| Ok(vec![0; 11]))
        .unwrap();
    assert!(matches!(
        pump(&mut r, id).error.as_deref().and_then(|error| (error
            as &(dyn std::error::Error + 'static))
            .downcast_ref::<EditError>()),
        Some(EditError::Runtime(B::ArtifactTooLarge))
    ));
    let (id, token) = r
        .submit(spec(ResourceClass::Cpu), |_| {
            panic!("cancelled queued work ran")
        })
        .unwrap();
    token.cancel();
    assert_eq!(pump(&mut r, id).state, JobState::Cancelled);
    assert_eq!(budget.snapshot().unwrap().used, 0);
    for wrapped in [false, true] {
        let (id, _) = r
            .submit(spec(ResourceClass::Network), move |_| {
                Err(if wrapped {
                    EditError::Runtime(engine::RuntimeError::Failed(Arc::new(EditError::Cancelled)))
                } else {
                    EditError::Cancelled
                })
            })
            .unwrap();
        let snapshot = pump(&mut r, id);
        assert_eq!(
            (snapshot.state, snapshot.attempts),
            (
                if wrapped {
                    JobState::Failed
                } else {
                    JobState::Cancelled
                },
                1
            )
        );
        let cause = snapshot
            .error
            .as_deref()
            .and_then(|error| {
                (error as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>()
            })
            .unwrap();
        assert_eq!(cause.code(), engine::control::ErrorCode::Cancelled);
        assert_eq!(matches!(cause, EditError::Runtime(B::Failed(_))), wrapped);
        assert!(r.take_output(id).is_none());
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}

#[test]
fn worker_unwind_routes_keep_typed_failures_and_cancellation_order() {
    struct Capture(Arc<AtomicUsize>);
    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct PanicOnDrop(Arc<AtomicUsize>);
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("payload drop escaped the inner catch");
        }
    }
    for mode in 0..4 {
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, _) = runner(&budget);
        let captures = Arc::new(AtomicUsize::new(0));
        let payloads = Arc::new(AtomicUsize::new(0));
        let capture = Capture(captures.clone());
        let payload = payloads.clone();
        let (started, start) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let gate = std::sync::Mutex::new(gate);
        let (id, token) = r
            .submit(spec(ResourceClass::Network), move |ctx| {
                let _capture = &capture;
                if mode == 3 {
                    ctx.begin_publication()?;
                }
                started.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
                if mode == 1 {
                    std::panic::panic_any(PanicOnDrop(payload.clone()));
                }
                panic!("ordinary task panic");
            })
            .unwrap();
        r.tick().unwrap();
        start.recv_timeout(Duration::from_secs(10)).unwrap();
        if mode >= 2 {
            token.cancel();
            assert_eq!(token.cancelled(), mode == 2);
        }
        release.send(()).unwrap();
        let snapshot = pump(&mut r, id);
        assert_eq!(
            (snapshot.state, snapshot.attempts),
            (
                if mode == 2 {
                    JobState::Cancelled
                } else {
                    JobState::Failed
                },
                1
            )
        );
        let cause = snapshot.error.as_deref().unwrap();
        assert!(matches!(
            (
                mode,
                (cause as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>()
            ),
            (2, Some(EditError::Cancelled))
                | (0 | 1 | 3, Some(EditError::Runtime(B::WorkerPanicked)))
        ));
        assert_eq!(
            cause.code(),
            if mode == 2 {
                engine::control::ErrorCode::Cancelled
            } else {
                engine::control::ErrorCode::WorkerFailure
            }
        );
        assert!(!cause.transient());
        assert_eq!(
            cause.to_string(),
            if mode == 2 {
                "job cancelled"
            } else {
                "job worker unwound"
            }
        );
        assert!(cause.source().is_none());
        assert!(r.take_output(id).is_none());
        assert_eq!(budget.snapshot().unwrap().used, 0);
        assert_eq!(captures.load(Ordering::SeqCst), 1);
        assert_eq!(payloads.load(Ordering::SeqCst), usize::from(mode == 1));
    }
}

#[test]
fn submitted_raw_and_wrapped_causes_share_lifetime_past_retention_and_runner_drop() {
    #[derive(Debug)]
    struct DiskCause(Arc<AtomicUsize>);
    impl std::fmt::Display for DiskCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("owned disk cause")
        }
    }
    impl std::error::Error for DiskCause {}
    impl Drop for DiskCause {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for depth in 0..=2 {
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, _) = runner(&budget);
        let drops = Arc::new(AtomicUsize::new(0));
        let capture_drops = Arc::new(AtomicUsize::new(0));
        let capture = DiskCause(capture_drops.clone());
        let owned = drops.clone();
        let (id, _) = r
            .submit(spec(ResourceClass::Network), move |_| {
                let _capture = &capture;
                let mut error = EditError::Io(std::io::Error::other(DiskCause(owned.clone())));
                for _ in 0..depth {
                    error = EditError::Runtime(B::Failed(Arc::new(error)));
                }
                Err(error)
            })
            .unwrap();
        let snapshot = pump(&mut r, id);
        assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
        assert!(r.take_output(id).is_none());
        assert_eq!(budget.snapshot().unwrap().used, 0);
        assert_eq!(capture_drops.load(Ordering::SeqCst), 1);
        let raw = snapshot.error.unwrap();
        assert_eq!(raw.code(), engine::control::ErrorCode::Io);
        assert!(!raw.transient());
        assert_eq!(
            raw.to_string(),
            "background job failed: ".repeat(depth) + "job/cache IO failed"
        );
        let error: &(dyn std::error::Error + 'static) = raw.as_ref();
        assert_eq!(error.source().is_some(), depth == 0);
        let mut cause = raw.as_ref();
        for _ in 0..depth {
            let Some(EditError::Runtime(B::Failed(inner))) =
                (cause as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>()
            else {
                panic!("semantic wrapper or typed cause was lost");
            };
            cause = inner.as_ref();
        }
        let Some(EditError::Io(io)) =
            (cause as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>()
        else {
            panic!("original IO was normalized or discarded");
        };
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
        let payload = io.get_ref().unwrap().downcast_ref::<DiskCause>().unwrap();
        assert!(Arc::ptr_eq(&payload.0, &drops));
        let listed = r
            .snapshots()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
            .error
            .unwrap();
        let event = r
            .drain_events()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
            .error
            .unwrap();
        assert!(Arc::ptr_eq(&raw, &listed));
        assert!(Arc::ptr_eq(&raw, &event));
        let weak = Arc::downgrade(&raw);
        for _ in 0..3 {
            let (next, _) = r.submit(spec(ResourceClass::Cpu), |_| Ok(vec![])).unwrap();
            assert_eq!(pump(&mut r, next).state, JobState::Completed);
            drop(r.take_output(next).unwrap());
        }
        assert!(
            r.snapshot(id).is_none(),
            "original terminal entry must be evicted"
        );
        drop(r);
        assert_eq!(budget.snapshot().unwrap().used, 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(raw);
        drop(listed);
        assert!(weak.upgrade().is_some());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(event);
        assert!(weak.upgrade().is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn eligible_retry_respects_a_filled_queue_and_gated_publication_or_cancellation() {
    for mode in 0..3 {
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, _) = runner(&budget);
        let (started, start) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let gate = std::sync::Mutex::new(gate);
        let calls = Arc::new(AtomicUsize::new(0));
        let attempts = calls.clone();
        let (id, token) = r
            .submit(spec(ResourceClass::Network), move |context| {
                attempts.fetch_add(1, Ordering::SeqCst);
                if mode == 2 {
                    context.begin_publication()?;
                }
                started.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
                if mode == 1 {
                    context.begin_publication()?;
                }
                Err(EditError::Runtime(engine::RuntimeError::NetworkTransient))
            })
            .unwrap();
        r.tick().unwrap();
        start.recv_timeout(Duration::from_secs(10)).unwrap();
        if mode == 0 {
            for _ in 0..config().queue {
                // The active network worker keeps fillers queued until its join,
                // so an earlier tick cannot free the retry's pending slot.
                r.submit(spec(ResourceClass::Network), |_| Ok(vec![]))
                    .unwrap();
            }
            assert_eq!(r.counts().0, config().queue);
        } else {
            token.cancel();
            assert_eq!(token.cancelled(), mode == 1);
        }
        release.send(()).unwrap();
        let snapshot = pump(&mut r, id);
        assert_eq!(
            (snapshot.state, snapshot.attempts),
            (
                if mode == 1 {
                    JobState::Cancelled
                } else {
                    JobState::Failed
                },
                1
            )
        );
        let error = (snapshot.error.as_deref().unwrap() as &(dyn std::error::Error + 'static))
            .downcast_ref::<EditError>()
            .unwrap();
        assert!(matches!(
            (mode, error),
            (1, EditError::Cancelled)
                | (
                    0 | 2,
                    EditError::Runtime(engine::RuntimeError::NetworkTransient)
                )
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(r.counts().0 <= config().queue);
        assert!(r.take_output(id).is_none());
        drop(r);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
#[test]
fn retries_use_real_submission_and_deterministic_backoff_local_never_retries() {
    let budget = ByteBudget::new(1000).unwrap();
    let (mut r, clock) = runner(&budget);
    let count = Arc::new(AtomicUsize::new(0));
    let attempts = count.clone();
    let (id, _) = r
        .submit(spec(ResourceClass::Network), move |_| {
            if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(EditError::Runtime(engine::RuntimeError::NetworkTransient))
            } else {
                Ok(vec![9])
            }
        })
        .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Backoff);
    clock.advance(99);
    r.tick().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    clock.advance(1);
    assert_eq!(pump(&mut r, id).state, JobState::Backoff);
    clock.advance(199);
    r.tick().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    clock.advance(1);
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    assert_eq!(r.take_output(id).unwrap().bytes(), [9]);
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), |_| {
            Err(EditError::Runtime(engine::RuntimeError::NetworkTransient))
        })
        .unwrap();
    let snapshot = pump(&mut r, id);
    assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
    let raw = snapshot.error.as_ref().unwrap();
    assert!(raw.transient());
    assert_eq!(raw.code(), engine::control::ErrorCode::Job);
    assert_eq!(raw.to_string(), "transient network failure");
    assert!(raw.source().is_none());
    assert!(matches!(
        (raw.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>(),
        Some(EditError::Runtime(B::NetworkTransient))
    ));
    let event = r
        .drain_events()
        .into_iter()
        .find(|event| event.id == id)
        .unwrap();
    assert!(Arc::ptr_eq(raw, event.error.as_ref().unwrap()));
    assert!(r.take_output(id).is_none());
    let (id, _) = r
        .submit(spec(ResourceClass::Network), |_| {
            Err(EditError::Runtime(engine::RuntimeError::NetworkTransient))
        })
        .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Backoff);
    clock.advance(100);
    assert_eq!(pump(&mut r, id).state, JobState::Backoff);
    clock.advance(200);
    assert_eq!(pump(&mut r, id).state, JobState::Failed);
}
#[test]
fn ai_transient_failures_retry_with_retry_after_but_auth_fails_once() {
    for kind in [
        JobKind::Transcribe,
        JobKind::Translate,
        JobKind::SuggestClips,
    ] {
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, clock) = runner(&budget);
        let count = Arc::new(AtomicUsize::new(0));
        let attempts = count.clone();
        let mut suggestion = spec(ResourceClass::Network);
        suggestion.kind = kind;
        let (id, _) = r
            .submit(suggestion, move |_| {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(EditError::Ai(ai::Error::RateLimit {
                        retry_after_seconds: Some(3),
                    }))
                } else {
                    Ok(vec![7])
                }
            })
            .unwrap();
        assert_eq!(pump(&mut r, id).state, JobState::Backoff);
        clock.advance(2_999);
        r.tick().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        clock.advance(1);
        assert_eq!(pump(&mut r, id).state, JobState::Completed);
        assert_eq!(r.take_output(id).unwrap().bytes(), [7]);

        let (id, _) = r
            .submit(suggestion, |_| Err(EditError::Ai(ai::Error::Auth)))
            .unwrap();
        assert_eq!(pump(&mut r, id).attempts, 1);
        assert!(matches!(
            r.snapshot(id)
                .unwrap()
                .error
                .as_deref()
                .and_then(|error| (error as &(dyn std::error::Error + 'static))
                    .downcast_ref::<EditError>()),
            Some(EditError::Ai(ai::Error::Auth))
        ));

        let count = Arc::new(AtomicUsize::new(0));
        let attempts = count.clone();
        let (id, _) = r
            .submit(suggestion, move |_| {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(EditError::Ai(ai::Error::RateLimit {
                        retry_after_seconds: Some(3_600),
                    }))
                } else {
                    Ok(vec![8])
                }
            })
            .unwrap();
        assert_eq!(pump(&mut r, id).state, JobState::Backoff);
        clock.advance(299_999);
        r.tick().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        clock.advance(1);
        assert_eq!(pump(&mut r, id).state, JobState::Completed);
        assert_eq!(r.take_output(id).unwrap().bytes(), [8]);
    }
}

#[test]
fn bounded_queue_resource_classes_and_aggregate_bytes() {
    let budget = ByteBudget::new(250).unwrap();
    let (mut r, _) = runner(&budget);
    let hold = Arc::new(std::sync::Barrier::new(2));
    let gate = hold.clone();
    let (cpu, _) = r
        .submit(spec(ResourceClass::Cpu), move |_| {
            gate.wait();
            Ok(vec![])
        })
        .unwrap();
    r.tick().unwrap();
    let (cpu2, token) = r.submit(spec(ResourceClass::Cpu), |_| Ok(vec![])).unwrap();
    let (network, _) = r
        .submit(spec(ResourceClass::Network), |_| Ok(vec![]))
        .unwrap();
    let ran = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(());
    let released = Arc::downgrade(&captured);
    let calls = ran.clone();
    let before = (r.counts(), budget.snapshot().unwrap().used);
    assert!(matches!(
        r.submit(spec(ResourceClass::Cpu), move |_| {
            let _capture = &captured;
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }),
        Err(EditError::Runtime(engine::RuntimeError::QueueFull))
    ));
    assert_eq!((r.counts(), budget.snapshot().unwrap().used), before);
    assert!(released.upgrade().is_none());
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    r.tick().unwrap();
    assert_eq!(r.snapshot(cpu2).unwrap().state, JobState::Queued);
    assert_eq!(budget.snapshot().unwrap().used, 220);
    assert_eq!(pump(&mut r, network).state, JobState::Completed);
    token.cancel();
    hold.wait();
    assert_eq!(pump(&mut r, cpu).state, JobState::Completed);
    assert_eq!(pump(&mut r, cpu2).state, JobState::Cancelled);
    // Counts alone would allow a second class, but shared bytes prevent it.
    let held = budget.reserve(ByteOwner::Compositor, 200).unwrap();
    let (id, _) = r
        .submit(spec(ResourceClass::GpuEncoder), |_| Ok(vec![]))
        .unwrap();
    assert_eq!(id, JobId(network.0 + 1), "refusal consumed an ID");
    r.tick().unwrap();
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Queued);
    drop(held);
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
}
#[test]
fn progress_and_slow_consumer_retention_are_bounded_over_repeated_jobs() {
    let budget = ByteBudget::new(1000).unwrap();
    let (mut r, clock) = runner(&budget);
    for _ in 0..200 {
        let clock = clock.clone();
        let (id, _) = r
            .submit(spec(ResourceClass::Cpu), move |ctx| {
                assert!(ctx.progress(JobStage::Process, f32::NAN).is_err());
                for _ in 0..100 {
                    ctx.progress(JobStage::Process, 0.5)?;
                }
                clock.advance(10);
                ctx.progress(JobStage::Process, 0.75)?;
                Ok(vec![3])
            })
            .unwrap();
        assert_eq!(pump(&mut r, id).state, JobState::Completed);
        assert!(r.counts().2 <= 2);
        assert!(r.counts().3 <= 2);
        assert!(budget.snapshot().unwrap().used <= 20);
    }
    assert_eq!(r.drain_events().len(), 2);
    assert!(r.drain_events().is_empty());
    drop(r);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn stale_result_rejected_after_edit_undo_save_or_session_switch() {
    let p = project::Project::empty(project::ProjectId(0), "".into()).unwrap();
    let mut s = Session::new(p.clone(), 10).unwrap();
    let command = || Command::AddTrack {
        placement: engine::TrackPlacement::Auto,
        name: "".into(),
        kind: project::TrackKind::Video,
    };
    let before = s.job_target();
    s.apply(command()).unwrap();
    s.undo().unwrap();
    assert!(matches!(
        s.apply_job_result(&before, command()),
        Err(EditError::StaleResult)
    ));
    let before = s.job_target();
    s.saved().unwrap();
    assert!(matches!(
        s.apply_job_result(&before, command()),
        Err(EditError::StaleResult)
    ));
    let other = Session::new(p, 10).unwrap();
    assert!(matches!(
        s.apply_job_result(&other.job_target(), command()),
        Err(EditError::StaleResult)
    ));
    assert!(s.apply_job_result(&s.job_target(), command()).is_ok());
}
#[test]
fn content_identity_invalidates_every_semantic_input_without_path_identity() {
    let baseline = key(1);
    for field in 0..11 {
        let mut i = inputs(1);
        match field {
            0 => i.source = [9; 32],
            1 => i.provider = "remote",
            2 => i.model = "other",
            3 => i.version = "2",
            4 => i.settings = b"[]",
            5 => i.transcript = [1; 32],
            6 => i.cues = [1; 32],
            7 => i.clip_range = [1; 32],
            8 => i.translation = [1; 32],
            9 => i.kind = ArtifactKind::Transcript,
            _ => {
                i.provider = "loc";
                i.model = "altest";
            }
        }
        assert_ne!(CacheKey::new(i).unwrap(), baseline);
    }
}
#[test]
fn persistent_chunks_resume_and_corruption_is_a_typed_miss() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(4096).unwrap();
    {
        let (mut cache, n) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
        assert_eq!(n, 0);
        cache.put(key(1), b"complete chunk one").unwrap();
        // Simulated process termination: atomic writer debris cannot be a checkpoint.
        std::fs::write(dir.path().join(".vibecut-interrupted"), b"partial").unwrap();
    }
    let (mut cache, n) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        cache.get(key(1)).unwrap().unwrap().bytes(),
        b"complete chunk one"
    );
    assert!(cache.get(key(2)).unwrap().is_none());
    cache.put(key(2), b"complete chunk two").unwrap();
    let path = dir.path().join(key(1).filename());
    let mut corrupt = std::fs::read(&path).unwrap();
    corrupt[72] ^= 1;
    std::fs::write(path, corrupt).unwrap();
    assert!(matches!(
        cache.get(key(1)),
        Err(EditError::Runtime(engine::RuntimeError::CacheCorrupt))
    ));
    assert!(cache.get(key(1)).unwrap().is_none());
    assert_eq!(
        cache.get(key(2)).unwrap().unwrap().bytes(),
        b"complete chunk two"
    );
}
#[test]
fn byte_eviction_pins_clear_in_use_oversize_and_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(450).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(1), &[1; 28]).unwrap();
    cache.put(key(2), &[2; 28]).unwrap();
    let pinned = cache.get(key(1)).unwrap().unwrap();
    cache.put(key(3), &[3; 28]).unwrap(); // Need 200 staged+disk: evict unpinned key 2.
    assert!(cache.get(key(2)).unwrap().is_none());
    assert_eq!(pinned.bytes(), [1; 28]);
    assert_eq!(cache.clear().unwrap(), 1);
    assert_eq!(pinned.bytes(), [1; 28]);
    assert!(matches!(
        cache.put(key(4), &[0; 500]),
        Err(EditError::Runtime(B::ArtifactTooLarge))
    ));
    drop(pinned);
    assert_eq!(cache.clear().unwrap(), 0);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn hostile_cache_reads_preserve_unrelated_files_and_release_charges() {
    for fault in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let budget = ByteBudget::new(4096).unwrap();
        let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
        let sentinel = dir.path().join("unrelated");
        std::fs::write(&sentinel, b"preserve").unwrap();
        cache.put(key(1), b"complete").unwrap();
        let path = dir.path().join(key(1).filename());
        let mut bytes = std::fs::read(&path).unwrap();
        match fault {
            0 => bytes.truncate(73),
            1 => bytes[0] ^= 1,
            2 => bytes[8] ^= 1,
            3 => bytes.push(0),
            _ => std::fs::remove_file(&path).unwrap(),
        }
        if fault != 4 {
            std::fs::write(&path, bytes).unwrap();
        }
        assert!(matches!(
            cache.get(key(1)),
            Err(EditError::Runtime(B::CacheCorrupt))
        ));
        assert!(cache.get(key(1)).unwrap().is_none());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"preserve");
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join(".cache.lock");
    std::fs::create_dir(&lock).unwrap();
    assert!(matches!(
        ArtifactCache::open(dir.path(), ByteBudget::new(4096).unwrap(), 10),
        Err(EditError::Runtime(B::CacheCorrupt))
    ));
    assert!(lock.is_dir());
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, b"preserve").unwrap();
    assert!(matches!(
        ArtifactCache::open(&file, ByteBudget::new(4096).unwrap(), 10),
        Err(EditError::Io(_))
    ));
    assert_eq!(std::fs::read(file).unwrap(), b"preserve");
    std::fs::remove_dir(lock).unwrap();
    let foreign = dir.path().join(key(1).filename());
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join(".bundle"), b"truncated").unwrap();
    let (mut cache, diagnostics) =
        ArtifactCache::open(dir.path(), ByteBudget::new(4096).unwrap(), 10).unwrap();
    assert_eq!(diagnostics, 2); // foreign bundle and unrelated regular file
    assert_eq!(cache.clear().unwrap(), 0);
    assert_eq!(
        std::fs::read(foreign.join(".bundle")).unwrap(),
        b"truncated"
    );
}

#[test]
fn cache_failure_keeps_token_seal_order_and_raw_cause_custody() {
    for (class, cancel, seal) in [
        (ResourceClass::Cpu, false, false),
        (ResourceClass::Network, false, false),
        (ResourceClass::Network, true, false),
        (ResourceClass::Network, true, true),
    ] {
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, _) = runner(&budget);
        let (arrived, ready) = mpsc::sync_channel(0);
        let (resume, wait) = mpsc::sync_channel(0);
        let wait = std::sync::Mutex::new(wait);
        let (id, token) = r
            .submit(spec(class), move |context| {
                if seal {
                    context.begin_publication()?;
                }
                arrived.send(()).unwrap();
                wait.lock().unwrap().recv().unwrap();
                Err(B::CacheCorrupt.into())
            })
            .unwrap();
        r.tick().unwrap();
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        if cancel {
            token.cancel();
        }
        resume.send(()).unwrap();
        let snapshot = pump(&mut r, id);
        assert_eq!(snapshot.attempts, 1);
        let cancelled = cancel && !seal;
        assert_eq!(
            snapshot.state,
            if cancelled {
                JobState::Cancelled
            } else {
                JobState::Failed
            }
        );
        let cause = snapshot.error.as_ref().unwrap();
        let typed = (cause.as_ref() as &(dyn std::error::Error + 'static))
            .downcast_ref::<EditError>()
            .unwrap();
        assert!(matches!(
            (cancelled, typed),
            (true, EditError::Cancelled) | (false, EditError::Runtime(B::CacheCorrupt))
        ));
        assert!(!cause.transient());
        assert!(std::error::Error::source(typed).is_none());
        let weak = Arc::downgrade(cause);
        assert!(r.take_output(id).is_none());
        for _ in 0..3 {
            let (other, _) = r
                .submit(spec(ResourceClass::Cpu), |_| Ok(Vec::new()))
                .unwrap();
            pump(&mut r, other);
        }
        assert!(r.snapshot(id).is_none());
        drop(r);
        assert!(weak.upgrade().is_some());
        assert_eq!(budget.snapshot().unwrap().used, 0);
        drop(snapshot);
        assert!(weak.upgrade().is_none());
    }
}
#[test]
fn public_job_resumes_finalized_asr_chunks_after_worker_crash() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(10_000).unwrap();
    let (mut r, _) = runner(&budget);
    let (cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    let cache = Arc::new(std::sync::Mutex::new(cache));
    let checkpoint = cache.clone();
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), move |_| {
            checkpoint.lock().unwrap().put(key(1), b"one").unwrap();
            panic!("simulated crash after finalized chunk, before whole result");
        })
        .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Failed);
    assert!(r.take_output(id).is_none());
    drop(r);
    drop(cache);
    let (cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    let cache = Arc::new(std::sync::Mutex::new(cache));
    let (mut r, _) = runner(&budget);
    let executed = Arc::new(AtomicUsize::new(0));
    let count = executed.clone();
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), move |ctx| {
            let mut output = Vec::with_capacity(6);
            for chunk in 1..=2 {
                ctx.check_cancelled()?;
                let mut cache = cache.lock().unwrap();
                if let Some(lease) = cache.get(key(chunk))? {
                    output.extend_from_slice(lease.bytes());
                    continue;
                }
                count.fetch_add(1, Ordering::SeqCst);
                cache.put(key(chunk), b"two")?;
                output.extend_from_slice(b"two");
            }
            Ok(output)
        })
        .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    assert_eq!(executed.load(Ordering::SeqCst), 1);
    assert_eq!(r.take_output(id).unwrap().bytes(), b"onetwo");
}
#[test]
fn worker_progress_is_throttled_and_shutdown_joins_cancellable_work() {
    let budget = ByteBudget::new(1000).unwrap();
    let (mut r, clock) = runner(&budget);
    let (ready, observed) = mpsc::sync_channel(1);
    let (resume, next) = mpsc::sync_channel(1);
    let next = std::sync::Mutex::new(next);
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), move |ctx| {
            ai::ExecutionHooks::progress(ctx, ai::Stage::Transcribe, 25);
            ready.send(()).unwrap();
            next.lock().unwrap().recv().unwrap();
            ai::ExecutionHooks::progress(ctx, ai::Stage::Transcribe, 50);
            ready.send(()).unwrap();
            while !ctx.cancelled() {
                std::thread::yield_now();
            }
            Err(EditError::Cancelled)
        })
        .unwrap();
    r.tick().unwrap();
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    r.tick().unwrap();
    assert_eq!(r.snapshot(id).unwrap().progress, 0.0);
    clock.advance(10);
    resume.send(()).unwrap();
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    r.tick().unwrap();
    assert_eq!(r.snapshot(id).unwrap().progress, 0.5);
    assert_eq!(r.snapshot(id).unwrap().stage, JobStage::Transcribe);
    r.shutdown();
    assert_eq!(r.counts().0, 0);
    assert_eq!(r.counts().1, 0);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn cache_reopen_skips_bad_headers_and_changed_sizes_are_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(4096).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(1), b"good").unwrap();
    cache.put(key(2), b"other").unwrap();
    let path = dir.path().join(key(1).filename());
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(1000).unwrap();
    assert!(matches!(
        cache.get(key(1)),
        Err(EditError::Runtime(engine::RuntimeError::CacheCorrupt))
    ));
    let path = dir.path().join(key(2).filename());
    std::fs::write(path, [0; 80]).unwrap();
    drop(cache);
    let (cache, corrupt) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    assert_eq!(corrupt, 1);
    assert!(cache.is_empty());
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn submitted_output_carries_original_session_revision_through_completion() {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
        10,
    )
    .unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    let (mut r, _) = runner(&budget);
    let command = || Command::AddTrack {
        placement: engine::TrackPlacement::Auto,
        name: "".into(),
        kind: project::TrackKind::Video,
    };
    let (id, _) = r
        .submit_for_session(&session, spec(ResourceClass::Cpu), |_| Ok(vec![1]))
        .unwrap();
    session.apply(command()).unwrap();
    session.undo().unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    let output = r.take_output(id).unwrap();
    assert!(matches!(
        output.apply(&mut session, command()),
        Err(EditError::StaleResult)
    ));
    let (id, _) = r
        .submit_for_session(&session, spec(ResourceClass::Cpu), |_| Ok(vec![2]))
        .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    assert!(
        r.take_output(id)
            .unwrap()
            .apply(&mut session, command())
            .unwrap()
            .changed
    );
}
#[test]
fn fully_pinned_cache_refuses_pressure_and_repeated_inserts_remain_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(450).unwrap();
    std::fs::write(dir.path().join("unrelated.txt"), b"preserve").unwrap();
    let (mut cache, diagnostics) = ArtifactCache::open(dir.path(), budget.clone(), 3).unwrap();
    assert_eq!(diagnostics, 1);
    assert_eq!(
        std::fs::read(dir.path().join("unrelated.txt")).unwrap(),
        b"preserve"
    );
    cache.put(key(1), &[1; 28]).unwrap();
    cache.put(key(2), &[2; 28]).unwrap();
    let a = cache.get(key(1)).unwrap().unwrap();
    let b = cache.get(key(2)).unwrap().unwrap();
    assert!(matches!(
        cache.put(key(3), &[3; 28]),
        Err(EditError::Runtime(B::BudgetFull))
    ));
    assert_eq!(cache.clear().unwrap(), 2);
    assert_eq!(budget.snapshot().unwrap().used, 400);
    drop(a);
    drop(b);
    cache.clear().unwrap();
    for n in 0..100 {
        cache.put(key(n), &[n; 28]).unwrap();
        assert!(cache.len() <= 3);
        assert!(budget.snapshot().unwrap().used <= 300);
    }
    cache.clear().unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn every_group_exit_invalidates_captured_preview_targets_and_submitted_outputs() {
    for mode in 0..4 {
        let mut session = Session::new(
            project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
            10,
        )
        .unwrap();
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, _) = runner(&budget);
        let mut target = None;
        let mut job = None;
        let add = || Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "".into(),
            kind: project::TrackKind::Video,
        };
        let result: Result<(), EditError> = session.transaction(|s| {
            s.apply(add())?;
            target = Some(s.job_target());
            job = Some(
                r.submit_for_session(s, spec(ResourceClass::Cpu), |_| Ok(vec![1]))?
                    .0,
            );
            match mode {
                0 => Err(EditError::Cancelled),
                1 => panic!("unwind after preview target capture"),
                2 => {
                    let track = s.project().data().scene().timeline.overlays[0].id;
                    s.apply(Command::RemoveTrack { track })?;
                    Ok(())
                }
                _ => Ok(()),
            }
        });
        if mode < 2 {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        assert!(matches!(
            session.apply_job_result(&target.unwrap(), add()),
            Err(EditError::StaleResult)
        ));
        let id = job.unwrap();
        assert_eq!(pump(&mut r, id).state, JobState::Completed);
        assert!(matches!(
            r.take_output(id).unwrap().apply(&mut session, add()),
            Err(EditError::StaleResult)
        ));
        assert!(
            session
                .apply_job_result(&session.job_target(), add())
                .is_ok()
        );
    }
    // Net-zero explicit gestures also discard their internal preview identity.
    for cancel in [false, true] {
        let mut s = Session::new(
            project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
            10,
        )
        .unwrap();
        s.begin_gesture().unwrap();
        s.apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "".into(),
            kind: project::TrackKind::Video,
        })
        .unwrap();
        let track = s.project().data().scene().timeline.overlays[0].id;
        s.apply(Command::RemoveTrack { track }).unwrap();
        let target = s.job_target();
        if cancel {
            s.cancel_gesture().unwrap();
        } else {
            s.commit_gesture().unwrap();
        }
        assert!(matches!(
            s.apply_job_result(
                &target,
                Command::AddTrack {
                    placement: engine::TrackPlacement::Auto,
                    name: "".into(),
                    kind: project::TrackKind::Video
                }
            ),
            Err(EditError::StaleResult)
        ));
    }
}
#[test]
fn reopened_cache_admits_large_work_and_protects_pinned_readers() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    {
        let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
        for n in 0..9 {
            cache.put(key(n), &[n; 28]).unwrap();
        }
    }
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 900);
    assert!(matches!(
        budget.reserve(ByteOwner::Export, 1001),
        Err(B::ArtifactTooLarge)
    ));
    assert_eq!(budget.snapshot().unwrap().used, 900);
    let pinned = cache.get(key(0)).unwrap().unwrap(); // 1000, request itself cannot be evicted
    let (mut r, _) = runner(&budget);
    let mut request = spec(ResourceClass::Cpu);
    request.bytes = JobBytes {
        compositor: 800,
        ..Default::default()
    };
    let (id, _) = r.submit(request, |_| Ok(vec![])).unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    assert_eq!(pinned.bytes(), [0; 28]);
    assert!(cache.get(key(0)).unwrap().is_some());
    assert!(cache.len() <= 2);
    drop(pinned);
    cache.clear().unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 0);
    // Truly oversized work is rejected at admission; active pins are temporary pressure.
    request.bytes.compositor = 1001;
    assert!(matches!(
        r.submit(request, |_| Ok(vec![])),
        Err(EditError::Runtime(B::ArtifactTooLarge))
    ));
    drop(cache); // Release the exclusive root writer before reopening.
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(1), &[1; 328]).unwrap();
    let pin = cache.get(key(1)).unwrap().unwrap();
    request.bytes.compositor = 300;
    let (id, _) = r.submit(request, |_| Ok(vec![])).unwrap();
    r.tick().unwrap();
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Queued);
    assert_eq!(pin.bytes(), [1; 328]);
    drop(pin);
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
}
#[test]
fn read_reclaims_other_caches_by_global_recency_and_refuses_all_pinned_pressure() {
    for project_children in [false, true] {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let budget = ByteBudget::new(1000).unwrap();
        let first_budget = if project_children {
            budget.child(1000).unwrap()
        } else {
            budget.clone()
        };
        let second_budget = if project_children {
            budget.child(1000).unwrap()
        } else {
            budget.clone()
        };
        let (mut a, _) = ArtifactCache::open(first.path(), first_budget.clone(), 10).unwrap();
        let (mut b, _) = ArtifactCache::open(second.path(), second_budget, 10).unwrap();
        a.put(key(0), &[0; 328]).unwrap();
        for n in 1..=5 {
            b.put(key(n), &[n; 28]).unwrap();
        }
        let touched = b.get(key(1)).unwrap().unwrap();
        drop(touched);
        let large = a.get(key(0)).unwrap().unwrap();
        assert_eq!(large.bytes(), [0; 328]);
        assert!(b.get(key(2)).unwrap().is_none());
        assert!(b.get(key(1)).unwrap().is_some());
        drop(large);
        a.clear().unwrap();
        b.clear().unwrap();
        // Reopened heterogeneous reads use the same reclaimer, not a per-cache workaround.
        a.put(key(0), &[0; 328]).unwrap();
        for n in 1..=5 {
            a.put(key(n), &[n; 28]).unwrap();
        }
        drop(a);
        let (mut a, _) = ArtifactCache::open(first.path(), first_budget, 10).unwrap();
        assert_eq!(a.get(key(0)).unwrap().unwrap().bytes(), [0; 328]);
        a.clear().unwrap();
        a.put(key(1), &[1; 328]).unwrap();
        b.put(key(2), &[2; 128]).unwrap();
        let other = b.get(key(2)).unwrap().unwrap(); // 600 disk + 200 pin: only 200 free
        assert!(matches!(
            a.get(key(1)),
            Err(EditError::Runtime(B::BudgetFull))
        ));
        assert_eq!(other.bytes(), [2; 128]);
        drop(other);
        assert_eq!(a.get(key(1)).unwrap().unwrap().bytes(), [1; 328]);
    }
}

#[test]
fn concurrent_reclaimers_preserve_live_pins_and_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(0), &[0; 28]).unwrap();
    let pinned = cache.get(key(0)).unwrap().unwrap();
    for _ in 0..30 {
        for n in 1..=7 {
            cache.put(key(n), &[n; 28]).unwrap();
        }
        let start = Arc::new(std::sync::Barrier::new(3));
        let release = Arc::new(std::sync::Barrier::new(3));
        let (tx, rx) = mpsc::sync_channel(2);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let budget = budget.clone();
                let start = start.clone();
                let release = release.clone();
                let tx = tx.clone();
                scope.spawn(move || {
                    start.wait();
                    let result = budget.reserve(ByteOwner::Compositor, 400);
                    tx.send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|e| matches!(e, B::BudgetFull)),
                    )
                    .unwrap();
                    release.wait();
                    drop(result);
                });
            }
            start.wait();
            let results = [
                rx.recv_timeout(Duration::from_secs(10)).unwrap(),
                rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            ];
            let snapshot = budget.snapshot().unwrap();
            release.wait();
            assert!(results.iter().any(Result::is_ok));
            assert!(results.iter().all(|r| matches!(r, Ok(()) | Err(true))));
            assert!(snapshot.used <= snapshot.limit);
            assert_eq!(snapshot.owners.iter().sum::<u64>(), snapshot.used);
        });
        assert_eq!(pinned.bytes(), [0; 28]);
        assert!(cache.len() <= 8);
        assert_eq!(budget.snapshot().unwrap().owners[1], 0);
    }
    drop(pinned);
    cache.clear().unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn reclaim_io_failure_preserves_charge_and_can_retry_after_repair() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(0), &[0; 328]).unwrap();
    let path = dir.path().join(key(0).filename());
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let kind = match budget.reserve(ByteOwner::Export, 700) {
        Err(B::Io(error)) => error.kind(),
        _ => panic!("unlink failure must retain its typed runtime IO"),
    };
    assert_eq!(budget.snapshot().unwrap().used, 400);
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    assert!(
        matches!(render::ExecutionHooks::reserve(&hooks, render::MemoryKind::Export, 700),
        Err(render::Error::Io(error)) if error.kind() == kind)
    );
    assert_eq!(budget.snapshot().unwrap().used, 400);
    let (mut r, _) = runner(&budget);
    let mut session = crate::support::control::session();
    let mut cursor = session.snapshot_and_subscribe(&r).unwrap().cursor;
    let mut request = spec(ResourceClass::Network);
    request.bytes = JobBytes {
        compositor: 700,
        ..Default::default()
    };
    let (id, _) = r
        .submit(request, |_| panic!("failed admission must not run"))
        .unwrap();
    let failed = pump(&mut r, id);
    assert_eq!((failed.state, failed.attempts), (JobState::Failed, 0));
    assert!(
        matches!(failed.error.as_deref().and_then(|error| (error as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>()), Some(EditError::Runtime(B::Io(error))) if error.kind() == kind)
    );
    assert!(r.take_output(id).is_none());
    let state = session.snapshot_and_subscribe(&r).unwrap();
    assert_eq!(state.jobs[0].error, Some(engine::control::ErrorCode::Io));
    assert!(
        session
            .events(&mut cursor)
            .unwrap()
            .iter()
            .any(|event| matches!(
        &event.payload, engine::control::Payload::JobChanged { snapshot }
        if snapshot.id == id && snapshot.error == Some(engine::control::ErrorCode::Io)))
    );
    assert_eq!(budget.snapshot().unwrap().used, 400);
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let work = budget.reserve(ByteOwner::Export, 700).unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 700);
    assert!(cache.get(key(0)).unwrap().is_none());
    drop(work);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn corrupt_read_preserves_existing_pin_and_removes_entry_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(1), &[1; 28]).unwrap();
    let original = cache.get(key(1)).unwrap().unwrap();
    let path = dir.path().join(key(1).filename());
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[72] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        cache.get(key(1)),
        Err(EditError::Runtime(engine::RuntimeError::CacheCorrupt))
    ));
    assert_eq!(original.bytes(), [1; 28]);
    assert_eq!(cache.clear().unwrap(), 1);
    drop(original);
    assert!(matches!(
        cache.get(key(1)),
        Err(EditError::Runtime(engine::RuntimeError::CacheCorrupt))
    ));
    assert!(cache.get(key(1)).unwrap().is_none());
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn vision_inference_uses_the_existing_process_job_stage() {
    assert_eq!(JobStage::from(ai::Stage::Vision), JobStage::Process);
}

#[test]
fn import_jobs_expose_the_registered_probe_and_thumbnail_ids() {
    for (kind, id) in [
        (JobKind::Probe, "import.probe"),
        (JobKind::Thumbnail, "import.thumbnail"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(id));
        assert_eq!(
            serde_json::from_value::<JobKind>(serde_json::json!(id)).unwrap(),
            kind
        );
    }
}

#[test]
fn project_budget_children_account_globally_and_roll_back_refused_reservations() {
    let app = ByteBudget::new(1000).unwrap();
    assert!(app.child(0).is_err());
    let first = app.child(600).unwrap();
    let second = app.child(600).unwrap();
    let a = first.reserve(ByteOwner::Export, 600).unwrap();
    assert!(matches!(
        first.reserve(ByteOwner::DecodedFrames, 1),
        Err(B::BudgetFull)
    ));
    assert_eq!(
        app.snapshot().unwrap().used,
        600,
        "refused child reservation rolls back parent"
    );
    assert_eq!(first.snapshot().unwrap().used, 600);
    assert!(matches!(
        second.reserve(ByteOwner::Export, 401),
        Err(B::BudgetFull)
    ));
    assert_eq!(second.snapshot().unwrap().used, 0);
    let b = second.reserve(ByteOwner::Export, 400).unwrap();
    assert_eq!(app.snapshot().unwrap().used, 1000);
    drop(a);
    assert_eq!(first.snapshot().unwrap().used, 0);
    assert_eq!(app.snapshot().unwrap().used, 400);
    drop(b);
    assert_eq!(app.snapshot().unwrap().used, 0);
}

#[test]
fn cache_read_should_reclaim_unpinned_entries() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    cache.put(key(0), &[0; 328]).unwrap(); // 400-byte envelope
    for n in 1..=5 {
        cache.put(key(n), &[n; 28]).unwrap();
    } // 900 total
    assert_eq!(budget.snapshot().unwrap().used, 900);
    assert!(
        cache.get(key(0)).is_ok(),
        "valid 400-byte artifact cannot be read with 500 reclaimable cache bytes"
    );
}

#[test]
fn unpinned_cache_must_not_permanently_starve_admitted_job() {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(1000).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
    for n in 0..9 {
        cache.put(key(n), &[n; 28]).unwrap();
    }
    assert_eq!(budget.snapshot().unwrap().used, 900);
    let config = RunnerConfig {
        concurrency: [1; 3],
        queue: 2,
        retained: 2,
        network_attempts: 1,
        backoff: Duration::from_millis(10),
        progress_interval: Duration::from_millis(10),
    };
    let mut runner = JobRunner::new(config, budget, Arc::new(SystemClock::default())).unwrap();
    let (id, _) = runner
        .submit(
            JobSpec {
                kind: JobKind::Preview,
                class: ResourceClass::Cpu,
                input: key(0),
                bytes: JobBytes {
                    compositor: 200,
                    result: 0,
                    ..Default::default()
                },
            },
            |_| Ok(vec![]),
        )
        .unwrap();
    for _ in 0..100 {
        runner.tick().unwrap();
    }
    assert_ne!(
        runner.snapshot(id).unwrap().state,
        JobState::Queued,
        "idle runner never reclaims any of nine unpinned persistent entries"
    );
}

#[test]
fn classified_failures_reach_job_resync_without_broadening_retries() {
    use EditError as E;
    use engine::control::{ErrorCode as C, Payload};
    type Case = (fn() -> E, C, bool);
    let cases: &[Case] = &[
        (
            || E::Runtime(B::Failed(Arc::new(B::NetworkTransient))),
            C::Job,
            false,
        ),
        (
            || {
                E::Runtime(B::Failed(Arc::new(E::Runtime(B::Failed(Arc::new(
                    B::NetworkTransient,
                ))))))
            },
            C::Job,
            false,
        ),
        (|| E::Runtime(B::CacheCorrupt), C::Job, false),
        (
            || {
                let cause = E::Runtime(B::Failed(Arc::new(B::CacheCorrupt)));
                E::Runtime(B::Failed(Arc::new(cause)))
            },
            C::Job,
            false,
        ),
        (
            || E::Runtime(B::Failed(Arc::new(B::CacheCorrupt))),
            C::Job,
            false,
        ),
        (
            || E::Runtime(B::Failed(Arc::new(E::Runtime(B::CacheCorrupt)))),
            C::Job,
            false,
        ),
        (|| E::Runtime(B::QueueFull), C::QueueFull, false),
        (|| E::Runtime(B::WorkerPanicked), C::WorkerFailure, false),
        (
            || E::Runtime(B::Failed(Arc::new(E::Runtime(B::QueueFull)))),
            C::QueueFull,
            false,
        ),
        (
            || {
                let inner = E::Runtime(B::WorkerPanicked);
                let wrapped = E::Runtime(B::Failed(Arc::new(inner)));
                E::Runtime(B::Failed(Arc::new(wrapped)))
            },
            C::WorkerFailure,
            false,
        ),
        (
            || {
                E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Runtime(
                    engine::RuntimeError::Failed(Arc::new(E::Limit)),
                ))))
            },
            C::Limit,
            false,
        ),
        (
            || E::Runtime(engine::RuntimeError::Failed(Arc::new(E::StaleResult))),
            C::StaleResult,
            false,
        ),
        (
            || {
                E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Io(
                    std::io::Error::other("disk"),
                ))))
            },
            C::Io,
            false,
        ),
        (
            || {
                E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Render(
                    render::Error::Unavailable(Vec::new()),
                ))))
            },
            C::Unavailable,
            false,
        ),
        (
            || {
                E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Runtime(
                    engine::RuntimeError::NetworkTransient,
                ))))
            },
            C::Job,
            false,
        ),
        (
            || {
                E::Runtime(B::Failed(Arc::new(E::Ai(ai::Error::RateLimit {
                    retry_after_seconds: Some(3_600),
                }))))
            },
            C::Ai,
            false,
        ),
        (
            || {
                E::Sounds(engine::SoundError::RateLimit {
                    retry_after_seconds: Some(3),
                })
            },
            C::RateLimit,
            false,
        ),
        (|| E::Io(std::io::Error::other("io")), C::Io, false),
        (|| E::Ai(ai::Error::Incomplete), C::Ai, false),
        (
            || E::Runtime(engine::RuntimeError::NetworkTransient),
            C::Job,
            true,
        ),
    ];
    for &(make, code, seal) in cases {
        let budget = ByteBudget::new(1000).unwrap();
        let (mut r, _) = runner(&budget);
        let mut s = crate::support::control::session();
        let mut cursor = s.snapshot_and_subscribe(&r).unwrap().cursor;
        let (id, _) = r
            .submit(spec(ResourceClass::Network), move |context| {
                if seal {
                    context.begin_publication()?;
                }
                Err(make())
            })
            .unwrap();
        let snapshot = pump(&mut r, id);
        assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
        assert!(r.take_output(id).is_none());
        let state = s.snapshot_and_subscribe(&r).unwrap();
        assert_eq!(state.jobs[0].error, Some(code));
        assert!(s.events(&mut cursor).unwrap().iter().any(|event| matches!(&event.payload, Payload::JobChanged { snapshot } if snapshot.id == id && snapshot.error == Some(code))));
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
