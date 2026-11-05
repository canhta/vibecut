#![allow(clippy::unwrap_used, clippy::panic)]
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::*;
use engine::{ByteBudget, Command, EditError, Session};
use std::{sync::Arc, time::Duration};
fn key(n: u8) -> CacheKey {
    CacheKey::new(KeyInputs {
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
    })
    .unwrap()
}
fn add() -> Command {
    Command::AddTrack {
        placement: engine::TrackPlacement::Auto,
        name: "".into(),
        kind: project::TrackKind::Video,
    }
}
#[test]
fn rollback_must_invalidate_target_captured_in_discarded_preview() {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
        10,
    )
    .unwrap();
    let mut target = None;
    let result: Result<(), EditError> = session.transaction(|s| {
        s.apply(add())?;
        target = Some(s.job_target());
        Err(EditError::Cancelled)
    });
    assert!(result.is_err());
    assert!(
        session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .is_empty()
    );
    assert!(
        matches!(
            session.apply_job_result(&target.unwrap(), add()),
            Err(EditError::StaleResult)
        ),
        "discarded preview target was accepted against rolled-back content"
    );
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
