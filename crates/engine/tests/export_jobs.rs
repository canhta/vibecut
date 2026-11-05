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
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
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

fn caption_project() -> project::Project {
    use captions::{Cue, CueText, CueTrack, LanguageTag};
    use project::*;
    use time::{Tick, TimeRange};
    let mut d = project().as_ref().clone().into_data();
    d.canvas.width = 360;
    d.canvas.height = 640;
    let range = TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap();
    d.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            id: media::AssetId(1),
            path: std::env::temp_dir().join(format!(
                "subtitle-nonexistent-source-{}-{}",
                std::process::id(),
                {
                    static NEXT: AtomicUsize = AtomicUsize::new(0);
                    NEXT.fetch_add(1, Ordering::SeqCst)
                }
            )),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "caption identity".into(),
                duration: Some(Tick(time::TICKS_PER_SECOND * 3)),
                streams: vec![],
            },
        })
        .unwrap(),
    );
    let texts = [("vi", "Tiếng Việt"), ("zh-Hant", "繁體中文")]
        .into_iter()
        .map(|(lang, text)| {
            (
                LanguageTag::parse(lang).unwrap(),
                CueText {
                    text: text.into(),
                    edited: true,
                    translation_ref: None,
                },
            )
        })
        .collect();
    let cue = Cue {
        id: "cue".into(),
        range,
        source_lang: LanguageTag::parse("vi").unwrap(),
        texts,
    };
    // Two retained occurrences with an output gap; source is never opened for interchange.
    d.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
        name: "".into(),
        kind: TrackKind::Text,
        hidden: false,
        muted: false,
        locked: false,
        items: [0, 2]
            .into_iter()
            .enumerate()
            .map(|(n, at)| Item {
                masks: Vec::new(),
                effects: Vec::new(),
                id: ItemId(n as u64 + 1),
                animations: Default::default(),
                source_asset: Some(media::AssetId(1)),
                source_range: range,
                start: Tick(at * time::TICKS_PER_SECOND),
                content: ItemParams::Caption(CaptionParams {
                    cues: CueTrack {
                        cues: vec![cue.clone()],
                    },
                }),
            })
            .collect(),
    });
    Project::new(d).unwrap()
}
fn languages() -> Vec<captions::LanguageTag> {
    ["vi", "zh-Hant"]
        .into_iter()
        .map(|s| captions::LanguageTag::parse(s).unwrap())
        .collect()
}
#[test]
fn registered_subtitle_export_maps_repeated_items_and_preserves_gaps_without_media() {
    use time::Tick;
    for ordinary_title in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut data = caption_project().into_data();
        if ordinary_title {
            data.scene_mut()
                .timeline
                .overlays
                .iter_mut()
                .find(|track| track.id == project::TrackId(2))
                .unwrap()
                .items
                .insert(
                    1,
                    project::Item {
                        id: project::ItemId(3),
                        masks: Vec::new(),
                        effects: Vec::new(),
                        animations: Default::default(),
                        source_asset: None,
                        source_range: time::SourceRange::new(
                            Tick::ZERO,
                            Tick(time::TICKS_PER_SECOND),
                        )
                        .unwrap(),
                        start: Tick(time::TICKS_PER_SECOND),
                        content: project::ItemParams::Text(project::TextParams {
                            content: "Ordinary title".into(),
                            font_size: 32.0,
                            color: project::Color([255; 4]),
                        }),
                    },
                );
        }
        let session = engine::Session::new(project::Project::new(data).unwrap(), 10).unwrap();
        let budget = ByteBudget::new(16 << 20).unwrap();
        let mut r = runner(&budget, Arc::new(SystemClock::default()));
        for (n, format) in [captions::Format::Srt, captions::Format::WebVtt]
            .into_iter()
            .enumerate()
        {
            let path = dir.path().join(format!("字幕-{n}.txt"));
            let preview =
                engine::subtitle_file(session.project(), project::TrackId(2), format, &languages())
                    .unwrap();
            let (id, _) = session.execute_export(&mut r, key(40+n as u8), "export_subtitles", serde_json::json!({"track":2, "format":format, "languages":languages(), "path":path, "overwrite":false})).unwrap();
            let result = complete(&mut r, id);
            assert_eq!(result.state, JobState::Completed, "{:?}", result.error);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), preview.text);
            let imported = captions::parse(
                preview.text.as_bytes(),
                format,
                captions::LanguageTag::parse("vi").unwrap(),
            )
            .unwrap();
            assert_eq!(imported.track.cues.len(), 2);
            assert_eq!(
                imported.track.cues[1].range.start(),
                time::Tick(2 * time::TICKS_PER_SECOND)
            );
            assert!(preview.text.contains("Tiếng Việt\n繁體中文"));
            assert!(!preview.text.contains("Ordinary title"));
        }
        assert_eq!(session.history_len(), (0, 0));
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
#[test]
fn subtitle_export_refuses_alias_existing_output_and_batch_and_cancels_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.srt");
    std::fs::write(&path, b"previous").unwrap();
    let mut session = engine::Session::new(caption_project(), 10).unwrap();
    let budget = ByteBudget::new(16 << 20).unwrap();
    let mut r = runner(&budget, Arc::new(SystemClock::default()));
    let command = engine::Command::ExportSubtitles {
        track: project::TrackId(2),
        format: captions::Format::Srt,
        languages: languages(),
        path: path.clone(),
        overwrite: false,
    };
    assert!(
        session
            .apply(engine::Command::Batch {
                commands: vec![command.clone()]
            })
            .is_err()
    );
    assert_eq!(session.history_len(), (0, 0));
    let (id, _) = session
        .submit_export_command(&mut r, key(43), command.clone())
        .unwrap();
    assert_eq!(complete(&mut r, id).state, JobState::Failed);
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    let (id, token) = session
        .submit_export_command(&mut r, key(44), command)
        .unwrap();
    token.cancel();
    assert_eq!(complete(&mut r, id).state, JobState::Cancelled);
    let alias = session.project().data().assets[0].path().to_owned();
    let (id, _) = session
        .submit_export_command(
            &mut r,
            key(45),
            engine::Command::ExportSubtitles {
                track: project::TrackId(2),
                format: captions::Format::Srt,
                languages: languages(),
                path: alias.clone(),
                overwrite: true,
            },
        )
        .unwrap();
    let result = complete(&mut r, id);
    assert!(matches!(
        result.error.as_deref(),
        Some(EditError::Render(render::Error::Invalid(
            "output aliases a source"
        )))
    ));
    assert!(!alias.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn registered_burn_job_exports_and_typed_missing_media_preserves_previous_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("burn.mp4");
    let p = caption_project();
    let mut session = engine::Session::new(p, 10).unwrap();
    let budget = ByteBudget::new(256 << 20).unwrap();
    let mut r = runner(&budget, Arc::new(SystemClock::default()));
    let range =
        time::TimeRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND / 10)).unwrap();
    let command = engine::Command::BurnSubtitles {
        range,
        languages: languages(),
        style: render::CaptionStyle::default(),
        portrait: false,
        path: path.clone(),
        overwrite: true,
    };
    let (id, _) = session
        .submit_export_command(&mut r, key(46), command.clone())
        .unwrap();
    let result = complete(&mut r, id);
    assert_eq!(result.state, JobState::Completed, "{:?}", result.error);
    assert!(path.exists());
    // Turn the missing caption-identity asset into required video; production admission must refuse.
    let mut data = session.project().as_ref().clone().into_data();
    let mut asset = media::AssetData {
        id: data.assets[0].id(),
        path: data.assets[0].path().to_owned(),
        content_hash: data.assets[0].content_hash(),
        probe: data.assets[0].probe().clone(),
    };
    asset.probe.streams.push(media::SourceStream {
        index: 0,
        kind: media::StreamKind::Video,
        codec: "h264".into(),
        time_base: media::TimeBase::new(1, 30).unwrap(),
        duration: Some(time::Tick(3 * time::TICKS_PER_SECOND)),
        start: Some(time::Tick::ZERO),
        frame_rate: Some(time::FrameRate::FPS_30),
        frame_count: Some(90),
        variable_frame_rate: Some(false),
        audio: None,
        video: Some(media::VideoInfo {
            width: 360,
            height: 640,
            bit_depth: Some(8),
            color: media::ColorInfo::default(),
        }),
    });
    data.assets[0] = media::SourceAsset::try_from(asset).unwrap();
    let video = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find(|item| item.id == project::ItemId(0))
        .unwrap();
    video.source_asset = Some(media::AssetId(1));
    video.content = project::ItemParams::Video(project::VideoParams {
        source_audio_enabled: false,
    });
    session = engine::Session::new(project::Project::new(data).unwrap(), 10).unwrap();
    std::fs::write(&path, b"previous").unwrap();
    let (id, _) = session
        .submit_export_command(&mut r, key(47), command)
        .unwrap();
    let result = complete(&mut r, id);
    assert!(
        matches!(result.error.as_deref(), Some(EditError::Render(render::Error::Unavailable(details))) if details.len() == 1 && details[0].reason == render::UnavailableReason::Missing)
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

struct SubtitleFinalizationClock {
    owner: std::thread::ThreadId,
    calls: AtomicUsize,
    token: std::sync::Mutex<Option<CancelToken>>,
    panic: bool,
}
impl Clock for SubtitleFinalizationClock {
    fn now(&self) -> Duration {
        if std::thread::current().id() != self.owner
            && self.calls.fetch_add(1, Ordering::SeqCst) == 2
        {
            if self.panic {
                panic!("injected subtitle failure after stage finalization");
            }
            self.token.lock().unwrap().as_ref().unwrap().cancel();
        }
        Duration::from_millis(100)
    }
}
#[test]
fn subtitle_cancel_or_panic_after_finalization_preserves_destination_and_cleans_stage() {
    for panic in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.srt");
        std::fs::write(&path, b"previous").unwrap();
        let session = engine::Session::new(caption_project(), 10).unwrap();
        let budget = ByteBudget::new(16 << 20).unwrap();
        let clock = Arc::new(SubtitleFinalizationClock {
            owner: std::thread::current().id(),
            calls: AtomicUsize::new(0),
            token: std::sync::Mutex::new(None),
            panic,
        });
        let mut r = runner(&budget, clock.clone());
        let (id, token) = session
            .submit_export_command(
                &mut r,
                key(48),
                engine::Command::ExportSubtitles {
                    track: project::TrackId(2),
                    format: captions::Format::Srt,
                    languages: languages(),
                    path: path.clone(),
                    overwrite: true,
                },
            )
            .unwrap();
        *clock.token.lock().unwrap() = Some(token);
        let result = complete(&mut r, id);
        assert_eq!(
            result.state,
            if panic {
                JobState::Failed
            } else {
                JobState::Cancelled
            }
        );
        if panic {
            assert!(matches!(
                result.error.as_deref(),
                Some(EditError::WorkerPanicked)
            ));
        }
        assert_eq!(clock.calls.load(Ordering::SeqCst), 3);
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
