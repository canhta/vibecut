//! Speech-window splitting exercised through the public FFmpeg-backed interface.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use media::{
    AssetId, MediaError, SPEECH_SAMPLE_RATE, SourceAsset, SpeechExtractSettings, SpeechWindow,
    extract_speech_windows_with_cancel,
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

const SAMPLE_TICKS: i64 = TICKS_PER_SECOND / SPEECH_SAMPLE_RATE as i64;

fn seconds(value: f64) -> Tick {
    Tick::from_seconds_f64(value, Rounding::Nearest).unwrap()
}

fn settings(max_duration: f64, overlap: f64) -> SpeechExtractSettings {
    SpeechExtractSettings {
        max_duration: seconds(max_duration),
        overlap: seconds(overlap),
    }
}

fn asset(path: &Path, id: u64) -> SourceAsset {
    SourceAsset::import(AssetId(id), path.to_path_buf()).unwrap()
}

fn speech_source() -> Option<PathBuf> {
    testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "speech chunking",
        "speech_30s.m4a",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=30",
            "-ac",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
        ],
    )
}

fn silence_source() -> Option<PathBuf> {
    testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "speech silence",
        "speech_silence_12s.m4a",
        &[
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "12",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
        ],
    )
}

fn video_source() -> Option<PathBuf> {
    testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "speech video only",
        "speech_video_only.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-an",
        ],
    )
}

fn collect(
    asset: &SourceAsset,
    range: SourceRange,
    settings: &SpeechExtractSettings,
) -> Vec<SpeechWindow> {
    let mut windows = Vec::new();
    extract_speech_windows_with_cancel(
        asset,
        range,
        settings,
        Arc::new(AtomicBool::new(false)),
        |window| {
            windows.push(window);
            Ok(())
        },
    )
    .unwrap();
    windows
}

#[test]
fn speech_windows_tile_each_range_with_lowest_energy_splits() {
    let Some(source) = speech_source() else {
        return;
    };
    let asset = asset(&source, 1);
    // (range start, range end, max duration, overlap, minimum windows)
    let rows = [
        (0.0, 30.0, 10.0, 0.0, 3),
        (0.0, 30.0, 10.0, 2.0, 3),
        (5.0, 15.0, 6.0, 1.0, 2),
    ];
    for (start, end, max_duration, overlap, minimum) in rows {
        let range = SourceRange::new(seconds(start), seconds(end)).unwrap();
        let settings = settings(max_duration, overlap);
        let windows = collect(&asset, range, &settings);
        assert!(
            windows.len() >= minimum,
            "range {start}..{end}: only {} windows",
            windows.len()
        );
        let tolerance = Tick(SAMPLE_TICKS);
        let first = windows.first().unwrap();
        assert_eq!(first.overlap, Tick::ZERO);
        assert!(
            (first.source_range.start().0 - seconds(start).0).abs() <= tolerance.0,
            "start {:?}",
            first.source_range.start()
        );
        let last = windows.last().unwrap();
        assert!(
            (last.source_range.end().0 - seconds(end).0).abs() <= tolerance.0,
            "end {:?}",
            last.source_range.end()
        );
        for (index, window) in windows.iter().enumerate() {
            assert!(window.source_range.duration() <= seconds(max_duration));
            assert!(window.source_range.duration() > seconds(overlap));
            assert!(window.speech_detected);
            assert!(window.samples.iter().all(|sample| sample.is_finite()));
            assert_eq!(
                window.samples.len() as i64,
                window
                    .source_range
                    .duration()
                    .to_units(i64::from(SPEECH_SAMPLE_RATE), Rounding::Nearest)
                    .unwrap(),
                "range {start}..{end} window {index} sample count"
            );
            if index > 0 {
                let previous = windows[index - 1].source_range;
                assert_eq!(window.overlap, seconds(overlap));
                assert_eq!(
                    window.source_range.start(),
                    previous.end() - seconds(overlap)
                );
            }
        }
    }
}

#[test]
fn silent_windows_report_no_speech_hint() {
    let Some(source) = silence_source() else {
        return;
    };
    let range = SourceRange::new(Tick::ZERO, seconds(12.0)).unwrap();
    let windows = collect(&asset(&source, 2), range, &settings(10.0, 2.0));
    assert!(!windows.is_empty());
    assert!(windows.iter().all(|window| !window.speech_detected));
    assert_eq!(windows.last().unwrap().source_range.end(), seconds(12.0));
}

#[test]
fn video_only_input_returns_typed_no_audio_stream() {
    let Some(source) = video_source() else {
        return;
    };
    let range = SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap();
    assert!(matches!(
        extract_speech_windows_with_cancel(
            &asset(&source, 3),
            range,
            &SpeechExtractSettings::default(),
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        ),
        Err(MediaError::NoAudioStream)
    ));
}

#[test]
fn cancellation_before_decode_returns_cancelled() {
    let Some(source) = speech_source() else {
        return;
    };
    let cancel = Arc::new(AtomicBool::new(true));
    let range = SourceRange::new(Tick::ZERO, seconds(30.0)).unwrap();
    let result = extract_speech_windows_with_cancel(
        &asset(&source, 4),
        range,
        &settings(10.0, 2.0),
        cancel.clone(),
        |_| Ok(()),
    );
    assert!(matches!(result, Err(MediaError::Cancelled)));
    cancel.store(false, Ordering::Relaxed);
}

#[test]
fn window_callback_error_stops_extraction() {
    let Some(source) = speech_source() else {
        return;
    };
    let range = SourceRange::new(Tick::ZERO, seconds(30.0)).unwrap();
    let mut delivered = 0;
    let result = extract_speech_windows_with_cancel(
        &asset(&source, 5),
        range,
        &settings(10.0, 2.0),
        Arc::new(AtomicBool::new(false)),
        |_| {
            delivered += 1;
            Err(MediaError::Invalid("window callback"))
        },
    );
    assert!(matches!(
        result,
        Err(MediaError::Invalid("window callback"))
    ));
    assert_eq!(delivered, 1);
}
