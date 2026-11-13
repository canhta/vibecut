#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and explicit assertions"
)]
use engine::{
    AnalysisError, ByteBudget, Session,
    jobs::{JobRunner, JobState, RunnerConfig, SystemClock},
    pause_candidates::{PauseCandidates, PauseCandidatesRequest, PauseSettings},
};
use media::{AssetId, SourceAsset};
use project::{Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

fn ms(value: i64) -> Tick {
    Tick(value * (TICKS_PER_SECOND / 1000))
}
fn t(seconds: i64) -> Tick {
    Tick(seconds * TICKS_PER_SECOND)
}
fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(t(start), t(end)).unwrap()
}

/// 12 s of speech-like tone over a continuous 60 Hz room tone at 0.01 amplitude.
/// The gated speech leaves three pauses and one continuous final segment.
fn audio_fixture() -> Option<SourceAsset> {
    if !testkit::oracle::require("pause candidates") {
        return None;
    }
    let dir = testkit::fixture::dir("pause-candidates");
    let expression = "(0.25*sin(2*PI*180*t)+0.15*sin(2*PI*320*t)+0.1*sin(2*PI*520*t))*\
        (between(t\\,0\\,2)+between(t\\,2.8\\,4.6)+between(t\\,5.8\\,7.8)+between(t\\,9.4\\,12))+\
        0.01*sin(2*PI*60*t)";
    let input = format!("aevalsrc={expression}:s=16000:d=12");
    let path = testkit::oracle::generate(
        &dir,
        "pause candidates",
        "pauses.wav",
        &["-f", "lavfi", "-i", input.as_str(), "-c:a", "pcm_s16le"],
    )?;
    Some(SourceAsset::import(AssetId(1), path).unwrap())
}

fn video_fixture() -> Option<SourceAsset> {
    if !testkit::oracle::require("pause candidates") {
        return None;
    }
    let dir = testkit::fixture::dir("pause-candidates");
    let path = testkit::oracle::generate(
        &dir,
        "pause candidates",
        "clip.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=gray:s=64x36:r=8:d=12",
            "-c:v",
            "ffv1",
        ],
    )?;
    Some(SourceAsset::import(AssetId(2), path).unwrap())
}

fn audio_item(id: u64, start: Tick, source: SourceRange, asset: &SourceAsset) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: Some(asset.id()),
        source_range: source,
        start,
        content: ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
            cleanup: Default::default(),
        }),
    }
}

fn video_item(id: u64, start: Tick, source: SourceRange, asset: &SourceAsset) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: Some(asset.id()),
        source_range: source,
        start,
        content: ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
            cleanup: Default::default(),
            background: ClipBackground::None,
        }),
    }
}

fn caption_item(audio: &SourceAsset, word: SourceRange) -> Item {
    let language = captions::LanguageTag::parse("en").unwrap();
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(90),
        source_asset: Some(audio.id()),
        source_range: range(0, 12),
        start: Tick::ZERO,
        content: ItemParams::Caption(project::CaptionParams {
            emphasis: Default::default(),
            word_highlight: false,
            preset: None,
            overrides: captions::CaptionOverrides::default(),
            cues: captions::CueTrack {
                styles: Vec::new(),
                cues: vec![captions::Cue {
                    word_timing: crate::support::session::measured(&language, "Hello", &[word]),
                    id: "cue-1".into(),
                    range: range(0, 12),
                    source_lang: language.clone(),
                    texts: [(
                        language,
                        captions::CueText {
                            text: "Hello".into(),
                            edited: false,
                            translation_ref: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                    style: None,
                }],
            },
        }),
    }
}

/// One audio clip across the whole fixture plus an optional caption word.
fn single_clip(audio: &SourceAsset, word: Option<SourceRange>) -> Project {
    let mut data = Project::empty(ProjectId(0), "pauses".into())
        .unwrap()
        .into_data();
    data.assets.push(audio.clone());
    data.scene_mut()
        .timeline
        .insert_track(audio_track(vec![audio_item(
            1,
            Tick::ZERO,
            range(0, 12),
            audio,
        )]));
    if let Some(word) = word {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(9),
            name: "Captions".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![caption_item(audio, word)],
        });
    }
    Project::new(data).unwrap()
}

fn audio_track(items: Vec<Item>) -> Track {
    Track {
        id: TrackId(1),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items,
    }
}

/// Five aligned video/audio clips over the fixture: 0-2.9, 2.9-4, 4-6.5,
/// 6.5-9.6 and 9.6-12 s. Every pause sits inside one clip.
fn five_clips(audio: &SourceAsset, video: &SourceAsset) -> Project {
    let edges = [0, 2_900, 4_000, 6_500, 9_600, 12_000];
    let mut videos = Vec::new();
    let mut audios = Vec::new();
    for (index, pair) in edges.windows(2).enumerate() {
        let start = ms(pair[0]);
        let end = ms(pair[1]);
        let source = SourceRange::new(start, end).unwrap();
        videos.push(video_item(index as u64 + 1, start, source, video));
        audios.push(audio_item(index as u64 + 11, start, source, audio));
    }
    let mut data = Project::empty(ProjectId(0), "pauses".into())
        .unwrap()
        .into_data();
    data.assets.push(audio.clone());
    data.assets.push(video.clone());
    data.scene_mut().timeline.main.items = videos;
    data.scene_mut().timeline.insert_track(audio_track(audios));
    Project::new(data).unwrap()
}

fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(768 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}

fn wait(runner: &mut JobRunner, id: engine::jobs::JobId) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return state;
        }
        assert!(Instant::now() < deadline, "pause candidate timeout");
        std::thread::yield_now();
    }
}

fn candidates(project: Project, settings: PauseSettings) -> PauseCandidates {
    let request = PauseCandidatesRequest::new(Arc::new(project), settings).unwrap();
    let mut runner = runner();
    let job = request.submit(&mut runner).unwrap();
    assert_eq!(
        wait(&mut runner, job.id),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|snapshot| snapshot.error)
    );
    let result = job.finish(&mut runner).unwrap().unwrap();
    runner.shutdown();
    result
}

fn overlaps(a: &engine::pause_candidates::OutputSpan, word: &SourceRange) -> bool {
    word.start() < a.end && a.start < word.end()
}

#[test]
fn candidates_pad_skip_words_and_the_continuous_tail() {
    let Some(audio) = audio_fixture() else {
        return;
    };
    // A word over the middle pause removes that pause from the cut list.
    let word = SourceRange::new(ms(4_600), ms(5_800)).unwrap();
    let result = candidates(single_clip(&audio, Some(word)), PauseSettings::default());
    assert_eq!(result.analyzed_sources, 1);
    assert_eq!(result.spans.len(), 2, "{:?}", result.spans);
    let expected = [(2_000, 2_800), (7_800, 9_400)];
    for (span, (start, end)) in result.spans.iter().zip(expected) {
        for (actual, expected) in [(span.start, ms(start + 20)), (span.end, ms(end - 20))] {
            assert!(
                (actual.0 - expected.0).abs() <= ms(20).0,
                "candidate {actual:?} vs {expected:?}"
            );
        }
        assert!(!overlaps(span, &word));
    }
    // The continuous 9.4..12 s segment produces no candidate.
    assert!(result.spans.iter().all(|span| span.start < ms(9_400)));
}

#[test]
fn candidate_job_cancels_and_reports_typed_failures() {
    let Some(audio) = audio_fixture() else {
        return;
    };
    let request = PauseCandidatesRequest::new(
        Arc::new(single_clip(&audio, None)),
        PauseSettings::default(),
    )
    .unwrap();
    let mut runner = runner();
    let job = request.submit(&mut runner).unwrap();
    job.cancel.cancel();
    assert_eq!(wait(&mut runner, job.id), JobState::Cancelled);
    assert!(matches!(
        job.finish(&mut runner),
        Err(AnalysisError::Runtime(engine::RuntimeError::Cancelled))
    ));

    // A moved source fails the job with the media error.
    let copy = audio.path().with_file_name("candidate-unreadable.wav");
    std::fs::copy(audio.path(), &copy).unwrap();
    let missing = SourceAsset::import(AssetId(3), copy.clone()).unwrap();
    std::fs::remove_file(&copy).unwrap();
    let project = single_clip(&missing, None);
    let request = PauseCandidatesRequest::new(Arc::new(project), PauseSettings::default()).unwrap();
    let job = request.submit(&mut runner).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Failed);
    assert!(matches!(
        job.finish(&mut runner),
        Err(AnalysisError::Runtime(engine::RuntimeError::Failed(inner))) if matches!((inner.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<AnalysisError>(), Some(AnalysisError::Media(_)))
    ));
    runner.shutdown();
}

/// Minimal render hooks for an offline mix in a test.
struct Hooks;
struct Lease;
impl render::Reservation for Lease {}
impl render::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(Lease))
    }
}

fn mix(project: &Project, start: i64, frames: usize) -> Vec<f32> {
    let block = render::mix_audio(
        project,
        start,
        frames,
        1,
        &mut render::FileSources::default(),
        &Hooks,
    )
    .unwrap();
    block.planes().first().cloned().unwrap_or_default()
}

fn max_delta(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

fn sample(at: Tick) -> i64 {
    at.to_units(48_000, Rounding::Nearest).unwrap()
}

#[test]
fn removal_keeps_five_clips_aligned_and_joins_are_click_free() {
    let Some(audio) = audio_fixture() else {
        return;
    };
    let Some(video) = video_fixture() else {
        return;
    };
    let project = five_clips(&audio, &video);
    let settings = PauseSettings::default();
    let found = candidates(project, settings);
    assert_eq!(found.spans.len(), 3, "{:?}", found.spans);

    let mut session = Session::new(five_clips(&audio, &video), 10).unwrap();
    let initial = project::encode(session.project(), false).unwrap();
    let outcome = session
        .apply(engine::Command::RemovePauses {
            spans: found.spans.clone(),
            crossfade_ms: settings.crossfade_ms,
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(session.history_len(), (1, 0));
    let edited = project::encode(session.project(), false).unwrap();

    // Each remaining video piece has an audio partner starting at the same output tick.
    let data = session.project().data().clone();
    let starts = |kind: TrackKind| -> Vec<Tick> {
        let mut starts: Vec<Tick> = data
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == kind)
            .flat_map(|track| track.items.iter().map(|item| item.start))
            .collect();
        starts.sort_unstable();
        starts
    };
    let videos = starts(TrackKind::Video);
    let audios = starts(TrackKind::Audio);
    assert_eq!(videos.len(), 8);
    assert_eq!(videos, audios);
    let saved: i64 = found
        .spans
        .iter()
        .map(|span| span.end.0 - span.start.0)
        .sum();
    assert_eq!(
        data.scene()
            .timeline
            .main
            .items
            .last()
            .unwrap()
            .end()
            .unwrap(),
        t(12) - Tick(saved)
    );

    // Joins are the fade-in edges of the surviving right pieces.
    let joins: Vec<i64> = data
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == TrackKind::Audio)
        .flat_map(|track| track.items.iter())
        .filter(|item| match &item.content {
            ItemParams::Audio(params) => params.fades.fade_in.window() > Tick::ZERO,
            _ => false,
        })
        .map(|item| sample(item.start))
        .collect();
    assert_eq!(joins.len(), 3);
    let mixed = session.project().clone();
    let joined: Vec<f32> = joins
        .iter()
        .map(|join| {
            let start = join - 48;
            max_delta(&mix(&mixed, start, 192))
        })
        .collect();
    let reference = max_delta(&mix(&five_clips(&audio, &video), sample(ms(5_000)), 960));
    assert!(reference > 0.0);
    for (join, delta) in joins.iter().zip(&joined) {
        assert!(
            *delta <= reference * 4.0 + 1e-6,
            "join {join} delta {delta} vs room tone {reference}"
        );
    }

    // A zero crossfade splices room tone and must show the discontinuity the check detects.
    let mut abrupt = Session::new(five_clips(&audio, &video), 10).unwrap();
    abrupt
        .apply(engine::Command::RemovePauses {
            spans: found.spans.clone(),
            crossfade_ms: 0,
        })
        .unwrap();
    let abrupt_project = abrupt.project().clone();
    let abrupt_delta = joins
        .iter()
        .map(|join| max_delta(&mix(&abrupt_project, join - 48, 192)))
        .fold(0.0, f32::max);
    assert!(
        abrupt_delta > reference * 20.0,
        "abrupt join {abrupt_delta} vs room tone {reference}"
    );

    // Undo restores the exact bytes; redo restores the edited project.
    session.undo().unwrap();
    assert_eq!(project::encode(session.project(), false).unwrap(), initial);
    session.redo().unwrap();
    assert_eq!(project::encode(session.project(), false).unwrap(), edited);
    let loaded = project::decode(&edited).unwrap().project;
    assert_eq!(&loaded, session.project().as_ref());
}

/// Manual check against the real 30-minute interview. Not part of CI: the sample
/// is local and may be absent, and the file carries no transcript to guard.
#[test]
#[ignore = "manual: set VIBECUT_INTERVIEW_SAMPLE to interview-30m.mp4"]
fn interview_first_300_seconds_lists_candidates_and_their_cost() {
    let Some(path) = std::env::var_os("VIBECUT_INTERVIEW_SAMPLE").map(std::path::PathBuf::from)
    else {
        eprintln!("skipping: VIBECUT_INTERVIEW_SAMPLE is not set");
        return;
    };
    if !path.is_file() {
        eprintln!("skipping: {} does not exist", path.display());
        return;
    }
    let asset = SourceAsset::import(AssetId(9), path).unwrap();
    let mut data = Project::empty(ProjectId(0), "interview".into())
        .unwrap()
        .into_data();
    data.assets.push(asset.clone());
    let item = Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(1),
        source_asset: Some(asset.id()),
        source_range: range(0, 300),
        start: Tick::ZERO,
        content: ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
            cleanup: Default::default(),
            background: ClipBackground::None,
        }),
    };
    data.scene_mut().timeline.main.items = vec![item];
    let started = Instant::now();
    let found = candidates(Project::new(data).unwrap(), PauseSettings::default());
    let elapsed = started.elapsed();
    for span in &found.spans {
        eprintln!(
            "candidate {:.3} {:.3}",
            span.start.seconds(),
            span.end.seconds()
        );
    }
    eprintln!(
        "{} candidates in {:.1} s (no artifact cache; no transcript attached)",
        found.spans.len(),
        elapsed.as_secs_f64()
    );
    assert!(!found.spans.is_empty());
    assert_eq!(found.analyzed_sources, 1);
}
