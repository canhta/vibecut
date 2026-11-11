//! Shared accounting policy; storage remains with its domain owner.
use super::{Error, Result};
use crate::cache::StoredArtifact;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};

/// Shared accounting categories. Native model disk usage has separate free-disk admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOwner {
    DecodedFrames,
    Compositor,
    Export,
    PersistentCache,
    CacheReads,
    JobResults,
    /// Reported installed/checkpoint model disk bytes; separately preflighted, not memory admission.
    ModelWeights,
}
impl ByteOwner {
    fn index(self) -> usize {
        match self {
            Self::DecodedFrames => 0,
            Self::Compositor => 1,
            Self::Export => 2,
            Self::PersistentCache => 3,
            Self::CacheReads => 4,
            Self::JobResults => 5,
            Self::ModelWeights => 6,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub limit: u64,
    pub used: u64,
    pub owners: [u64; 7],
}
impl BudgetSnapshot {
    /// Native model disk bytes are reported, but admitted by free-disk preflight.
    pub fn model_bytes(&self) -> u64 {
        self.owners
            .get(ByteOwner::ModelWeights.index())
            .copied()
            .unwrap_or(0)
    }
    /// Bytes constrained by limit; excludes separately reported native model disk usage.
    pub fn limited_used(&self) -> u64 {
        self.used.saturating_sub(self.model_bytes())
    }
}
/// Clone this handle into every cache/adapter; never instantiate one per cache.
#[derive(Clone)]
pub struct ByteBudget(Arc<BudgetState>);
struct BudgetState {
    parent: Option<ByteBudget>,
    accounting: Mutex<BudgetSnapshot>,
    artifacts: Mutex<Vec<Weak<StoredArtifact>>>,
    clock: AtomicU64,
}
impl ByteBudget {
    pub fn new(limit: u64) -> Result<Self> {
        Self::with_parent(limit, None)
    }
    fn with_parent(limit: u64, parent: Option<Self>) -> Result<Self> {
        if limit == 0 {
            return Err(Error::Limit);
        }
        Ok(Self(Arc::new(BudgetState {
            parent,
            accounting: Mutex::new(BudgetSnapshot {
                limit,
                used: 0,
                owners: [0; 7],
            }),
            artifacts: Mutex::new(Vec::new()),
            clock: AtomicU64::new(0),
        })))
    }
    /// Per-project admission with aggregate app accounting. Every child lease
    /// also retains a parent lease; refusal rolls back both reservations.
    pub fn child(&self, limit: u64) -> Result<Self> {
        Self::with_parent(limit, Some(self.clone()))
    }
    pub(crate) fn same_policy(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn snapshot(&self) -> Result<BudgetSnapshot> {
        Ok(self
            .0
            .accounting
            .lock()
            .map_err(|_| Error::Poisoned)?
            .clone())
    }
    pub(crate) fn stamp(&self) -> u64 {
        if let Some(parent) = &self.0.parent {
            return parent.stamp();
        }
        let mut current = self.0.clock.load(Ordering::Relaxed);
        loop {
            match self.0.clock.compare_exchange_weak(
                current,
                current.saturating_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return current,
                Err(next) => current = next,
            }
        }
    }

    pub(crate) fn register(&self, artifact: &Arc<StoredArtifact>) -> Result<()> {
        let mut artifacts = self.0.artifacts.lock().map_err(|_| Error::Poisoned)?;
        artifacts.retain(|entry| entry.upgrade().is_some_and(|a| !a.removed()));
        artifacts.push(Arc::downgrade(artifact));
        drop(artifacts);
        if let Some(parent) = &self.0.parent {
            parent.register(artifact)?;
        }
        Ok(())
    }
    /// Reserve before allocation. On pressure, reclaim unpinned persistent
    /// artifacts in aggregate LRU order. No accounting/registry lock spans IO.
    /// Active pins/work can still yield BudgetFull; IO failures retain their source.
    pub fn reserve(&self, owner: ByteOwner, bytes: u64) -> Result<ByteLease> {
        match self.try_reserve(owner, bytes) {
            Err(Error::BudgetFull) => {}
            result => return result,
        }
        let mut candidates: Vec<_> = {
            let mut artifacts = self.0.artifacts.lock().map_err(|_| Error::Poisoned)?;
            artifacts.retain(|entry| entry.upgrade().is_some_and(|a| !a.removed()));
            artifacts
                .iter()
                .filter_map(Weak::upgrade)
                .map(|artifact| (artifact.stamp(), artifact))
                .collect()
        };
        // Snapshot recency once: concurrent reads must not change sort keys.
        candidates.sort_by_key(|(stamp, _)| *stamp);
        for (_, artifact) in candidates {
            if artifact.evict()? {
                match self.try_reserve(owner, bytes) {
                    Err(Error::BudgetFull) => {}
                    result => return result,
                }
            }
        }
        Err(Error::BudgetFull)
    }
    fn try_reserve(&self, owner: ByteOwner, bytes: u64) -> Result<ByteLease> {
        let parent = self
            .0
            .parent
            .as_ref()
            .map(|p| p.reserve(owner, bytes).map(Box::new))
            .transpose()?;
        let mut state = self.0.accounting.lock().map_err(|_| Error::Poisoned)?;
        if owner != ByteOwner::ModelWeights && bytes > state.limit {
            return Err(Error::ArtifactTooLarge);
        }
        let next = state.used.checked_add(bytes).ok_or(Error::BudgetFull)?;
        let memory = state.limited_used();
        if owner != ByteOwner::ModelWeights
            && memory.checked_add(bytes).is_none_or(|v| v > state.limit)
        {
            return Err(Error::BudgetFull);
        }
        let count = state.owners.get_mut(owner.index()).ok_or(Error::Limit)?;
        *count = count.checked_add(bytes).ok_or(Error::BudgetFull)?;
        state.used = next;
        Ok(ByteLease {
            parent,
            budget: self.clone(),
            owner,
            bytes,
        })
    }
}
/// Keep alive for the entire allocation/file lifetime, including in-flight users.
pub struct ByteLease {
    parent: Option<Box<ByteLease>>,
    budget: ByteBudget,
    owner: ByteOwner,
    bytes: u64,
}
impl ByteLease {
    pub(crate) fn resize_models(&mut self, bytes: u64) -> Result<()> {
        if self.owner != ByteOwner::ModelWeights {
            return Err(Error::Invalid("model accounting owner"));
        }
        let mut state = self
            .budget
            .0
            .accounting
            .lock()
            .map_err(|_| Error::Poisoned)?;
        let next = state
            .used
            .checked_sub(self.bytes)
            .and_then(|n| n.checked_add(bytes))
            .ok_or(Error::BudgetFull)?;
        let count = state
            .owners
            .get_mut(self.owner.index())
            .ok_or(Error::Limit)?;
        let next_count = count
            .checked_sub(self.bytes)
            .and_then(|n| n.checked_add(bytes))
            .ok_or(Error::BudgetFull)?;
        if let Some(parent) = &mut self.parent {
            parent.resize_models(bytes)?;
        }
        *count = next_count;
        state.used = next;
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for ByteLease {
    fn drop(&mut self) {
        // Poison is never recovered: future admissions fail closed.
        if let Ok(mut state) = self.budget.0.accounting.lock()
            && let Some(count) = state.owners.get_mut(self.owner.index())
        {
            *count = count.saturating_sub(self.bytes);
            state.used = state.used.saturating_sub(self.bytes);
        }
    }
}

/// The one bridge from render's memory kinds to aggregate byte accounting.
/// Construct it beside the `ByteBudget` its caller owns; an underlying hooks
/// owner supplies cancellation, scheduling and progress. This adapter never
/// seals publication: an irreversible output keeps that fence with its job.
pub struct ByteBudgetHooks<'a> {
    budget: ByteBudget,
    execution: Option<&'a dyn render::ExecutionHooks>,
}

impl<'a> ByteBudgetHooks<'a> {
    pub fn new(budget: &ByteBudget, execution: Option<&'a dyn render::ExecutionHooks>) -> Self {
        Self {
            budget: budget.clone(),
            execution,
        }
    }
}

impl render::ExecutionHooks for ByteBudgetHooks<'_> {
    fn cancelled(&self) -> bool {
        self.execution
            .is_some_and(|execution| execution.cancelled())
    }
    fn parallelism(&self) -> usize {
        self.execution
            .map_or(1, |execution| execution.parallelism())
    }
    fn run_tasks(
        &self,
        tasks: &mut [&mut (dyn FnMut() -> render::Result<()> + Send)],
    ) -> render::Result<()> {
        match self.execution {
            Some(execution) => execution.run_tasks(tasks),
            None => {
                for task in tasks {
                    task()?;
                }
                Ok(())
            }
        }
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        if self.cancelled() {
            return Err(render::Error::Cancelled);
        }
        let owner = match kind {
            render::MemoryKind::Decoded => ByteOwner::DecodedFrames,
            render::MemoryKind::Compositor => ByteOwner::Compositor,
            render::MemoryKind::Export => ByteOwner::Export,
        };
        self.budget
            .reserve(owner, bytes)
            .map(|lease| Box::new(lease) as Box<dyn render::Reservation>)
            .map_err(|error| match error {
                Error::Io(source) => render::Error::Io(source),
                _ => render::Error::Budget {
                    requested: bytes,
                    limit: self
                        .budget
                        .snapshot()
                        .map_or(0, |snapshot| snapshot.limit.saturating_sub(snapshot.used)),
                },
            })
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        match self.execution {
            Some(execution) => execution.progress(stage, fraction),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use render::ExecutionHooks;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn adapter_charges_each_kind_to_its_owner_and_releases_on_drop() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let hooks = ByteBudgetHooks::new(&budget, None);
        for (kind, owner) in [
            (render::MemoryKind::Decoded, 0),
            (render::MemoryKind::Compositor, 1),
            (render::MemoryKind::Export, 2),
        ] {
            let lease = hooks.reserve(kind, 4096).unwrap();
            let snapshot = budget.snapshot().unwrap();
            assert_eq!(snapshot.owners[owner], 4096);
            assert_eq!(snapshot.used, 4096);
            drop(lease);
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
    }

    #[test]
    fn adapter_refuses_with_budget_and_the_real_remaining_bytes() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let hooks = ByteBudgetHooks::new(&budget, None);
        let held = budget.reserve(ByteOwner::Export, 1024).unwrap();
        let Err(error) = hooks.reserve(render::MemoryKind::Compositor, 2 << 20) else {
            panic!("oversized reservation was admitted");
        };
        assert!(matches!(
            error,
            render::Error::Budget {
                requested: 2_097_152,
                limit: 1_047_552,
            }
        ));
        drop(held);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }

    struct Probe(AtomicBool);

    impl render::ExecutionHooks for Probe {
        fn cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn reserve(
            &self,
            _: render::MemoryKind,
            _: u64,
        ) -> render::Result<Box<dyn render::Reservation>> {
            panic!("the adapter owns reservations");
        }
    }

    #[test]
    fn adapter_checks_its_optional_execution_owner_before_reserving() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let probe = Probe(AtomicBool::new(false));
        let hooks = ByteBudgetHooks::new(&budget, Some(&probe));
        assert!(!hooks.cancelled());
        assert!(hooks.reserve(render::MemoryKind::Decoded, 16).is_ok());
        assert_eq!(budget.snapshot().unwrap().used, 0);
        probe.0.store(true, Ordering::SeqCst);
        assert!(hooks.cancelled());
        assert!(matches!(
            hooks.reserve(render::MemoryKind::Decoded, 16),
            Err(render::Error::Cancelled)
        ));
    }

    #[test]
    fn poisoned_accounting_and_registry_refuse_without_leaking_parent_reservations() {
        for registry in [false, true] {
            let parent = ByteBudget::new(2000).unwrap();
            let child = parent.child(1000).unwrap();
            let held = child.reserve(ByteOwner::Export, 1000).unwrap();
            let poisoned = child.clone();
            assert!(
                std::thread::spawn(move || {
                    if registry {
                        let _guard = poisoned.0.artifacts.lock().unwrap();
                        panic!("injected artifact registry poison");
                    } else {
                        let _guard = poisoned.0.accounting.lock().unwrap();
                        panic!("injected accounting poison");
                    }
                })
                .join()
                .is_err()
            );
            assert!(matches!(
                child.reserve(ByteOwner::DecodedFrames, 1),
                Err(Error::Poisoned)
            ));
            assert_eq!(parent.snapshot().unwrap().used, 1000);
            if registry {
                assert_eq!(child.snapshot().unwrap().used, 1000);
            } else {
                assert!(matches!(child.snapshot(), Err(Error::Poisoned)));
            }
            drop(held);
            assert_eq!(parent.snapshot().unwrap().used, 0);
            if registry {
                assert_eq!(child.snapshot().unwrap().used, 0);
            } else {
                assert!(matches!(child.snapshot(), Err(Error::Poisoned)));
            }
        }
    }

    struct CheckpointAsr {
        config: ai::ProviderConfig,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        poison: Option<ByteBudget>,
    }
    impl ai::Transcriber for CheckpointAsr {
        fn config(&self) -> &ai::ProviderConfig {
            &self.config
        }
        fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
            ai::Capabilities {
                model: self.config.model().clone(),
                mode: "transcribe".into(),
                operation: ai::Operation::Transcribe,
                languages: vec![captions::LanguageTag::parse("en").unwrap()],
                target_languages: vec![],
                timing: std::collections::BTreeSet::from([ai::TimingUnit::Segment]),
                diarization: ai::Support::Unsupported,
                live_audio_input: ai::Support::Unsupported,
                file_result_stream: ai::Support::Unsupported,
                max_upload_bytes: None,
                max_duration_micros: None,
                evidence: ai::Evidence::FixtureVerified,
                revision: "fixture-1".into(),
            }
        }
        fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
            ai::CostEstimate::Free
        }
        fn transcribe(
            &self,
            audio: ai::AudioWindow<'_>,
            _: &ai::Request<'_>,
            _: &dyn ai::ExecutionHooks,
        ) -> std::result::Result<Vec<ai::RecognitionSegment>, ai::Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(budget) = self.poison.clone() {
                assert!(
                    std::thread::spawn(move || {
                        let _guard = budget.0.accounting.lock().unwrap();
                        panic!("checkpoint accounting poison after cache miss");
                    })
                    .join()
                    .is_err()
                );
            }
            Ok(vec![ai::RecognitionSegment {
                id: format!("fixture:{}", audio.range.start()).into(),
                text: "hello".into(),
                range: Some(audio.range),
                timing: vec![],
                raw_tokens: vec![],
            }])
        }
    }

    #[test]
    fn real_transcription_checkpoint_tolerates_only_pressure_not_io_or_poison() {
        use crate::{
            cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
            jobs::{JobRunner, RunnerConfig, SystemClock},
            providers::{AllowAll, Providers, TranscriptionProvider},
            transcript::{Cancellation, SilentProgress},
        };
        let Some(path) = testkit::oracle::generate(
            &testkit::fixture::dir("engine"),
            "runtime checkpoint",
            "runtime-checkpoint-2s.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x240:r=10:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ],
        ) else {
            return;
        };
        let duration = time::Tick(2 * time::TICKS_PER_SECOND);
        let asset = testkit::source::asset(1, path)
            .duration(duration)
            .stream(testkit::source::audio().build())
            .build();
        // 0: oversized checkpoint, 1: active-work pressure, 2: unlink IO, 3: poison.
        for case in 0..4 {
            let parent = ByteBudget::new(256 << 20).unwrap();
            let budget = parent.child(if case == 0 { 1 } else { 1 << 20 }).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 10).unwrap();
            let victim = CacheKey::new(KeyInputs {
                kind: ArtifactKind::Preview,
                source: [0; 32],
                provider: "fixture",
                model: "fixture",
                version: "1",
                settings: b"{}",
                transcript: [0; 32],
                cues: [0; 32],
                clip_range: [0; 32],
                translation: [0; 32],
            })
            .unwrap();
            let mut held = None;
            let mut repair = None;
            if case == 1 {
                held = Some(budget.reserve(ByteOwner::Export, 1 << 20).unwrap());
            } else if case == 2 {
                cache.put(victim, &[0; 328]).unwrap();
                held = Some(budget.reserve(ByteOwner::Export, (1 << 20) - 400).unwrap());
                let path = dir.path().join(victim.filename());
                let bytes = std::fs::read(&path).unwrap();
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
                repair = Some((path, bytes));
            }
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let provider = CheckpointAsr {
                config: ai::ProviderConfig::new(
                    "fixture".into(),
                    "fixture".into(),
                    "fixture-1".into(),
                    ai::Location::Local {
                        backend: "fixture".into(),
                        model_ref: "fixture".into(),
                    },
                )
                .unwrap(),
                calls: calls.clone(),
                poison: (case == 3).then(|| budget.clone()),
            };
            let providers = Providers::from_parts(
                TranscriptionProvider::Whisper {
                    model: "fixture".into(),
                },
                "transcribe".into(),
                Arc::new(provider),
                None,
                None,
                None,
            )
            .unwrap();
            let mut runner = JobRunner::new(
                RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 2,
                    retained: 2,
                    network_attempts: 3,
                    backoff: std::time::Duration::from_millis(100),
                    progress_interval: std::time::Duration::from_millis(10),
                },
                parent.clone(),
                Arc::new(SystemClock::default()),
            )
            .unwrap();
            let result = crate::transcript::transcribe(
                &asset,
                duration,
                &captions::LanguageTag::parse("en").unwrap(),
                &providers,
                &mut runner,
                &mut cache,
                &AllowAll,
                &SilentProgress,
                &Cancellation::new(),
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "must exercise the real checkpoint write"
            );
            match case {
                0 | 1 => assert!(!result.unwrap().cues.is_empty()),
                2 => {
                    assert!(matches!(
                        result,
                        Err(crate::EditError::Runtime(Error::Io(_)))
                    ));
                    assert_eq!(budget.snapshot().unwrap().used, 1 << 20);
                }
                3 => {
                    assert!(matches!(
                        result,
                        Err(crate::EditError::Runtime(Error::Poisoned))
                    ));
                    assert!(matches!(budget.snapshot(), Err(Error::Poisoned)));
                }
                _ => unreachable!(),
            }
            if let Some((path, bytes)) = repair {
                std::fs::remove_dir(&path).unwrap();
                std::fs::write(&path, bytes).unwrap();
            }
            drop(held);
            drop(cache);
            assert_eq!(parent.snapshot().unwrap().used, 0);
        }
    }
}
