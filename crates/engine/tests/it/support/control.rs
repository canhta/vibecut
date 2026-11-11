//! Control fixtures shared by the command-family tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use engine::control::{
    self, Caller, ControlResult, ErrorCode, MediaAccess, Policy, Request, Response, ResponseValue,
};
use engine::jobs::{
    JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig, SystemClock,
};
use engine::{ByteBudget, Command, EditError, Session};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use time::{FrameRate, SourceRange, Tick};

pub(crate) struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &Project) -> engine::Result<()> {
        Ok(())
    }
}
pub(crate) struct Deny;
impl Policy for Deny {
    fn authorize(&self, _: &Command, _: &Project) -> engine::Result<()> {
        Err(EditError::Permission)
    }
}
pub(crate) fn caller(policy: &dyn Policy) -> Caller<'_> {
    Caller {
        principal: "test",
        external: true,
        policy,
        fonts: None,
        sounds: None,
        media: None,
        chapters: None,
        captions: None,
    }
}
pub(crate) fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 16,
            retained: 16,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}
pub(crate) fn session() -> Session {
    Session::new(
        Project::empty(ProjectId(u64::MAX), "Test".into()).unwrap(),
        10,
    )
    .unwrap()
}
pub(crate) fn request(s: &Session, id: &str, params: Value, key: &str) -> Request {
    Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: s.session_id().into(),
        request_id: "request".into(),
        idempotency_key: Some(key.into()),
        base_rev: Some(s.revision()),
        command: id.into(),
        params,
    }
}
pub(crate) fn run(
    s: &mut Session,
    r: &mut JobRunner,
    id: &str,
    params: Value,
    key: &str,
) -> Response {
    let req = request(s, id, params, key);
    s.control_request(req, caller(&Allow), r).unwrap()
}
pub(crate) fn ok(response: Response) -> ControlResult {
    match response.value {
        ResponseValue::Result(v) => v,
        ResponseValue::Error(e) => panic!("unexpected error {e:?}"),
    }
}
pub(crate) fn error(response: Response, code: ErrorCode) {
    assert!(
        matches!(response.value,ResponseValue::Error(c) if c==code),
        "{response:?}"
    );
}
pub(crate) fn unchanged(
    s: &Session,
    bytes: &[u8],
    revision: &control::Revision,
    history: (usize, usize),
) {
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(&s.revision(), revision);
    assert_eq!(s.history_len(), history);
}
pub(crate) fn add(s: &mut Session, r: &mut JobRunner, key: &str) {
    ok(run(
        s,
        r,
        "timeline.add_track",
        json!({"name":"Added","kind":"video"}),
        key,
    ));
}
pub(crate) fn seed() -> Session {
    let mut data = session().project().data().clone();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.fps = FrameRate::FPS_30;
    let duration = Tick(time::TICKS_PER_SECOND);
    let mut animations = project::Animations::default();
    let mut param = project::Param::new(project::ParamValue::Scalar(1.0)).unwrap();
    param
        .channel_mut(None)
        .unwrap()
        .put(project::Keyframe::new(
            Tick::ZERO,
            project::ParamValue::Scalar(1.0),
        ))
        .unwrap();
    animations
        .set(project::AnimationPath::new("opacity").unwrap(), param)
        .unwrap();
    for (id, path) in [
        (0, "vibecut-control-caption-source.mp4"),
        (98, "vibecut-control-unused-source.mp4"),
    ] {
        data.assets.push(
            testkit::source::asset(id, std::env::temp_dir().join(path))
                .format("mp4")
                .duration(duration)
                .content_hash(media::ContentHash::from_bytes(b"fixture"))
                .stream(
                    testkit::source::video()
                        .time_base(media::TimeBase::new(1, 30000).unwrap())
                        .duration(duration)
                        .start(Tick::ZERO)
                        .size(64, 64)
                        .build(),
                )
                .build(),
        );
    }
    data.scene_mut().timeline.overlays = vec![
        Track {
            id: TrackId(1),
            name: "Overlay".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![
                Item {
                    hidden: false,
                    blend_mode: Default::default(),
                    retime: Default::default(),
                    masks: vec![project::Mask::new("mask1", project::MaskShape::Rectangle)],
                    effects: vec![
                        project::effect_definition("blur")
                            .unwrap()
                            .instance("blur1"),
                    ],
                    id: ItemId(1),
                    animations,
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                    start: Tick::ZERO,
                    content: ItemParams::Overlay(OverlayParams {
                        color: Color([0, 64, 128, 255]),
                    }),
                },
                Item {
                    hidden: false,
                    blend_mode: Default::default(),
                    retime: Default::default(),
                    masks: Vec::new(),
                    effects: Vec::new(),
                    id: ItemId(5),
                    animations: Default::default(),
                    source_asset: Some(media::AssetId(0)),
                    source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                    start: duration,
                    content: ItemParams::Video(project::VideoParams {
                        fades: Default::default(),
                        source_audio_enabled: false,
                        volume: 0.0,
                        muted: false,
                        crop: None,
                    }),
                },
            ],
        },
        Track {
            id: TrackId(3),
            name: "Empty".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![],
        },
        Track {
            id: TrackId(4),
            name: "Captions".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                id: ItemId(4),
                animations: Default::default(),
                source_asset: Some(media::AssetId(0)),
                source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Caption(project::CaptionParams {
                    cues: captions::CueTrack {
                        styles: Vec::new(),
                        cues: vec![captions::Cue {
                            id: "cue-1".into(),
                            range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                            source_lang: captions::LanguageTag::parse("en").unwrap(),
                            texts: [(
                                captions::LanguageTag::parse("en").unwrap(),
                                captions::CueText {
                                    text: "Hello".into(),
                                    edited: false,
                                    translation_ref: None,
                                },
                            )]
                            .into_iter()
                            .collect(),
                            word_ranges: Vec::new(),
                            style: None,
                        }],
                    },
                }),
            }],
        },
    ];
    data.scene_mut().timeline.overlays.reverse();
    data.scene_mut().bookmarks.push(project::Bookmark {
        id: project::BookmarkId(1),
        start: Tick::ZERO,
        duration: Tick::ZERO,
        note: "Mark".into(),
        color: Color([255, 255, 0, 255]),
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// Only this copied seed gets Text item 3 on the previously empty track 3.
pub(crate) fn with_caption_presets(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    let style = captions::CueStyle {
        font_family: "Inter".into(),
        font_size_ratio: 48.0 / 1080.0,
        primary_color: [255; 4],
        outline_color: [0, 0, 0, 255],
        background_color: [0; 4],
        border_style: 1,
        alignment: captions::CueAlignment::from_numpad(2).unwrap(),
        margin_left_ratio: 0.05,
        margin_right_ratio: 0.05,
        margin_vertical_ratio: 0.05,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    };
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if item.id == ItemId(4) {
            let ItemParams::Caption(params) = &mut item.content else {
                panic!("caption seed")
            };
            params.cues.styles = vec![style.clone()];
            for cue in &mut params.cues.cues {
                cue.style = Some(captions::CueStyleId(0));
            }
        }
    }
    data.caption_presets = vec![project::SavedCaptionPreset {
        id: project::CaptionPresetId(100),
        name: "Project look".into(),
        style,
    }];
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

/// Only this copied seed gets Text item 3 on the previously empty track 3.
pub(crate) fn with_text(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    let track = data
        .scene_mut()
        .timeline
        .iter_mut()
        .find(|t| t.id == TrackId(3))
        .unwrap();
    assert!(track.items.is_empty());
    track.kind = TrackKind::Text;
    track.items.push(Item {
        id: ItemId(3),
        start: Tick::ZERO,
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
        content: ItemParams::Text(project::TextParams::new("Tiếng Việt\nEnglish")),
        animations: Default::default(),
        effects: Vec::new(),
        masks: Vec::new(),
        blend_mode: Default::default(),
        hidden: false,
        retime: Default::default(),
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The seed plus one real mono audio asset (id 97) for waveform command examples.
pub(crate) fn with_waveform(seed: &Session, path: &Path) -> Session {
    let mut data = seed.project().data().clone();
    data.assets.push(
        media::SourceAsset::import(media::AssetId(97), path.to_owned())
            .expect("fixture wav imports"),
    );
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The seed plus one real 2 s video asset (id 97): ten black frames, then ten white.
pub(crate) fn with_video(seed: &Session, path: &Path) -> Session {
    let mut data = seed.project().data().clone();
    data.assets.push(
        media::SourceAsset::import(media::AssetId(97), path.to_owned())
            .expect("fixture video imports"),
    );
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The audible-clip seed with a real local wav behind asset 97 and a master target.
pub(crate) fn with_loudness(seed: &Session, wav: &Path) -> Session {
    let mut data = with_audio(seed).project().data().clone();
    let slot = data
        .assets
        .iter()
        .position(|asset| asset.id() == media::AssetId(97))
        .unwrap();
    data.assets[slot] = media::SourceAsset::import(media::AssetId(97), wav.to_owned())
        .expect("fixture wav imports");
    let mut session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    session
        .apply(Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -14.0,
            true_peak_db: -1.0,
        })
        .unwrap();
    session
}
/// Twenty 64x64 frames at 10 fps through the oracle FFmpeg: black, then white from 1 s.
pub(crate) fn write_video(path: &Path) {
    use std::io::Write;
    let mut child = testkit::oracle::command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgb24",
            "-video_size",
            "64x64",
            "-framerate",
            "10",
            "-i",
            "pipe:0",
            "-c:v",
            "ffv1",
        ])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for frame in 0..20 {
        input
            .write_all(&vec![if frame < 10 { 0 } else { 255 }; 64 * 64 * 3])
            .unwrap();
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}
/// The seed plus one real still-image clip (item 6) for thumbnail command examples.
pub(crate) fn with_thumbnails(seed: &Session, path: &Path) -> Session {
    let mut data = seed.project().data().clone();
    data.assets.push(
        media::SourceAsset::import(media::AssetId(97), path.to_owned())
            .expect("fixture png imports"),
    );
    let duration = Tick(time::TICKS_PER_SECOND);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(6),
        name: "Thumbnails".into(),
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
            id: ItemId(6),
            source_asset: Some(media::AssetId(97)),
            source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.0,
                muted: false,
                crop: None,
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// One second of 8 kHz mono PCM, written without an external tool.
pub(crate) fn write_wav(path: &Path, sample_rate: u32) {
    let samples = sample_rate;
    let bytes = samples
        .checked_mul(2)
        .and_then(|data| data.checked_add(36))
        .unwrap();
    let mut wav = Vec::with_capacity(bytes as usize + 8);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&bytes.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&sample_rate.saturating_mul(2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    for index in 0..samples {
        let value = (i16::try_from(index % 50).unwrap_or(0) - 25).saturating_mul(200);
        wav.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}
pub(crate) fn write_png(path: &Path) {
    image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 40, 60, 255]))
        .save(path)
        .unwrap();
}
pub(crate) fn media_access(root: &Path) -> MediaAccess {
    let budget = ByteBudget::new(256 << 20).unwrap();
    let (artifacts, _) =
        engine::cache::ArtifactCache::open(&root.join("media-waveform"), budget.clone(), 16)
            .unwrap();
    let thumbnails =
        engine::thumbnails::ThumbnailCache::open(&root.join("media-thumbnails"), budget, 16)
            .unwrap();
    MediaAccess::new(artifacts, thumbnails)
}
/// The seed with two adjacent measured-word cues for transcript edit examples.
pub(crate) fn with_words(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            let language = captions::LanguageTag::parse("en").unwrap();
            let cue = params.cues.cues.first_mut().unwrap();
            let end = cue.range.end();
            cue.range = SourceRange::new(Tick::ZERO, Tick(500)).unwrap();
            cue.texts.values_mut().next().unwrap().text = "Um brave".into();
            cue.word_ranges = (0..2)
                .map(|index| SourceRange::new(Tick(index * 100), Tick(index * 100 + 100)).unwrap())
                .collect();
            params.cues.cues.push(captions::Cue {
                id: "cue-2".into(),
                range: SourceRange::new(Tick(500), end).unwrap(),
                source_lang: language.clone(),
                texts: [(
                    language,
                    captions::CueText {
                        text: "like".into(),
                        edited: false,
                        translation_ref: None,
                    },
                )]
                .into_iter()
                .collect(),
                word_ranges: vec![SourceRange::new(Tick(500), Tick(600)).unwrap()],
                style: None,
            });
        }
    }
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The seed plus one audible clip (track 5, item 5) for audio registry examples.
pub(crate) fn with_audio(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        track.items.retain(|item| item.id != ItemId(5));
    }
    let duration = Tick(time::TICKS_PER_SECOND);
    data.assets.push(
        testkit::source::asset(
            97,
            std::env::temp_dir().join("vibecut-control-audio-source.wav"),
        )
        .format("wav")
        .duration(duration)
        .content_hash(media::ContentHash::from_bytes(b"audio fixture"))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(duration)
                .build(),
        )
        .build(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(5),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            retime: Default::default(),
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(5),
            animations: Default::default(),
            source_asset: Some(media::AssetId(97)),
            source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The seed whose item 5 is a video with an audible source asset.
pub(crate) fn with_source_audio(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    let duration = Tick(time::TICKS_PER_SECOND);
    let position = data
        .assets
        .iter()
        .position(|asset| asset.id() == media::AssetId(0))
        .unwrap();
    data.assets[position] = testkit::source::asset(
        0,
        std::env::temp_dir().join("vibecut-control-caption-source.mp4"),
    )
    .format("mp4")
    .duration(duration)
    .content_hash(media::ContentHash::from_bytes(b"fixture"))
    .stream(
        testkit::source::video()
            .time_base(media::TimeBase::new(1, 30000).unwrap())
            .duration(duration)
            .start(Tick::ZERO)
            .size(64, 64)
            .build(),
    )
    .stream(
        testkit::source::audio()
            .index(1)
            .codec("aac")
            .time_base(media::TimeBase::new(1, 48000).unwrap())
            .duration(duration)
            .start(Tick::ZERO)
            .build(),
    )
    .build();
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if item.id == ItemId(5)
                && let ItemParams::Video(params) = &mut item.content
            {
                params.source_audio_enabled = true;
            }
        }
    }
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
pub(crate) fn key() -> engine::cache::CacheKey {
    engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::Preview,
        source: [0; 32],
        provider: "fixture",
        model: "test",
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap()
}
pub(crate) fn submit(r: &mut JobRunner) -> engine::jobs::JobId {
    r.submit::<engine::EditError>(
        JobSpec {
            kind: JobKind::Import,
            class: ResourceClass::Cpu,
            input: key(),
            bytes: JobBytes::default(),
        },
        |_| Ok(vec![]),
    )
    .unwrap()
    .0
}
pub(crate) fn fake_fonts_list(_: Option<&str>) -> engine::Result<Vec<engine::control::FontEntry>> {
    Ok(vec![engine::control::FontEntry {
        family: "Inter".into(),
        origin: engine::control::FontOrigin::Bundled,
        styles: vec!["Regular".into()],
        state: engine::control::FontState::Available,
        license: Some("OFL-1.1".into()),
        source: None,
        download_bytes: None,
    }])
}
pub(crate) fn fake_fonts_load(family: &str) -> engine::Result<engine::control::FontLoad> {
    Ok(engine::control::FontLoad {
        family: family.into(),
        state: engine::control::FontLoadState::Loaded,
        origin: Some(engine::control::FontOrigin::Bundled),
        effective_family: family.into(),
        styles: vec!["Regular".into()],
    })
}
pub(crate) fn fake_fonts_download(
    runner: &mut JobRunner,
    _: &str,
    accepted: bool,
) -> engine::Result<engine::jobs::JobId> {
    if !accepted {
        return Err(engine::providers::Error::ConsentRequired.into());
    }
    Ok(submit(runner))
}
/// Synthetic capability for registry dispatch; real-store coverage lives in `libraries`.
pub(crate) fn font_access() -> engine::control::FontAccess<'static> {
    engine::control::FontAccess::new(
        &fake_fonts_list,
        &fake_fonts_load,
        &fake_fonts_download,
        &fake_fonts_import,
    )
}
fn fake_fonts_import(
    runner: &mut JobRunner,
    _: &std::path::Path,
    accepted: bool,
) -> engine::Result<engine::jobs::JobId> {
    if !accepted {
        return Err(engine::EditError::Permission);
    }
    Ok(submit(runner))
}
pub(crate) fn fake_sound_search(
    query: &engine::control::SoundQuery,
) -> engine::Result<engine::control::SoundPage> {
    Ok(engine::control::SoundPage {
        source: engine::control::SoundSource::Local,
        count: 0,
        page: query.page,
        page_size: query.page_size,
        has_next: false,
        commercial_only: query.commercial_only,
        results: Vec::new(),
    })
}
pub(crate) fn fake_sound_save(
    sound: &engine::control::SoundEntry,
) -> engine::Result<engine::control::SavedSoundEntry> {
    Ok(engine::control::SavedSoundEntry {
        id: sound.id,
        saved_at_unix_ms: 0,
        attribution: sound.attribution.clone(),
        licence: sound.licence.clone(),
    })
}
pub(crate) fn fake_sound_unsave(_: u64) -> engine::Result<bool> {
    Ok(true)
}
/// Synthetic capability for registry dispatch; real-library coverage lives in `libraries`.
pub(crate) fn sound_access() -> engine::control::SoundAccess<'static> {
    engine::control::SoundAccess::new(&fake_sound_search, &fake_sound_save, &fake_sound_unsave)
}
pub(crate) fn finish(r: &mut JobRunner, id: engine::jobs::JobId) {
    for _ in 0..10000 {
        r.tick().unwrap();
        if r.snapshot(id).is_some_and(|s| {
            matches!(
                s.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        }) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("job deadline");
}
/// Offline chapter adapter used by the registry and control conformance tests.
pub(crate) struct FixtureChapters {
    config: ai::ProviderConfig,
}
impl ai::chapters::ChapterGenerator for FixtureChapters {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::chapters::MODE.into(),
            operation: ai::Operation::GenerateChapters,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::chapters::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "chapters-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        input: &ai::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::chapters::ChapterProposal, ai::Error> {
        let first = input.segments.first().ok_or(ai::Error::Schema)?;
        let mut chapters = vec![ai::chapters::ChapterProposalEntry {
            start_segment_id: first.id.clone(),
            title: "Opening".into(),
        }];
        if input.segments.len() > 1
            && let Some(last) = input.segments.last()
        {
            chapters.push(ai::chapters::ChapterProposalEntry {
                start_segment_id: last.id.clone(),
                title: "Closing".into(),
            });
        }
        Ok(ai::chapters::ChapterProposal {
            title: "Chapters".into(),
            description: "Generated chapters.".into(),
            chapters,
        })
    }
}
/// Offline filler adapter used by the registry and control conformance tests.
pub(crate) struct FixtureFillers {
    config: ai::ProviderConfig,
}
impl ai::fillers::FillerClassifier for FixtureFillers {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::fillers::MODE.into(),
            operation: ai::Operation::ClassifyFillers,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::fillers::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "fillers-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        input: &ai::fillers::FillerInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::fillers::FillerProposal, ai::Error> {
        let fillers = input
            .segments
            .iter()
            .filter_map(|segment| segment.words.last())
            .map(|word| ai::fillers::FillerFinding {
                id: word.id.clone(),
                kind: captions::FillerKind::FilledPause,
            })
            .collect();
        Ok(ai::fillers::FillerProposal { fillers })
    }
}
pub(crate) fn fixture_chapters() -> engine::providers::TextAccess {
    let chapter =
        ai::chapters::OpenAiChapterGenerator::provider_config(ai::chapters::OPENAI_MODEL).unwrap();
    let filler =
        ai::fillers::OpenAiFillerClassifier::provider_config(ai::fillers::OPENAI_MODEL).unwrap();
    engine::providers::TextAccess::new(
        Arc::new(FixtureChapters {
            config: chapter.clone(),
        }),
        Arc::new(FixtureFillers {
            config: filler.clone(),
        }),
        Box::new(engine::providers::StoredConsent::grant([
            (chapter, ai::Money::new("USD", 0).unwrap()),
            (filler, ai::Money::new("USD", 0).unwrap()),
        ])),
    )
}
pub(crate) fn fillers(
    s: &mut Session,
    r: &mut JobRunner,
    language: &str,
    markers: bool,
    key: &str,
) -> Vec<engine::control::FillerCandidate> {
    let result = ok(run(
        s,
        r,
        "timeline.filler_candidates",
        json!({"language": language, "include_markers": markers}),
        key,
    ));
    let ControlResult::Fillers { candidates } = result else {
        panic!("expected filler candidates: {result:?}");
    };
    candidates
}
