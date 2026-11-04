//! Speech-audio extraction exercised through the public FFmpeg-backed interface.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod common;

use media::{
    AssetId, MediaError, SPEECH_SAMPLE_RATE, SourceAsset, SpeechExtractSettings,
    extract_speech_audio, extract_speech_audio_with, extract_speech_audio_with_cancel,
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
        ..SpeechExtractSettings::default()
    }
}

fn asset(path: &Path, id: u64) -> SourceAsset {
    SourceAsset::import(AssetId(id), path.to_path_buf()).unwrap()
}

fn out_dir(name: &str) -> PathBuf {
    let dir = common::fixtures_dir().join("speech").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn speech_source() -> Option<PathBuf> {
    common::generate(
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
    common::generate(
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
    common::generate(
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

fn long_source() -> Option<PathBuf> {
    common::generate(
        "speech two hours",
        "speech_2h.m4a",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=220:sample_rate=48000:duration=7200",
            "-ac",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "32k",
        ],
    )
}

fn assert_chunk_audio(path: &std::path::Path) {
    assert_eq!(common::stream_field(path, "a:0", "codec_name"), "aac");
    assert_eq!(common::stream_field(path, "a:0", "sample_rate"), "16000");
    assert_eq!(common::stream_field(path, "a:0", "channels"), "1");
    assert!(common::decodes(path));
}

fn assert_no_temp_files(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        assert_ne!(
            path.extension().and_then(|value| value.to_str()),
            Some("part")
        );
    }
}

#[test]
fn chunks_tile_the_range_with_exact_ticks_and_ffprobe_audio() {
    let Some(source) = speech_source() else {
        return;
    };
    let dir = out_dir("tile");
    let range = SourceRange::new(Tick::ZERO, seconds(30.0)).unwrap();
    let chunks =
        extract_speech_audio_with(&asset(&source, 1), range, &dir, &settings(10.0, 2.0)).unwrap();
    assert!(chunks.len() >= 3, "only {} chunks", chunks.len());
    let overlap = seconds(2.0);
    assert_eq!(chunks[0].overlap, Tick::ZERO);
    assert_eq!(chunks[0].source_range.start(), Tick::ZERO);
    assert_eq!(chunks.last().unwrap().source_range.end(), seconds(30.0));
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(chunk.source_range.duration() <= seconds(10.0));
        assert!(chunk.source_range.duration() > overlap);
        assert!(chunk.speech_detected);
        let size = std::fs::metadata(&chunk.path).unwrap().len();
        assert!(size > 0 && size <= 24_000_000, "chunk size {size}");
        assert_chunk_audio(&chunk.path);
        common::approx(
            common::duration(&chunk.path),
            chunk.source_range.duration().seconds(),
            0.05,
            "chunk duration",
        );
        if index > 0 {
            assert_eq!(chunk.overlap, overlap);
            assert_eq!(
                chunk.source_range.start(),
                chunks[index - 1].source_range.end() - overlap
            );
        }
        println!(
            "{} range={:?} duration={:.3}s size={size} speech={}",
            chunk.path.display(),
            chunk.source_range,
            chunk.source_range.duration().seconds(),
            chunk.speech_detected
        );
    }
    assert_no_temp_files(&dir);
}

#[test]
fn subrange_seek_trims_to_exact_source_ticks() {
    let Some(source) = speech_source() else {
        return;
    };
    let dir = out_dir("range");
    let range = SourceRange::new(seconds(5.0), seconds(15.0)).unwrap();
    let chunks =
        extract_speech_audio_with(&asset(&source, 2), range, &dir, &settings(6.0, 1.0)).unwrap();
    assert!(chunks.len() >= 2, "only {} chunks", chunks.len());
    let tolerance = Tick(SAMPLE_TICKS);
    assert!(
        (chunks[0].source_range.start().0 - seconds(5.0).0).abs() <= tolerance.0,
        "start {:?}",
        chunks[0].source_range.start()
    );
    assert!(
        (chunks.last().unwrap().source_range.end().0 - seconds(15.0).0).abs() <= tolerance.0,
        "end {:?}",
        chunks.last().unwrap().source_range.end()
    );
    let overlap = seconds(1.0);
    for pair in chunks.windows(2) {
        assert_eq!(pair[1].overlap, overlap);
        assert_eq!(
            pair[1].source_range.start(),
            pair[0].source_range.end() - overlap
        );
    }
    for chunk in &chunks {
        assert!(chunk.source_range.duration() <= seconds(6.0));
        assert!(chunk.speech_detected);
        assert_chunk_audio(&chunk.path);
    }
    assert_no_temp_files(&dir);
}

#[test]
fn silent_input_still_produces_chunks_without_speech_hint() {
    let Some(source) = silence_source() else {
        return;
    };
    let dir = out_dir("silence");
    let range = SourceRange::new(Tick::ZERO, seconds(12.0)).unwrap();
    let chunks =
        extract_speech_audio_with(&asset(&source, 1), range, &dir, &settings(10.0, 2.0)).unwrap();
    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(|chunk| !chunk.speech_detected));
    assert_eq!(chunks.last().unwrap().source_range.end(), seconds(12.0));
    for chunk in &chunks {
        assert_chunk_audio(&chunk.path);
    }
    assert_no_temp_files(&dir);
}

#[test]
fn byte_bound_keeps_every_file_under_the_limit() {
    let Some(source) = speech_source() else {
        return;
    };
    let dir = out_dir("bytes");
    let limit = 200_000;
    let settings = SpeechExtractSettings {
        max_bytes: limit,
        max_duration: seconds(60.0),
        overlap: seconds(0.5),
        ..SpeechExtractSettings::default()
    };
    let range = SourceRange::new(Tick::ZERO, seconds(30.0)).unwrap();
    let chunks = extract_speech_audio_with(&asset(&source, 4), range, &dir, &settings).unwrap();
    assert!(chunks.len() > 1, "only {} chunks", chunks.len());
    for chunk in &chunks {
        let size = std::fs::metadata(&chunk.path).unwrap().len();
        assert!(size <= limit, "chunk size {size} exceeds {limit}");
        assert!(chunk.source_range.duration() <= seconds(60.0));
        println!(
            "{} range={:?} size={size}",
            chunk.path.display(),
            chunk.source_range
        );
    }
    assert_no_temp_files(&dir);
}

#[test]
fn video_only_input_returns_typed_no_audio_stream() {
    let Some(source) = video_source() else {
        return;
    };
    let dir = out_dir("no_audio");
    let range = SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap();
    assert!(matches!(
        extract_speech_audio(&asset(&source, 5), range, &dir),
        Err(MediaError::NoAudioStream)
    ));
}

#[test]
fn cancellation_before_decode_returns_cancelled() {
    let Some(source) = speech_source() else {
        return;
    };
    let dir = out_dir("cancel");
    let cancel = Arc::new(AtomicBool::new(true));
    let range = SourceRange::new(Tick::ZERO, seconds(30.0)).unwrap();
    let result = extract_speech_audio_with_cancel(
        &asset(&source, 6),
        range,
        &dir,
        &settings(10.0, 2.0),
        cancel.clone(),
    );
    assert!(matches!(result, Err(MediaError::Cancelled)));
    cancel.store(false, Ordering::Relaxed);
}

#[test]
#[ignore = "two-hour decode; run manually for streaming evidence"]
fn two_hour_input_streams_with_bounded_memory() {
    let Some(source) = long_source() else {
        return;
    };
    let dir = out_dir("long");
    let range = SourceRange::new(Tick::ZERO, seconds(7200.0)).unwrap();
    let started = std::time::Instant::now();
    let chunks = extract_speech_audio(&asset(&source, 5), range, &dir).unwrap();
    let elapsed = started.elapsed();
    assert!(chunks.len() >= 10, "only {} chunks", chunks.len());
    common::approx(
        chunks.last().unwrap().source_range.end().seconds(),
        7200.0,
        0.1,
        "two-hour range end",
    );
    for chunk in &chunks {
        assert!(chunk.source_range.duration() <= seconds(600.0));
    }
    println!(
        "{} chunks covering {:.3}s in {elapsed:?}",
        chunks.len(),
        chunks.last().unwrap().source_range.end().seconds()
    );
    assert_no_temp_files(&dir);
}
