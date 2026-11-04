#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::*;
use engine::{ByteBudget, EditError};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn key(n: u8) -> CacheKey {
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source: [n; 32],
        provider: "render",
        model: "cpu",
        version: "1",
        settings: b"{}",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap()
}
fn project() -> Arc<project::Project> {
    let mut d = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    d.canvas.width = 64;
    d.canvas.height = 64;
    d.timeline.tracks.push(project::Track {
        id: project::TrackId(0),
        name: "".into(),
        kind: project::TrackKind::Overlay,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: None,
            source_range: time::TimeRange::new(
                time::Tick::ZERO,
                time::Tick(time::TICKS_PER_SECOND),
            )
            .unwrap(),
            start: time::Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([30, 160, 90, 255]),
            }),
        }],
    });
    Arc::new(project::Project::new(d).unwrap())
}
fn runner(budget: &ByteBudget, clock: Arc<dyn Clock>) -> JobRunner {
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
        clock,
    )
    .unwrap()
}
fn complete(r: &mut JobRunner, id: JobId) -> JobSnapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        r.tick().unwrap();
        let s = r.snapshot(id).unwrap();
        if matches!(
            s.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return s;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn export_job_accounts_bytes_reclaims_cache_and_preserves_live_pin() {
    let dir = tempfile::tempdir().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(96 << 20).unwrap();
    let (mut cache, _) = ArtifactCache::open(cache_dir.path(), budget.clone(), 10).unwrap();
    let payload = vec![1; 8 << 20];
    for n in 0..8 {
        cache.put(key(n), &payload).unwrap();
    }
    let pin = cache.get(key(0)).unwrap().unwrap();
    let mut runner = runner(&budget, Arc::new(SystemClock::default()));
    let p = project();
    let mut settings = render::ExportSettings::from_project(&p);
    settings.include_audio = false;
    let path = dir.path().join("export.mp4");
    let (id, _) = engine::submit_export(
        &mut runner,
        key(9),
        p,
        time::TimeRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND)).unwrap(),
        settings,
        path.clone(),
    )
    .unwrap();
    let result = complete(&mut runner, id);
    assert_eq!(result.state, JobState::Completed, "{:?}", result.error);
    assert!(path.exists());
    assert!(cache.len() < 8);
    assert_eq!(pin.bytes(), payload);
    assert_eq!(budget.snapshot().unwrap().owners[0], 0);
    assert_eq!(budget.snapshot().unwrap().owners[1], 0);
    assert_eq!(budget.snapshot().unwrap().owners[2], 0);
    drop(pin);
    cache.clear().unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
struct PanicClock {
    owner: std::thread::ThreadId,
    calls: AtomicUsize,
}
impl Clock for PanicClock {
    fn now(&self) -> Duration {
        if std::thread::current().id() != self.owner
            && self.calls.fetch_add(1, Ordering::SeqCst) == 5
        {
            panic!("injected render worker unwind after staged frames");
        }
        Duration::from_millis(100)
    }
}
#[test]
fn panic_during_production_export_cleans_stage_and_reports_worker_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.mp4");
    std::fs::write(&path, b"previous").unwrap();
    let budget = ByteBudget::new(2 << 30).unwrap();
    let mut runner = runner(
        &budget,
        Arc::new(PanicClock {
            owner: std::thread::current().id(),
            calls: AtomicUsize::new(0),
        }),
    );
    let p = project();
    let mut settings = render::ExportSettings::from_project(&p);
    settings.overwrite = project::atomic::Overwrite::Replace;
    settings.include_audio = false;
    let (id, _) = engine::submit_export(
        &mut runner,
        key(0),
        p,
        time::TimeRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND)).unwrap(),
        settings,
        path.clone(),
    )
    .unwrap();
    assert!(matches!(
        complete(&mut runner, id).error.as_deref(),
        Some(EditError::WorkerPanicked)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
struct Fence<'a> {
    context: &'a JobContext,
    token: &'a CancelToken,
    before: bool,
}
impl render::ExecutionHooks for Fence<'_> {
    fn cancelled(&self) -> bool {
        self.context.cancelled()
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        render::ExecutionHooks::reserve(self.context, kind, bytes)
    }
    fn progress(&self, s: render::Stage, p: f32) -> render::Result<()> {
        render::ExecutionHooks::progress(self.context, s, p)
    }
    fn begin_publish(&self) -> render::Result<()> {
        if self.before {
            self.token.cancel();
        }
        let result = render::ExecutionHooks::begin_publish(self.context);
        if !self.before {
            self.token.cancel();
        }
        result
    }
}
#[test]
fn cancellation_before_publication_refuses_and_after_commit_point_completes() {
    for before in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp4");
        std::fs::write(&path, b"previous").unwrap();
        let budget = ByteBudget::new(2 << 30).unwrap();
        let mut runner = runner(&budget, Arc::new(SystemClock::default()));
        let p = project();
        let output = path.clone();
        let mut settings = render::ExportSettings::from_project(&p);
        settings.overwrite = project::atomic::Overwrite::Replace;
        settings.include_audio = false;
        let stored = Arc::new(std::sync::Mutex::new(None::<CancelToken>));
        let token = stored.clone();
        let (id, cancel) = runner
            .submit(
                JobSpec {
                    kind: JobKind::Export,
                    class: ResourceClass::GpuEncoder,
                    input: key(0),
                    bytes: JobBytes::default(),
                },
                move |ctx| {
                    let token = token.lock().unwrap().clone().unwrap();
                    render::export(
                        &p,
                        time::TimeRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND))
                            .unwrap(),
                        &settings,
                        &output,
                        &Fence {
                            context: ctx,
                            token: &token,
                            before,
                        },
                    )?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        *stored.lock().unwrap() = Some(cancel);
        assert_eq!(
            complete(&mut runner, id).state,
            if before {
                JobState::Cancelled
            } else {
                JobState::Completed
            }
        );
        let bytes = std::fs::read(&path).unwrap();
        if before {
            assert_eq!(bytes, b"previous");
        } else {
            assert_ne!(bytes, b"previous");
            assert!(media::probe(&path).is_ok());
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
#[test]
fn oversized_export_fails_with_requested_bytes_before_frame_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.mp4");
    let budget = ByteBudget::new(256 << 20).unwrap();
    let mut runner = runner(&budget, Arc::new(SystemClock::default()));
    let p = project();
    let mut settings = render::ExportSettings::from_project(&p);
    settings.render.width = 8192;
    settings.render.height = 8192;
    let (id, _) = engine::submit_export(
        &mut runner,
        key(0),
        p,
        time::TimeRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND)).unwrap(),
        settings,
        path.clone(),
    )
    .unwrap();
    let s = complete(&mut runner, id);
    assert!(matches!(
        s.error.as_deref(),
        Some(EditError::Render(render::Error::Budget {
            requested: 4362076160,
            ..
        }))
    ));
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
