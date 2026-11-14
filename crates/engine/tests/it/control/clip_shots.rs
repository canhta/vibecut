//! Clip export lays its shorts out from the cached per-shot analysis, or says why not.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "public-interface conformance assertions"
)]
use crate::support::control::{
    Allow, caller, finish, media_access, ok, request, runner_with_budget, seed, vision_access,
    with_video, write_video,
};
use engine::control::{self, ClipShotsDiagnostic, ControlResult};
use engine::jobs::{JobId, JobRunner};
use engine::{ByteBudget, Session, ShotAnalysis};
use serde_json::json;
use std::sync::{Arc, Mutex};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// The first 1.8 s of the 2 s fixture: one clip over a black-to-white cut at 1 s.
fn piece() -> SourceRange {
    SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND * 9 / 5)).unwrap()
}

/// A suggestion with one clip over the fixture, whose export records the shots it was given.
struct Recording(Arc<Mutex<Vec<Vec<ShotAnalysis>>>>);
impl control::ClipWorkflow for Recording {
    fn suggest(
        &self,
        runner: &mut JobRunner,
        _: &engine::subtitles::CaptionAccess,
        session: &Session,
        request: control::ClipsSuggestRequest,
        input: engine::cache::CacheKey,
    ) -> control::Result<JobId> {
        let result = control::ClipsResult {
            asset: request.asset.id(),
            source_language: captions::LanguageTag::parse("en").unwrap(),
            transcription_provider: "canned".into(),
            transcription_model: "canned".into(),
            provider: "heuristic".into(),
            model: "sentence-pause-keyword".into(),
            clips: vec![control::ClipCandidateView {
                title: "Clip".into(),
                hook_text: "".into(),
                reason: "".into(),
                score: 1.0,
                source: "heuristic".into(),
                label: ai::text::clips::ClipLabel::Standalone,
                duration: piece().duration(),
                segments: vec![control::ClipPieceRange {
                    range: piece(),
                    role: "payoff".into(),
                    source_order: 0,
                }],
                adjustments: Vec::new(),
                pieces: Vec::new(),
            }],
            rejected: Vec::new(),
            dropped: Vec::new(),
            cues: Vec::new(),
            energy: vec![control::ClipEnergy(
                SourceRange::new(Tick::ZERO, Tick(1)).unwrap(),
                0.5,
            )],
            transcript_segments: 0,
            checkpoints: 0,
            transcription_cost: control::CostView::Free,
            suggestion_cost: control::CostView::Free,
            elapsed_seconds: 0.0,
        };
        let bytes = serde_json::to_vec(&result).unwrap();
        let (job, _) = runner.submit_for_session(
            session,
            engine::jobs::JobSpec {
                kind: engine::jobs::JobKind::SuggestClips,
                class: engine::jobs::ResourceClass::Cpu,
                input,
                bytes: engine::jobs::JobBytes {
                    result: bytes.len() as u64,
                    ..Default::default()
                },
            },
            move |_| -> Result<Vec<u8>, control::Error> { Ok(bytes.clone()) },
        )?;
        Ok(job)
    }
    fn export(
        &self,
        _: &mut JobRunner,
        _: &project::Project,
        request: control::ClipsExportRequest,
    ) -> control::Result<Vec<control::ClipExportShort>> {
        self.0.lock().unwrap().push(request.shots.clone());
        Ok(vec![control::ClipExportShort {
            candidate: 0,
            job: JobId(0),
            video: request.directory.join("clip-0.mp4"),
            subtitles: Vec::new(),
        }])
    }
}

/// Runs `clips.suggest`, then `clips.export` three times and returns what each export
/// reported. `between` runs after each export, with its attempt (to finish the analysis job
/// it started or to change what the detector load sees).
fn exports(
    vision: Option<&control::VisionAccess>,
    between: impl Fn(usize, &mut JobRunner, Option<ClipShotsDiagnostic>),
) -> (Vec<Option<ClipShotsDiagnostic>>, Vec<Vec<ShotAnalysis>>) {
    let root = tempfile::tempdir().unwrap();
    let video = root.path().join("shots.mkv");
    write_video(&video);
    let mut s = with_video(&seed(), &video);
    // The shot-analysis job reserves 608 MiB, more than the default runner holds.
    let mut r = runner_with_budget(3 << 30);
    let media = Mutex::new(media_access(root.path()));
    let (cache, _) = engine::cache::ArtifactCache::open(
        &root.path().join("captions-cache"),
        ByteBudget::new(512 << 20).unwrap(),
        16,
    )
    .unwrap();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let captions = engine::subtitles::CaptionAccess::new(
        Arc::new(crate::subtitles::workflow::providers(
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            false,
            true,
        )),
        Arc::new(engine::providers::AllowAll),
        Arc::new(Mutex::new(cache)),
    )
    .with_clips(Arc::new(Recording(recorded.clone())));
    let allow = Allow;
    let access = || {
        let mut access = caller(&allow);
        access.captions = Some(&captions);
        access.media = Some(&media);
        access.vision = vision;
        access
    };
    let suggest = request(
        &s,
        "clips.suggest",
        json!({"asset":97,"source_language":"en"}),
        "shots-suggest",
    );
    let ControlResult::JobAccepted { job, .. } =
        ok(s.control_request(suggest, access(), &mut r).unwrap())
    else {
        panic!("clip suggestion job");
    };
    finish(&mut r, job);
    let mut diagnostics = Vec::new();
    for attempt in 0..3 {
        let export = request(
            &s,
            "clips.export",
            json!({"job":job,"select":[0],"directory":root.path(),"overwrite":false}),
            &format!("shots-export-{attempt}"),
        );
        let ControlResult::ClipExports { shorts, shots, .. } =
            ok(s.control_request(export, access(), &mut r).unwrap())
        else {
            panic!("clip exports");
        };
        assert_eq!(shorts.len(), 1);
        diagnostics.push(shots);
        between(attempt, &mut r, shots);
    }
    let recorded = recorded.lock().unwrap().clone();
    (diagnostics, recorded)
}

#[test]
fn an_export_without_a_detector_reports_it_and_keeps_the_centred_fill() {
    let (diagnostics, recorded) = exports(None, |_, _, _| {});
    assert_eq!(
        diagnostics,
        vec![Some(ClipShotsDiagnostic::DetectorUnavailable); 3]
    );
    assert!(recorded.iter().all(Vec::is_empty));
}

#[test]
fn an_export_without_model_weights_reports_them_missing() {
    let missing = control::VisionAccess::new(|| {
        Err(control::Error::Providers(
            engine::providers::Error::ModelMissing("yunet-2026may"),
        ))
    });
    let (diagnostics, recorded) = exports(Some(&missing), |_, _, _| {});
    assert_eq!(
        diagnostics,
        vec![Some(ClipShotsDiagnostic::ModelsMissing); 3]
    );
    assert!(recorded.iter().all(Vec::is_empty));
}

#[test]
fn an_export_during_a_model_transfer_reports_busy_then_lays_out_once_it_finishes() {
    let transfer = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let held = transfer.clone();
    let vision = control::VisionAccess::new(move || {
        if held.load(std::sync::atomic::Ordering::Acquire) {
            return Err(engine::RuntimeError::CacheBusy.into());
        }
        Ok((
            Box::new(crate::support::control::NoFaces) as Box<dyn ai::vision::VisionDetector>,
            [9; 32],
        ))
    });
    let (diagnostics, recorded) = exports(Some(&vision), |attempt, runner, shots| match attempt {
        // The download finishes between the first and second export.
        0 => transfer.store(false, std::sync::atomic::Ordering::Release),
        1 => {
            let Some(ClipShotsDiagnostic::Pending { job }) = shots else {
                panic!("the export after the transfer starts the analysis: {shots:?}");
            };
            finish(runner, job);
        }
        _ => {}
    });
    assert_eq!(diagnostics[0], Some(ClipShotsDiagnostic::ModelsBusy));
    assert!(matches!(
        diagnostics[1],
        Some(ClipShotsDiagnostic::Pending { .. })
    ));
    assert_eq!(diagnostics[2], None);
    assert!(recorded[0].is_empty() && recorded[1].is_empty());
    assert_eq!(recorded[2].len(), 2);
}

#[test]
fn an_export_runs_the_analysis_once_then_reuses_the_cached_shots() {
    let vision = vision_access();
    let (diagnostics, recorded) = exports(Some(&vision), |attempt, runner, first| {
        if attempt > 0 {
            return;
        }
        let Some(ClipShotsDiagnostic::Pending { job }) = first else {
            panic!("the first export starts the analysis: {first:?}");
        };
        finish(runner, job);
    });
    assert!(matches!(
        diagnostics[0],
        Some(ClipShotsDiagnostic::Pending { .. })
    ));
    assert_eq!(&diagnostics[1..], &[None, None]);
    assert!(recorded[0].is_empty());
    // The black-to-white cut at 1 s splits the 1.8 s piece into two shots.
    for shots in &recorded[1..] {
        let ranges: Vec<_> = shots.iter().map(|shot| shot.range).collect();
        assert_eq!(
            ranges,
            vec![
                SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)).unwrap(),
                SourceRange::new(Tick(TICKS_PER_SECOND), piece().end()).unwrap(),
            ]
        );
    }
}
