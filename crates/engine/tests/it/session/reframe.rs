#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface behaviour assertions"
)]
//! Reframe command family: generate, apply, set_manual and clear.
use crate::support::control::{Allow, caller, finish, media_access, ok, request, runner};
use ai::vision::{FaceBox, FaceFrame, SceneFrame, SceneResult, SpeakerChunk, VisionDetector};
use engine::control::{
    Caller, ControlResult, ErrorCode, JobOutput, JobView, MediaAccess, Response, VisionAccess,
};
use engine::jobs::{JobId, JobRunner};
use engine::{Command, Session};
use project::{
    Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind, VideoParams,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use time::{SourceRange, Tick};

fn fixture() -> Option<PathBuf> {
    if !testkit::oracle::require("reframe commands") {
        return None;
    }
    let dir = testkit::fixture::dir("reframe-commands");
    testkit::oracle::generate(
        &dir,
        "reframe commands",
        "two-shots.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=1280x720:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=c=0x808080:s=1280x720:r=25:d=1",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0",
            "-c:v",
            "ffv1",
        ],
    )
}

fn asset(path: &Path) -> media::SourceAsset {
    media::SourceAsset::import(media::AssetId(97), path.to_owned()).expect("fixture video imports")
}

fn project(source: &media::SourceAsset) -> Project {
    let duration = source.probe().duration.expect("fixture duration");
    let mut data = Project::empty(ProjectId(1), "Reframe".into())
        .unwrap()
        .into_data();
    data.assets.push(source.clone());
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(8),
        name: "Reframe".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(8),
            source_asset: Some(media::AssetId(97)),
            source_range: SourceRange::new(Tick::ZERO, duration).expect("fixture range"),
            start: Tick::ZERO,
            content: ItemParams::Video(VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.0,
                muted: false,
                crop: None,
                cleanup: Default::default(),
                background: project::ClipBackground::None,
                reduce_noise: false,
            }),
        }],
    });
    Project::new(data).unwrap()
}

fn setup(
    source: &media::SourceAsset,
) -> (Session, JobRunner, Mutex<MediaAccess>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let media = Mutex::new(media_access(dir.path()));
    (
        Session::new(project(source), 10).unwrap(),
        runner(),
        media,
        dir,
    )
}

fn send(
    session: &mut Session,
    runner: &mut JobRunner,
    media: &Mutex<MediaAccess>,
    vision: &VisionAccess,
    id: &str,
    params: Value,
    key: &str,
) -> Response {
    let req = request(session, id, params, key);
    let allow = Allow;
    let mut call: Caller<'_> = caller(&allow);
    call.media = Some(media);
    call.vision = Some(vision);
    session.control_request(req, call, runner).unwrap()
}

fn accepted(response: Response) -> JobId {
    match ok(response) {
        ControlResult::JobAccepted { job, .. } => job,
        other => panic!("{other:?}"),
    }
}

fn view(
    session: &mut Session,
    runner: &mut JobRunner,
    media: &Mutex<MediaAccess>,
    vision: &VisionAccess,
    job: JobId,
    key: &str,
) -> JobView {
    match ok(send(
        session,
        runner,
        media,
        vision,
        "job_status",
        json!({"job":job}),
        key,
    )) {
        ControlResult::Job { snapshot } => snapshot,
        other => panic!("{other:?}"),
    }
}

fn report(view: JobView) -> engine::control::ReframeReport {
    match view.output {
        Some(JobOutput::Reframe(report)) => report,
        other => panic!("{other:?}"),
    }
}

fn item(session: &Session) -> &Item {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == ItemId(8))
        .unwrap()
}

#[test]
fn generate_reports_each_shot_and_edits_nothing() {
    let Some(path) = fixture() else { return };
    let source = asset(&path);
    let (mut session, mut runner, media, _dir) = setup(&source);
    let vision = engine::control::VisionAccess::new(|| {
        Ok((Box::new(NoFaces) as Box<dyn VisionDetector>, [9; 32]))
    });
    let before = session.project().clone();
    let history = session.history_len();
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.generate",
        json!({"item":8,"target":"9:16"}),
        "generate",
    ));
    finish(&mut runner, job);
    let report = report(view(
        &mut session,
        &mut runner,
        &media,
        &vision,
        job,
        "generate-status",
    ));
    assert_eq!(report.target, engine::control::ReframeTarget::Vertical);
    assert!(report.shots.len() >= 2, "{report:?}");
    assert!(
        report.shots.iter().all(|shot| {
            shot.reasons.len() == 1 && shot.reasons[0] == engine::control::ReframeReason::NoSubject
        }),
        "{report:?}"
    );
    assert!(report.shots.iter().all(|shot| shot.keys >= 2), "{report:?}");
    assert!(report.size[0] < 1.0 && report.size[1] == 1.0, "{report:?}");
    assert_eq!(session.project(), &before);
    assert_eq!(session.history_len(), history);
}

#[test]
fn apply_is_one_undo_step_and_restores_the_previous_crop_exactly() {
    let Some(path) = fixture() else { return };
    let source = asset(&path);
    let (mut session, mut runner, media, _dir) = setup(&source);
    let vision = engine::control::VisionAccess::new(|| {
        Ok((Box::new(NoFaces) as Box<dyn VisionDetector>, [9; 32]))
    });
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.generate",
        json!({"item":8,"target":"9:16"}),
        "generate",
    ));
    finish(&mut runner, job);
    let before = session.project().clone();
    let history = session.history_len();
    let applied = ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.apply",
        json!({"job":job,"replace_manual":false}),
        "apply",
    ));
    let ControlResult::Edit { outcome } = applied else {
        panic!("apply is an edit")
    };
    assert!(outcome.changed);
    assert_eq!(session.history_len().0, history.0 + 1);
    let after = session.project().clone();
    assert_ne!(session.project().as_ref(), before.as_ref());
    let target = item(&session);
    assert!(matches!(&target.content, ItemParams::Video(video) if video.crop.is_some()));
    for path in [
        project::BuiltinPath::CropX,
        project::BuiltinPath::CropY,
        project::BuiltinPath::CropAuto,
    ] {
        assert!(
            target
                .animations
                .get(&project::AnimationPath::from(path))
                .is_some(),
            "{path:?}"
        );
    }
    session.apply(Command::Undo {}).unwrap();
    assert_eq!(session.project(), &before);
    session.apply(Command::Redo {}).unwrap();
    assert_eq!(session.project(), &after);
}

#[test]
fn set_manual_keeps_the_shot_and_clear_removes_the_channels() {
    let Some(path) = fixture() else { return };
    let source = asset(&path);
    let (mut session, mut runner, media, _dir) = setup(&source);
    let vision = engine::control::VisionAccess::new(|| {
        Ok((Box::new(NoFaces) as Box<dyn VisionDetector>, [9; 32]))
    });
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.generate",
        json!({"item":8,"target":"9:16"}),
        "generate",
    ));
    finish(&mut runner, job);
    ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.apply",
        json!({"job":job,"replace_manual":false}),
        "apply",
    ));
    let applied = session.project().clone();
    let history = session.history_len();
    let manual = ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.set_manual",
        json!({"item":8,"at":0}),
        "manual",
    ));
    let ControlResult::Edit { outcome } = manual else {
        panic!("set_manual is an edit")
    };
    assert!(outcome.changed);
    assert_eq!(session.history_len().0, history.0 + 1);
    let auto = item(&session)
        .animations
        .get(&project::AnimationPath::from(
            project::BuiltinPath::CropAuto,
        ))
        .unwrap()
        .value_at(Tick::ZERO)
        .unwrap();
    assert_eq!(auto, project::ParamValue::Bool(false));
    session.apply(Command::Undo {}).unwrap();
    assert_eq!(session.project(), &applied);

    let history = session.history_len();
    let cleared = ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.clear",
        json!({"item":8}),
        "clear",
    ));
    let ControlResult::Edit { outcome } = cleared else {
        panic!("clear is an edit")
    };
    assert!(outcome.changed);
    assert_eq!(session.history_len().0, history.0 + 1);
    let target = item(&session);
    for path in [
        project::BuiltinPath::CropX,
        project::BuiltinPath::CropY,
        project::BuiltinPath::CropAuto,
    ] {
        assert!(
            target
                .animations
                .get(&project::AnimationPath::from(path))
                .is_none(),
            "{path:?}"
        );
    }
    assert!(matches!(&target.content, ItemParams::Video(video) if video.crop.is_some()));
    session.apply(Command::Undo {}).unwrap();
    assert_eq!(session.project(), &applied);
}

#[test]
fn refusals_change_nothing() {
    let Some(path) = fixture() else { return };
    let source = asset(&path);
    let (mut session, mut runner, media, _dir) = setup(&source);
    let vision = engine::control::VisionAccess::new(|| {
        Ok((Box::new(NoFaces) as Box<dyn VisionDetector>, [9; 32]))
    });
    let before = session.project().clone();
    let history = session.history_len();
    // No such retained job.
    error(
        send(
            &mut session,
            &mut runner,
            &media,
            &vision,
            "reframe.apply",
            json!({"job":99,"replace_manual":false}),
            "missing",
        ),
        ErrorCode::JobNotRetained,
    );
    // An overlay item has no video.
    error(
        send(
            &mut session,
            &mut runner,
            &media,
            &vision,
            "reframe.generate",
            json!({"item":1,"target":"9:16"}),
            "overlay",
        ),
        ErrorCode::Refused,
    );
    assert_eq!(session.project(), &before);
    assert_eq!(session.history_len(), history);

    // A locked track refuses the apply after a completed job.
    session
        .apply(Command::SetTrackLocked {
            track: TrackId(8),
            locked: true,
        })
        .unwrap();
    let locked = session.project().clone();
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.generate",
        json!({"item":8,"target":"9:16"}),
        "locked-generate",
    ));
    finish(&mut runner, job);
    error(
        send(
            &mut session,
            &mut runner,
            &media,
            &vision,
            "reframe.apply",
            json!({"job":job,"replace_manual":false}),
            "locked-apply",
        ),
        ErrorCode::Refused,
    );
    assert_eq!(session.project(), &locked);
    session
        .apply(Command::SetTrackLocked {
            track: TrackId(8),
            locked: false,
        })
        .unwrap();

    // Any edit after the job started makes its target stale.
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.generate",
        json!({"item":8,"target":"9:16"}),
        "changed-generate",
    ));
    finish(&mut runner, job);
    session
        .apply(Command::RenameProject {
            name: "Changed".into(),
        })
        .unwrap();
    let changed = session.project().clone();
    error(
        send(
            &mut session,
            &mut runner,
            &media,
            &vision,
            "reframe.apply",
            json!({"job":job,"replace_manual":false}),
            "changed-apply",
        ),
        ErrorCode::StaleResult,
    );
    assert_eq!(session.project(), &changed);
}

#[test]
fn stale_base_revision_and_unfinished_jobs_refuse() {
    let Some(path) = fixture() else { return };
    let source = asset(&path);
    let (mut session, mut runner, media, _dir) = setup(&source);
    let vision = engine::control::VisionAccess::new(|| {
        Ok((Box::new(NoFaces) as Box<dyn VisionDetector>, [9; 32]))
    });
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.generate",
        json!({"item":8,"target":"9:16"}),
        "generate",
    ));
    finish(&mut runner, job);
    let stale = request(
        &session,
        "reframe.apply",
        json!({"job":job,"replace_manual":false}),
        "stale-apply",
    );
    session
        .apply(Command::RenameProject {
            name: "Changed".into(),
        })
        .unwrap();
    let allow = Allow;
    let mut call = caller(&allow);
    call.media = Some(&media);
    call.vision = Some(&vision);
    error(
        session.control_request(stale, call, &mut runner).unwrap(),
        ErrorCode::StaleRevision,
    );
    // A running job cannot be applied.
    let calls = Arc::new(AtomicUsize::new(0));
    let blocked = {
        let calls = calls.clone();
        VisionAccess::new(move || {
            Ok((
                Box::new(Blocking(calls.clone())) as Box<dyn VisionDetector>,
                [8; 32],
            ))
        })
    };
    let job = accepted(send(
        &mut session,
        &mut runner,
        &media,
        &blocked,
        "reframe.generate",
        json!({"item":8,"target":"1:1"}),
        "blocked-generate",
    ));
    let deadline = Instant::now() + Duration::from_secs(60);
    while calls.load(Ordering::SeqCst) == 0 {
        runner.tick().unwrap();
        assert!(Instant::now() < deadline, "blocked detector never ran");
        std::thread::yield_now();
    }
    error(
        send(
            &mut session,
            &mut runner,
            &media,
            &blocked,
            "reframe.apply",
            json!({"job":job,"replace_manual":false}),
            "running-apply",
        ),
        ErrorCode::StaleResult,
    );
    runner.cancel_job(job).unwrap();
    finish(&mut runner, job);
}

#[test]
fn manual_shot_keys_survive_regeneration_until_replace_manual() {
    let Some(path) = fixture() else { return };
    let source = asset(&path);
    let (mut session, mut runner, media, _dir) = setup(&source);
    let vision = engine::control::VisionAccess::new(|| {
        Ok((Box::new(NoFaces) as Box<dyn VisionDetector>, [9; 32]))
    });
    let generate = |session: &mut Session, runner: &mut JobRunner, key: &str| {
        let job = accepted(send(
            session,
            runner,
            &media,
            &vision,
            "reframe.generate",
            json!({"item":8,"target":"9:16"}),
            key,
        ));
        finish(runner, job);
        job
    };
    let job = generate(&mut session, &mut runner, "manual-generate");
    ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.apply",
        json!({"job":job,"replace_manual":false}),
        "manual-apply",
    ));
    ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.set_manual",
        json!({"item":8,"at":0}),
        "manual-mark",
    ));
    let channel = project::Channel::new(
        project::ParamValue::Scalar(0.0),
        Vec::new(),
        project::Extrapolation::Hold,
        project::Extrapolation::Hold,
    )
    .unwrap();
    session
        .apply(Command::SetAnimatedParam {
            item: ItemId(8),
            path: project::AnimationPath::from(project::BuiltinPath::CropX),
            param: project::Param::Value(channel),
        })
        .unwrap();
    let job = generate(&mut session, &mut runner, "manual-regenerate");
    ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.apply",
        json!({"job":job,"replace_manual":false}),
        "manual-keep",
    ));
    let channel = item(&session)
        .animations
        .get(&project::AnimationPath::from(project::BuiltinPath::CropX))
        .unwrap()
        .channel(None)
        .unwrap();
    assert!(
        channel.keys().iter().all(|key| key.time > Tick::ZERO),
        "manual shot's position keys were replaced"
    );
    let job = generate(&mut session, &mut runner, "manual-replace-generate");
    ok(send(
        &mut session,
        &mut runner,
        &media,
        &vision,
        "reframe.apply",
        json!({"job":job,"replace_manual":true}),
        "manual-replace",
    ));
    let channel = item(&session)
        .animations
        .get(&project::AnimationPath::from(project::BuiltinPath::CropX))
        .unwrap()
        .channel(None)
        .unwrap();
    assert!(
        channel
            .keys()
            .first()
            .is_some_and(|key| key.time == Tick::ZERO),
        "replace_manual did not regenerate the first shot"
    );
}

fn error(response: Response, code: ErrorCode) {
    assert!(
        matches!(response.value, engine::control::ResponseValue::Error(value) if value == code),
        "{response:?}"
    );
}

struct NoFaces;
impl VisionDetector for NoFaces {
    fn faces(
        &mut self,
        _: FaceFrame<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<Vec<FaceBox>, ai::Error> {
        Ok(Vec::new())
    }
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<Vec<f32>, ai::Error> {
        Ok(vec![0.; chunk.frames()])
    }
    fn scene(
        &mut self,
        _: SceneFrame<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<SceneResult, ai::Error> {
        Ok(SceneResult::unknown())
    }
}

struct Blocking(Arc<AtomicUsize>);
impl VisionDetector for Blocking {
    fn faces(
        &mut self,
        _: FaceFrame<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> Result<Vec<FaceBox>, ai::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        while !hooks.cancelled() {
            std::thread::yield_now();
        }
        Err(ai::Error::Cancelled)
    }
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<Vec<f32>, ai::Error> {
        Ok(vec![0.; chunk.frames()])
    }
    fn scene(
        &mut self,
        _: SceneFrame<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<SceneResult, ai::Error> {
        Ok(SceneResult::unknown())
    }
}
