#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "validated synthetic and oracle fixtures"
)]
use ai::clips::{ClipRole, ClipSourceSegment};
use captions::{Cue, CueText, CueTrack, Format, LanguageTag};
use engine::jobs::{JobRunner, JobSnapshot, JobState, RunnerConfig, SystemClock};
use engine::{ByteBudget, ClipBatch, ClipBatchRequest, ClipShort, EditError};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream,
    StreamKind, TimeBase, VideoInfo,
};
use project::{Project, ProjectId};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{FrameRate, Rounding, Tick, TimeRange};

fn lang(value: &str) -> LanguageTag {
    LanguageTag::parse(value).unwrap()
}
fn seconds(value: i64) -> Tick {
    Tick::from_units(value, 1, Rounding::Nearest).unwrap()
}
fn range(start: i64, end: i64) -> TimeRange {
    TimeRange::new(seconds(start), seconds(end)).unwrap()
}
fn tenths(value: i64) -> Tick {
    Tick::from_units(value, 10, Rounding::Nearest).unwrap()
}
fn short_range(start: i64, end: i64) -> TimeRange {
    TimeRange::new(tenths(start), tenths(end)).unwrap()
}
fn segment(start: i64, end: i64, order: usize) -> ClipSourceSegment {
    ClipSourceSegment {
        range: range(start, end),
        role: ClipRole::Payoff,
        source_order: order,
        repair: None,
    }
}
fn stream(index: usize, kind: StreamKind) -> SourceStream {
    let video = (kind == StreamKind::Video).then(|| VideoInfo {
        width: 320,
        height: 180,
        bit_depth: Some(8),
        color: ColorInfo::default(),
    });
    let audio = (kind == StreamKind::Audio).then_some(AudioInfo {
        sample_rate: 48_000,
        channels: 2,
        initial_padding: 0,
        trailing_padding: 0,
    });
    SourceStream {
        index,
        kind,
        codec: "fixture".into(),
        time_base: TimeBase::new(1, 30).unwrap(),
        duration: None,
        start: Some(Tick::ZERO),
        frame_rate: (kind == StreamKind::Video).then_some(FrameRate::FPS_30),
        frame_count: None,
        variable_frame_rate: (kind == StreamKind::Video).then_some(false),
        video,
        audio,
    }
}
fn asset(id: u64, path: PathBuf, duration: i64) -> SourceAsset {
    SourceAsset::try_from(AssetData {
        id: AssetId(id),
        path,
        content_hash: ContentHash([0; 32]),
        probe: SourceProbe {
            format: "fixture".into(),
            duration: Some(seconds(duration)),
            streams: vec![stream(0, StreamKind::Video), stream(1, StreamKind::Audio)],
        },
    })
    .unwrap()
}
fn project(asset: SourceAsset) -> Project {
    let mut data = project::Project::empty(ProjectId(0), "source".into())
        .unwrap()
        .into_data();
    data.assets.push(asset);
    Project::new(data).unwrap()
}
fn text(value: &str) -> CueText {
    CueText {
        text: value.into(),
        edited: false,
        translation_ref: None,
    }
}
fn cue(id: &str, start: i64, end: i64, entries: &[(&str, &str)]) -> Cue {
    Cue {
        id: id.into(),
        range: range(start, end),
        source_lang: lang("vi"),
        texts: entries
            .iter()
            .map(|(language, value)| (lang(language), text(value)))
            .collect::<BTreeMap<_, _>>(),
    }
}
fn runner(concurrency: usize) -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [concurrency; 3],
            queue: 32,
            retained: 32,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(2 << 30).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}
fn terminal(state: Option<JobState>) -> bool {
    matches!(
        state,
        Some(JobState::Completed | JobState::Failed | JobState::Cancelled)
    )
}
fn wait(batch: &ClipBatch, runner: &mut JobRunner) -> Vec<JobSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        runner.tick().unwrap();
        let states: Vec<_> = (0..batch.len())
            .map(|index| batch.snapshot(runner, index).map(|snapshot| snapshot.state))
            .collect();
        if states.iter().all(|state| terminal(*state)) {
            return (0..batch.len())
                .filter_map(|index| batch.snapshot(runner, index))
                .collect();
        }
        assert!(Instant::now() < deadline, "clip batch timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn tool(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    command
}
fn oracles() -> bool {
    let available = ["ffmpeg", "ffprobe"].iter().all(|program| {
        tool(program)
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success())
    });
    if !available {
        assert!(
            std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
            "required ffmpeg/ffprobe missing"
        );
        eprintln!("SKIP clip export oracle: ffmpeg/ffprobe missing");
    }
    available
}
fn fixture(directory: &Path, seconds: u32) -> PathBuf {
    let path = directory.join("source.mp4");
    let duration = seconds.to_string();
    let output = tool("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args([
            "-i",
            &format!("testsrc2=size=320x180:rate=30:duration={duration}"),
        ])
        .args(["-f", "lavfi"])
        .args([
            "-i",
            &format!("sine=frequency=440:sample_rate=48000:duration={duration}"),
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-shortest"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}
fn output_directory(directory: &Path) -> PathBuf {
    let path = directory.join("out");
    std::fs::create_dir(&path).unwrap();
    path
}
fn temporary_files(directory: &Path) -> usize {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".vibecut-export-")
        })
        .count()
}
fn batch(asset: u64, shorts: Vec<ClipShort>, cues: CueTrack, directory: &Path) -> ClipBatchRequest {
    ClipBatchRequest {
        cues,
        ..ClipBatchRequest::new(AssetId(asset), shorts, directory)
    }
}

#[test]
fn joins_play_order_excludes_gaps_and_matches_one_concatenated_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 30));
    let cues = CueTrack {
        cues: vec![
            cue(
                "a",
                2,
                12,
                &[("vi", "Xin chào"), ("en", "Hello"), ("zh-Hant", "你好")],
            ),
            cue("b", 20, 26, &[("vi", "Kết thúc"), ("zh-Hant", "結束")]),
        ],
    };
    let request = ClipBatchRequest {
        languages: vec![lang("vi"), lang("zh-Hant")],
        subtitle_formats: vec![Format::Srt, Format::WebVtt],
        ..batch(
            1,
            vec![ClipShort::new(
                "Bài học",
                vec![segment(20, 25, 3), segment(2, 5, 1), segment(10, 12, 2)],
            )],
            cues.clone(),
            dir.path(),
        )
    };
    let batch = ClipBatch::prepare(&source, request).unwrap();
    assert_eq!(batch.duration(0), Some(seconds(10)));
    let prepared = batch.project(0).unwrap();
    let video = prepared
        .data()
        .timeline
        .tracks
        .iter()
        .find(|track| track.kind == project::TrackKind::Video)
        .unwrap();
    assert_eq!(
        video
            .items
            .iter()
            .map(|item| (item.source_range, item.start))
            .collect::<Vec<_>>(),
        vec![
            (range(20, 25), seconds(0)),
            (range(2, 5), seconds(5)),
            (range(10, 12), seconds(8)),
        ]
    );
    let captions = prepared
        .data()
        .timeline
        .tracks
        .iter()
        .find(|track| track.kind == project::TrackKind::Caption)
        .unwrap();
    assert_eq!(captions.items.len(), 3);
    assert_eq!(
        captions
            .items
            .iter()
            .map(|item| (item.source_range, item.start))
            .collect::<Vec<_>>(),
        vec![
            (range(20, 25), seconds(0)),
            (range(2, 5), seconds(5)),
            (range(10, 12), seconds(8)),
        ]
    );
    for item in &captions.items {
        let project::ItemParams::Caption(params) = &item.content else {
            panic!("not a caption item");
        };
        for cue in &params.cues.cues {
            assert!(cue.range.start() >= item.source_range.start());
            assert!(cue.range.end() <= item.source_range.end());
        }
    }
    let settings = batch.settings(0).unwrap();
    assert_eq!(
        (settings.render.width, settings.render.height),
        (1080, 1920)
    );
    assert_eq!(
        settings.render.caption_languages,
        vec![lang("vi"), lang("zh-Hant")]
    );
    assert_eq!(settings.fps, FrameRate::FPS_30);
    assert!(!settings.allow_degraded);
    assert_eq!(batch.paths(0).unwrap().subtitles.len(), 2);
    let exported = engine::subtitle_file(
        prepared,
        project::TrackId(2),
        Format::Srt,
        &[lang("vi"), lang("zh-Hant")],
    )
    .unwrap();
    let mapping = captions::EditMapping::new(vec![
        (range(20, 25), seconds(0)),
        (range(2, 5), seconds(5)),
        (range(10, 12), seconds(8)),
    ])
    .unwrap();
    let direct = captions::export(
        &mapping.map_cues(&cues).unwrap(),
        Format::Srt,
        &[lang("vi"), lang("zh-Hant")],
    )
    .unwrap();
    assert_eq!(exported.text, direct.text);
    assert!(exported.text.contains("00:00:00,000 --> 00:00:05,000"));
    assert!(exported.text.contains("00:00:05,000 --> 00:00:08,000"));
    assert!(exported.text.contains("00:00:08,000 --> 00:00:10,000"));
    assert!(exported.text.contains("Xin chào\n你好"));
}

#[test]
fn applies_one_normalized_crop_to_every_piece() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 30));
    let crop = render::Rect {
        x: 0.1,
        y: 0.0,
        width: 0.8,
        height: 1.0,
    };
    let short =
        ClipShort::new("Crop", vec![segment(0, 2, 0), segment(4, 6, 1)]).with_crop(Some(crop));
    let batch = ClipBatch::prepare(
        &source,
        batch(1, vec![short], CueTrack::default(), dir.path()),
    )
    .unwrap();
    let settings = batch.settings(0).unwrap();
    let prepared = batch.project(0).unwrap();
    let items: Vec<_> = prepared
        .data()
        .timeline
        .tracks
        .iter()
        .filter(|track| track.kind == project::TrackKind::Video)
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(items.len(), 2);
    for item in items {
        let placement = settings.render.placements.get(&item.id).unwrap();
        assert_eq!(placement.crop, crop);
        assert_eq!(placement.fit, render::Fit::Cover);
    }
}

#[test]
fn output_names_are_unicode_safe_bounded_and_unique() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 60));
    let long = "dài".repeat(200);
    let titles = [
        "Bài học: 100% \"hay\"/<tốt>",
        "CON",
        "COM1.txt",
        "Tên 🎬 clip",
        "Tên 🎬 clip",
        "   ...   ",
        "\u{202e}evil",
        long.as_str(),
    ];
    let shorts = titles
        .iter()
        .map(|title| ClipShort::new(*title, vec![segment(0, 1, 0)]))
        .collect();
    let batch =
        ClipBatch::prepare(&source, batch(1, shorts, CueTrack::default(), dir.path())).unwrap();
    let names: Vec<String> = (0..batch.len())
        .map(|index| {
            batch
                .paths(index)
                .unwrap()
                .video
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    for name in &names {
        assert!(name.ends_with(".mp4"));
        assert!(name.len() <= MAX_NAME_BYTES + 4);
        assert!(!name.chars().any(|character| {
            matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        }));
        let stem = name.trim_end_matches(".mp4");
        assert!(!stem.is_empty());
        assert!(!stem.ends_with('.'));
        assert!(!stem.ends_with(' '));
        assert!(!reserved(stem));
    }
    let unique: std::collections::BTreeSet<_> =
        names.iter().map(|name| name.to_lowercase()).collect();
    assert_eq!(unique.len(), names.len());
    assert!(names[0].contains("Bài học"));
    assert!(names[1].starts_with("clip-CON"));
    assert!(names[2].starts_with("clip-COM1"));
    assert!(names[3].starts_with("Tên 🎬 clip"));
    assert_ne!(names[3], names[4]);
    assert_eq!(names[5], "clip-06.mp4");
    assert!(names[6].contains("evil"));
    assert!(!names[6].contains('\u{202e}'));
}
const MAX_NAME_BYTES: usize = 200;
fn reserved(stem: &str) -> bool {
    let upper = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || (upper.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.as_bytes()[3].is_ascii_digit()
        && upper.as_bytes()[3] != b'0')
}

#[test]
fn refuses_hostile_batches_at_the_short_or_request_level() {
    use engine::EditError;
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 30));
    let one = |segments| ClipShort::new("Short", segments);
    let prepare =
        |short: ClipShort, languages: Vec<LanguageTag>, formats: Vec<Format>, max: u64| {
            let request = ClipBatchRequest {
                languages,
                subtitle_formats: formats,
                max_output_bytes: max,
                ..batch(1, vec![short], CueTrack::default(), dir.path())
            };
            ClipBatch::prepare(&source, request)
        };
    assert!(matches!(
        prepare(one(vec![]), vec![], vec![], 1 << 20),
        Err(EditError::ClipExport {
            short: 0,
            reason: "no segments"
        })
    ));
    let too_many: Vec<_> = (0..33)
        .map(|index| segment(index, index + 1, index as usize))
        .collect();
    assert!(matches!(
        prepare(one(too_many), vec![], vec![], 1 << 20),
        Err(EditError::ClipExport {
            reason: "too many segments",
            ..
        })
    ));
    assert!(matches!(
        prepare(
            one(vec![segment(0, 5, 0), segment(4, 6, 1)]),
            vec![],
            vec![],
            1 << 20
        ),
        Err(EditError::ClipExport {
            reason: "segments overlap",
            ..
        })
    ));
    assert!(matches!(
        prepare(one(vec![segment(29, 31, 0)]), vec![], vec![], 1 << 20),
        Err(EditError::ClipExport {
            reason: "segment outside source",
            ..
        })
    ));
    let negative = TimeRange::new(Tick(-1), seconds(1)).unwrap();
    assert!(matches!(
        prepare(
            one(vec![ClipSourceSegment {
                range: negative,
                role: ClipRole::Payoff,
                source_order: 0,
                repair: None,
            }]),
            vec![],
            vec![],
            1 << 20,
        ),
        Err(EditError::ClipExport {
            reason: "negative source range",
            ..
        })
    ));
    assert!(matches!(
        prepare(
            one(vec![segment(0, 1, 0)]),
            vec![lang("vi"), lang("en"), lang("zh")],
            vec![],
            1 << 20,
        ),
        Err(EditError::Invalid("clip caption languages"))
    ));
    assert!(matches!(
        prepare(
            one(vec![segment(0, 1, 0)]),
            vec![lang("vi"), lang("vi")],
            vec![],
            1 << 20,
        ),
        Err(EditError::Invalid("clip caption languages"))
    ));
    assert!(matches!(
        prepare(
            one(vec![segment(0, 1, 0)]),
            vec![],
            vec![Format::Srt],
            1 << 20
        ),
        Err(EditError::Invalid("clip subtitle languages"))
    ));
    assert!(matches!(
        prepare(
            one(vec![segment(0, 1, 0)]),
            vec![lang("vi")],
            vec![Format::Srt, Format::Srt],
            1 << 20,
        ),
        Err(EditError::Invalid("clip subtitle formats"))
    ));
    assert!(matches!(
        prepare(one(vec![segment(0, 1, 0)]), vec![], vec![], 0),
        Err(EditError::Invalid("clip output byte ceiling"))
    ));
    let bad_crop = ClipShort::new("Crop", vec![segment(0, 1, 0)]).with_crop(Some(render::Rect {
        x: -0.1,
        ..Default::default()
    }));
    assert!(matches!(
        prepare(bad_crop, vec![], vec![], 1 << 20),
        Err(EditError::Render(render::Error::Invalid(
            "normalized rectangle"
        )))
    ));
    let missing_asset = ClipBatchRequest {
        asset: AssetId(9),
        ..batch(
            9,
            vec![one(vec![segment(0, 1, 0)])],
            CueTrack::default(),
            dir.path(),
        )
    };
    assert!(matches!(
        ClipBatch::prepare(&source, missing_asset),
        Err(EditError::Invalid("clip source asset"))
    ));
    let mut missing_directory = batch(
        1,
        vec![one(vec![segment(0, 1, 0)])],
        CueTrack::default(),
        dir.path(),
    );
    missing_directory.directory = dir.path().join("absent");
    assert!(matches!(
        ClipBatch::prepare(&source, missing_directory),
        Err(EditError::Invalid("clip output directory"))
    ));
    let empty = ClipBatchRequest {
        shorts: vec![],
        ..batch(1, vec![], CueTrack::default(), dir.path())
    };
    assert!(matches!(
        ClipBatch::prepare(&source, empty),
        Err(EditError::Invalid("clip batch size"))
    ));
    let mut audio_only = AssetData::from(asset(2, dir.path().join("missing-source.mp4"), 10));
    audio_only
        .probe
        .streams
        .retain(|stream| stream.kind != StreamKind::Video);
    let audio_source = project(SourceAsset::try_from(audio_only).unwrap());
    assert!(matches!(
        ClipBatch::prepare(
            &audio_source,
            batch(
                2,
                vec![one(vec![segment(0, 1, 0)])],
                CueTrack::default(),
                dir.path()
            ),
        ),
        Err(EditError::Invalid("clip source media"))
    ));
}

#[test]
fn missing_source_fails_each_short_with_typed_diagnostics_and_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 10));
    let shorts = vec![
        ClipShort::new("One", vec![segment(0, 1, 0), segment(3, 4, 1)]),
        ClipShort::new("Two", vec![segment(1, 2, 0), segment(4, 5, 1)]),
    ];
    let mut batch =
        ClipBatch::prepare(&source, batch(1, shorts, CueTrack::default(), dir.path())).unwrap();
    let mut runner = runner(2);
    assert_eq!(batch.submit(&mut runner).unwrap().len(), 2);
    let snapshots = wait(&batch, &mut runner);
    for (index, snapshot) in snapshots.iter().enumerate() {
        assert_eq!(snapshot.state, JobState::Failed);
        match snapshot.error.as_deref() {
            Some(EditError::Render(render::Error::Unavailable(details))) => {
                assert_eq!(details.len(), 2);
                for detail in details {
                    assert_eq!(detail.asset, AssetId(1));
                    assert_eq!(detail.reason, render::UnavailableReason::Missing);
                }
                let ranges: Vec<_> = details.iter().map(|detail| detail.range).collect();
                let expected = if index == 0 {
                    vec![range(0, 1), range(3, 4)]
                } else {
                    vec![range(1, 2), range(4, 5)]
                };
                assert_eq!(ranges, expected);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!batch.paths(index).unwrap().video.exists());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let previous = batch.job_id(0);
    let retried = batch.retry(&mut runner, 0).unwrap();
    assert_ne!(Some(retried), previous);
    let snapshot = wait(&batch, &mut runner);
    assert_eq!(snapshot[0].state, JobState::Failed);
}

#[test]
fn cancelling_one_short_mid_batch_leaves_neighbours_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 10));
    let shorts: Vec<_> = (0..3)
        .map(|index| ClipShort::new(format!("Clip {index}"), vec![segment(index, index + 1, 0)]))
        .collect();
    let mut batch =
        ClipBatch::prepare(&source, batch(1, shorts, CueTrack::default(), dir.path())).unwrap();
    let mut runner = runner(1);
    assert_eq!(batch.submit(&mut runner).unwrap().len(), 3);
    assert!(batch.submit(&mut runner).unwrap().is_empty());
    batch.cancel(1).unwrap();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(snapshots[0].state, JobState::Failed);
    assert_eq!(snapshots[1].state, JobState::Cancelled);
    assert_eq!(snapshots[2].state, JobState::Failed);
    assert!(!batch.paths(1).unwrap().video.exists());
    assert!(batch.retry(&mut runner, 1).is_ok());
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(snapshots[1].state, JobState::Failed);
    assert!(matches!(batch.cancel(9), Err(EditError::Invalid(_))));
    batch.cancel_all();
}

#[test]
fn storage_ceiling_refuses_before_publishing_and_preserves_previous_files() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 10));
    let cue = cue("long", 0, 1, &[("vi", &"x".repeat(4096))]);
    let request = ClipBatchRequest {
        languages: vec![lang("vi")],
        subtitle_formats: vec![Format::Srt],
        overwrite: true,
        max_output_bytes: 64,
        ..batch(
            1,
            vec![ClipShort::new("Cap", vec![segment(0, 1, 0)])],
            CueTrack { cues: vec![cue] },
            dir.path(),
        )
    };
    let mut batch = ClipBatch::prepare(&source, request).unwrap();
    let video = batch.paths(0).unwrap().video.clone();
    let subtitle = batch.paths(0).unwrap().subtitles[0].path.clone();
    std::fs::write(&video, b"previous video").unwrap();
    std::fs::write(&subtitle, b"previous subtitle").unwrap();
    let mut runner = runner(1);
    batch.submit(&mut runner).unwrap();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(snapshots[0].state, JobState::Failed);
    assert!(matches!(
        snapshots[0].error.as_deref(),
        Some(EditError::Io(_) | EditError::Project(_))
    ));
    assert_eq!(std::fs::read(&video).unwrap(), b"previous video");
    assert_eq!(std::fs::read(&subtitle).unwrap(), b"previous subtitle");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    assert_eq!(temporary_files(dir.path()), 0);
}

#[test]
fn existing_destination_is_preserved_and_sidecars_are_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let source = project(asset(1, dir.path().join("missing-source.mp4"), 10));
    let request = ClipBatchRequest {
        languages: vec![lang("vi")],
        subtitle_formats: vec![Format::Srt],
        ..batch(
            1,
            vec![ClipShort::new("Existing", vec![segment(0, 1, 0)])],
            CueTrack {
                cues: vec![cue("a", 0, 1, &[("vi", "Xin chào")])],
            },
            dir.path(),
        )
    };
    let mut batch = ClipBatch::prepare(&source, request).unwrap();
    let video = batch.paths(0).unwrap().video.clone();
    let subtitle = batch.paths(0).unwrap().subtitles[0].path.clone();
    std::fs::write(&video, b"previous").unwrap();
    let mut runner = runner(1);
    batch.submit(&mut runner).unwrap();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(snapshots[0].state, JobState::Failed);
    assert_eq!(std::fs::read(&video).unwrap(), b"previous");
    assert!(!subtitle.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(temporary_files(dir.path()), 0);
}

#[test]
fn bilingual_ordered_segments_export_mp4_and_subtitle_sidecars() {
    if !oracles() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = output_directory(dir.path());
    let path = fixture(dir.path(), 1);
    let source = project(SourceAsset::import(AssetId(1), path).unwrap());
    let cues = CueTrack {
        cues: vec![Cue {
            id: "speech".into(),
            range: short_range(1, 8),
            source_lang: lang("vi"),
            texts: [
                ("vi", "Xin chào các bạn"),
                ("en", "Hello everyone"),
                ("zh-Hant", "大家好"),
            ]
            .into_iter()
            .map(|(language, value)| (lang(language), text(value)))
            .collect(),
        }],
    };
    let request = ClipBatchRequest {
        languages: vec![lang("vi"), lang("zh-Hant")],
        subtitle_formats: vec![Format::Srt, Format::WebVtt],
        ..batch(
            1,
            vec![ClipShort::new(
                "Phỏng vấn",
                vec![
                    ClipSourceSegment {
                        range: short_range(1, 3),
                        role: ClipRole::Hook,
                        source_order: 0,
                        repair: None,
                    },
                    ClipSourceSegment {
                        range: short_range(6, 8),
                        role: ClipRole::Payoff,
                        source_order: 1,
                        repair: None,
                    },
                ],
            )],
            cues,
            &out,
        )
    };
    let mut batch = ClipBatch::prepare(&source, request).unwrap();
    let mut runner = runner(1);
    batch.submit(&mut runner).unwrap();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(
        snapshots[0].state,
        JobState::Completed,
        "{:?}",
        snapshots[0].error
    );
    let paths = batch.paths(0).unwrap();
    assert!(paths.video.exists());
    assert_eq!(paths.subtitles.len(), 2);
    assert!(
        paths
            .subtitles
            .iter()
            .all(|subtitle| subtitle.path.exists())
    );
    let probe = media::probe(&paths.video).unwrap();
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Video)
        .unwrap();
    let shape = video.video.as_ref().unwrap();
    assert_eq!((shape.width, shape.height), (1080, 1920));
    assert!(
        probe
            .streams
            .iter()
            .any(|stream| stream.kind == StreamKind::Audio)
    );
    let duration = probe.duration.unwrap();
    let frame = FrameRate::FPS_30.frame_duration().unwrap();
    assert!((duration - tenths(4)).0.abs() <= frame.0, "{duration:?}");
    let srt = std::fs::read_to_string(&paths.subtitles[0].path).unwrap();
    assert!(srt.contains("00:00:00,000 --> 00:00:00,200"));
    assert!(srt.contains("00:00:00,200 --> 00:00:00,400"));
    assert!(srt.contains("Xin chào các bạn\n大家好"));
    let parsed = captions::parse(srt.as_bytes(), Format::Srt, lang("vi")).unwrap();
    assert_eq!(parsed.track.cues.len(), 2);
    assert_eq!(
        parsed.track.cues[0].range,
        TimeRange::new(tenths(0), tenths(2)).unwrap()
    );
    assert_eq!(
        parsed.track.cues[1].range,
        TimeRange::new(tenths(2), tenths(4)).unwrap()
    );
    let vtt = std::fs::read_to_string(&paths.subtitles[1].path).unwrap();
    assert!(vtt.starts_with("WEBVTT\n\n"));
    assert_eq!(temporary_files(&out), 0);
}

#[test]
fn cancelling_mid_batch_completes_neighbours_and_retry_finishes() {
    if !oracles() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = output_directory(dir.path());
    let path = fixture(dir.path(), 1);
    let source = project(SourceAsset::import(AssetId(1), path).unwrap());
    let shorts: Vec<_> = (0..3)
        .map(|index| {
            ClipShort::new(
                format!("Short {index}"),
                vec![ClipSourceSegment {
                    range: short_range(1 + index * 2, 3 + index * 2),
                    role: ClipRole::Payoff,
                    source_order: 0,
                    repair: None,
                }],
            )
        })
        .collect();
    let mut batch =
        ClipBatch::prepare(&source, batch(1, shorts, CueTrack::default(), &out)).unwrap();
    let mut runner = runner(1);
    batch.submit(&mut runner).unwrap();
    batch.cancel(1).unwrap();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(
        snapshots[0].state,
        JobState::Completed,
        "{:?}",
        snapshots[0].error
    );
    assert_eq!(snapshots[1].state, JobState::Cancelled);
    assert_eq!(
        snapshots[2].state,
        JobState::Completed,
        "{:?}",
        snapshots[2].error
    );
    assert!(batch.paths(0).unwrap().video.exists());
    assert!(!batch.paths(1).unwrap().video.exists());
    assert!(batch.paths(2).unwrap().video.exists());
    let previous = batch.job_id(1);
    let retried = batch.retry(&mut runner, 1).unwrap();
    assert_ne!(Some(retried), previous);
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(
        snapshots[1].state,
        JobState::Completed,
        "{:?}",
        snapshots[1].error
    );
    assert!(batch.paths(1).unwrap().video.exists());
    assert!(matches!(
        batch.retry(&mut runner, 0),
        Err(EditError::Invalid("clip is not retryable"))
    ));
    assert_eq!(temporary_files(&out), 0);
}

#[test]
fn cancel_all_mid_batch_keeps_completed_shorts_and_stops_the_rest() {
    if !oracles() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = output_directory(dir.path());
    let path = fixture(dir.path(), 1);
    let source = project(SourceAsset::import(AssetId(1), path).unwrap());
    let shorts: Vec<_> = (0..3)
        .map(|index| {
            ClipShort::new(
                format!("Stop {index}"),
                vec![ClipSourceSegment {
                    range: short_range(1 + index * 2, 3 + index * 2),
                    role: ClipRole::Payoff,
                    source_order: 0,
                    repair: None,
                }],
            )
        })
        .collect();
    let mut batch =
        ClipBatch::prepare(&source, batch(1, shorts, CueTrack::default(), &out)).unwrap();
    let mut runner = runner(1);
    batch.submit(&mut runner).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        runner.tick().unwrap();
        if batch.snapshot(&runner, 0).map(|snapshot| snapshot.state) == Some(JobState::Completed) {
            break;
        }
        assert!(Instant::now() < deadline, "first short did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
    batch.cancel_all();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(snapshots[0].state, JobState::Completed);
    assert_eq!(snapshots[1].state, JobState::Cancelled);
    assert_eq!(snapshots[2].state, JobState::Cancelled);
    assert!(batch.paths(0).unwrap().video.exists());
    assert!(!batch.paths(1).unwrap().video.exists());
    assert!(!batch.paths(2).unwrap().video.exists());
    assert_eq!(temporary_files(&out), 0);
}

#[test]
fn video_storage_ceiling_preserves_previous_output() {
    if !oracles() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = output_directory(dir.path());
    let path = fixture(dir.path(), 1);
    let source = project(SourceAsset::import(AssetId(1), path).unwrap());
    let request = ClipBatchRequest {
        overwrite: true,
        max_output_bytes: 512,
        ..batch(
            1,
            vec![ClipShort::new(
                "Cap",
                vec![ClipSourceSegment {
                    range: short_range(1, 2),
                    role: ClipRole::Payoff,
                    source_order: 0,
                    repair: None,
                }],
            )],
            CueTrack::default(),
            &out,
        )
    };
    let mut batch = ClipBatch::prepare(&source, request).unwrap();
    let video = batch.paths(0).unwrap().video.clone();
    std::fs::write(&video, b"previous").unwrap();
    let mut runner = runner(1);
    batch.submit(&mut runner).unwrap();
    let snapshots = wait(&batch, &mut runner);
    assert_eq!(snapshots[0].state, JobState::Failed);
    assert!(
        matches!(
            snapshots[0].error.as_deref(),
            Some(
                EditError::Io(_)
                    | EditError::Project(_)
                    | EditError::Render(render::Error::Io(_) | render::Error::Media(_))
            )
        ),
        "{:?}",
        snapshots[0].error
    );
    assert_eq!(std::fs::read(&video).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
    assert_eq!(temporary_files(&out), 0);
}
