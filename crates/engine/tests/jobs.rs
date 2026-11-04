#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::*;
use engine::{ByteBudget, ByteOwner, Command, EditError, Session};
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
        pump(&mut r, id).error.as_deref(),
        Some(EditError::WorkerPanicked)
    ));
    let (id, _) = r
        .submit(spec(ResourceClass::Cpu), |_| Ok(vec![0; 11]))
        .unwrap();
    assert!(matches!(
        pump(&mut r, id).error.as_deref(),
        Some(EditError::ArtifactTooLarge)
    ));
    let (id, token) = r
        .submit(spec(ResourceClass::Cpu), |_| {
            panic!("cancelled queued work ran")
        })
        .unwrap();
    token.cancel();
    assert_eq!(pump(&mut r, id).state, JobState::Cancelled);
    assert_eq!(budget.snapshot().unwrap().used, 0);
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
                Err(EditError::NetworkTransient)
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
            Err(EditError::NetworkTransient)
        })
        .unwrap();
    assert_eq!(pump(&mut r, id).attempts, 1);
    let (id, _) = r
        .submit(spec(ResourceClass::Network), |_| {
            Err(EditError::NetworkTransient)
        })
        .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Backoff);
    clock.advance(100);
    assert_eq!(pump(&mut r, id).state, JobState::Backoff);
    clock.advance(200);
    assert_eq!(pump(&mut r, id).state, JobState::Failed);
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
    assert!(matches!(
        r.submit(spec(ResourceClass::Cpu), |_| Ok(vec![])),
        Err(EditError::QueueFull)
    ));
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
        name: "".into(),
        kind: project::TrackKind::Overlay,
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
    assert!(matches!(cache.get(key(1)), Err(EditError::CacheCorrupt)));
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
        Err(EditError::ArtifactTooLarge)
    ));
    drop(pinned);
    assert_eq!(cache.clear().unwrap(), 0);
    assert_eq!(budget.snapshot().unwrap().used, 0);
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
    assert!(matches!(cache.get(key(1)), Err(EditError::CacheCorrupt)));
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
        name: "".into(),
        kind: project::TrackKind::Overlay,
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
        Err(EditError::BudgetFull)
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
