#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and explicit assertions"
)]
use media::{
    AssetId, MediaError, PEAK_BUCKET_SAMPLES, PEAK_SUMMARY_MAX_BYTES, PeakSummary, SourceAsset,
    peak_summary,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use time::{SourceRange, Tick};

fn range(start_seconds: f64, end_seconds: f64) -> SourceRange {
    SourceRange::new(
        Tick::from_seconds_f64(start_seconds, time::Rounding::Floor).unwrap(),
        Tick::from_seconds_f64(end_seconds, time::Rounding::Ceil).unwrap(),
    )
    .unwrap()
}

fn fixture(name: &str, args: &[&str]) -> Option<PathBuf> {
    testkit::oracle::generate(
        &testkit::fixture::dir("waveform-peaks"),
        "waveform peaks",
        name,
        args,
    )
}

fn asset(path: &Path, id: u64) -> SourceAsset {
    SourceAsset::import(AssetId(id), path.to_path_buf()).unwrap()
}

fn summary(path: &Path) -> PeakSummary {
    peak_summary(&asset(path, 1), Arc::new(AtomicBool::new(false))).unwrap()
}

/// Decode the fixture through the external oracle as interleaved s16le PCM.
fn raw_pcm(path: &Path, channels: usize) -> Vec<Vec<i16>> {
    let out = path.with_extension("s16le");
    let _ = std::fs::remove_file(&out);
    let output = testkit::oracle::command("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-f", "s16le", "-acodec", "pcm_s16le"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    let (chunks, remainder) = bytes.as_chunks::<2>();
    assert!(remainder.is_empty(), "s16le byte count");
    let samples: Vec<i16> = chunks
        .iter()
        .map(|chunk| i16::from_le_bytes(*chunk))
        .collect();
    (0..channels)
        .map(|channel| {
            samples
                .chunks_exact(channels)
                .map(|frame| frame[channel])
                .collect()
        })
        .collect()
}

fn stereo_source() -> Option<PathBuf> {
    fixture(
        "stereo-16bit.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.5*sin(2*PI*100*t)|0.9*sin(2*PI*300*t):s=8000:d=0.5",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

fn mono_source() -> Option<PathBuf> {
    fixture(
        "mono-16bit.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.25*sin(2*PI*220*t):s=8000:d=0.5",
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

fn loud_source() -> Option<PathBuf> {
    fixture(
        "loud-float.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=1.2*sin(2*PI*440*t):s=8000:d=0.25",
            "-ac",
            "1",
            "-c:a",
            "pcm_f32le",
        ],
    )
}

#[test]
fn stereo_buckets_match_independently_decoded_samples_per_channel() {
    let Some(path) = stereo_source() else {
        return;
    };
    let peaks = summary(&path);
    assert_eq!(peaks.sample_rate(), 8_000);
    assert_eq!(peaks.channels(), 2);
    assert_eq!(peaks.anchor(), Tick::ZERO);
    assert_eq!(peaks.total_samples(), 4_000);
    assert_eq!(peaks.bucket_count(), 32);
    assert_eq!(
        peaks.bucket_count(),
        4_000_usize.div_ceil(PEAK_BUCKET_SAMPLES as usize)
    );

    let pcm = raw_pcm(&path, 2);
    let mut error: f32 = 0.0;
    for bucket in 0..peaks.bucket_count() {
        let start = bucket * PEAK_BUCKET_SAMPLES as usize;
        let end = (start + PEAK_BUCKET_SAMPLES as usize).min(4_000);
        for (channel, samples) in pcm.iter().enumerate().take(2) {
            let slice = &samples[start..end];
            let expected_min = slice
                .iter()
                .map(|sample| f32::from(*sample) / 32768.0)
                .fold(f32::INFINITY, f32::min);
            let expected_max = slice
                .iter()
                .map(|sample| f32::from(*sample) / 32768.0)
                .fold(f32::NEG_INFINITY, f32::max);
            let observed = peaks
                .peak(bucket, channel as u16)
                .expect("every bucket and channel is present");
            error = error.max((observed.min - expected_min).abs());
            error = error.max((observed.max - expected_max).abs());
        }
    }
    // Decoding s16 to f32 is exact (sample / 32768); the stated tolerance is
    // one 16-bit step and the observed error is printed for the report.
    println!("waveform peak max error vs decoded s16: {error}");
    assert!(error <= 1.0 / 32768.0, "peak max error {error}");
    assert!(!(0..peaks.bucket_count()).any(|bucket| peaks.over(bucket)));
}

#[test]
fn mono_is_deterministic_and_has_one_channel() {
    let Some(path) = mono_source() else {
        return;
    };
    let first = summary(&path);
    let second = summary(&path);
    assert_eq!(first, second, "repeated analysis must be deterministic");
    assert_eq!(first.channels(), 1);
    assert_eq!(first.total_samples(), 4_000);
    for bucket in 0..first.bucket_count() {
        let peak = first.peak(bucket, 0).unwrap();
        assert!(peak.min <= peak.max);
        assert!(peak.min >= -1.0 && peak.max <= 1.0);
        assert!(first.peak(bucket, 1).is_none());
    }
}

#[test]
fn samples_over_zero_dbfs_flag_their_bucket() {
    let Some(path) = loud_source() else {
        return;
    };
    let peaks = summary(&path);
    assert_eq!(peaks.channels(), 1);
    let over: Vec<usize> = (0..peaks.bucket_count())
        .filter(|bucket| peaks.over(*bucket))
        .collect();
    assert!(!over.is_empty(), "1.2 amplitude must leave [-1, 1]");
    for bucket in over {
        let peak = peaks.peak(bucket, 0).unwrap();
        assert!(peak.min < -1.0 || peak.max > 1.0);
    }
    let Some(quiet) = mono_source() else {
        return;
    };
    let quiet = summary(&quiet);
    assert!(
        (0..quiet.bucket_count()).all(|bucket| !quiet.over(bucket)),
        "0.25 amplitude stays under 0 dBFS"
    );
}

#[test]
fn bucket_span_clamps_to_the_decoded_source_grid() {
    let Some(path) = stereo_source() else {
        return;
    };
    let peaks = summary(&path);
    let span = peaks.bucket_span(range(0.1, 0.2)).unwrap();
    assert_eq!(span.start, 800 / PEAK_BUCKET_SAMPLES as usize);
    assert_eq!(
        span.end,
        (1_600_usize).div_ceil(PEAK_BUCKET_SAMPLES as usize)
    );
    let first = peaks.bucket_source_range(span.start).unwrap();
    let last = peaks.bucket_source_range(span.end - 1).unwrap();
    assert!(first.start() <= Tick::from_seconds_f64(0.1, time::Rounding::Floor).unwrap());
    assert!(last.end() >= Tick::from_seconds_f64(0.2, time::Rounding::Ceil).unwrap());
    for bucket in 0..peaks.bucket_count() {
        let range = peaks.bucket_source_range(bucket).unwrap();
        if bucket + 1 < peaks.bucket_count() {
            assert_eq!(
                range.end(),
                peaks.bucket_source_range(bucket + 1).unwrap().start()
            );
        }
    }
    assert!(peaks.bucket_source_range(peaks.bucket_count()).is_none());
    assert!(peaks.bucket_span(range(10.0, 11.0)).is_none());
    assert!(
        peaks
            .bucket_span(range(0.0, 0.001))
            .is_some_and(|span| span.start == 0)
    );
}

#[test]
fn encoded_summary_round_trips_and_rejects_hostile_bytes() {
    let Some(path) = stereo_source() else {
        return;
    };
    let peaks = summary(&path);
    let bytes = peaks.encode().unwrap();
    assert_eq!(PeakSummary::decode(&bytes).unwrap(), peaks);
    assert!(bytes.len() <= PEAK_SUMMARY_MAX_BYTES);

    let mut cases: Vec<Vec<u8>> = Vec::new();
    let mut short = bytes.clone();
    short.truncate(20);
    cases.push(short);
    let mut magic = bytes.clone();
    magic[0] = b'X';
    cases.push(magic);
    let mut rate = bytes.clone();
    rate[8..12].copy_from_slice(&1_000_u32.to_le_bytes());
    cases.push(rate);
    let mut channels = bytes.clone();
    channels[12..14].copy_from_slice(&3_u16.to_le_bytes());
    cases.push(channels);
    let mut bucket = bytes.clone();
    bucket[14..18].copy_from_slice(&256_u32.to_le_bytes());
    cases.push(bucket);
    let mut total = bytes.clone();
    total[26..34].copy_from_slice(&0_u64.to_le_bytes());
    cases.push(total);
    let mut count = bytes.clone();
    count.extend_from_slice(&[0, 0, 0, 0]);
    cases.push(count);
    let mut not_finite = bytes.clone();
    not_finite[34..38].copy_from_slice(&f32::NAN.to_le_bytes());
    cases.push(not_finite);
    let mut inverted = bytes;
    let end = inverted.len();
    inverted[end - 8..end - 4].copy_from_slice(&0.9_f32.to_le_bytes());
    inverted[end - 4..end].copy_from_slice(&0.1_f32.to_le_bytes());
    cases.push(inverted);
    for bytes in cases {
        assert!(
            PeakSummary::decode(&bytes).is_err(),
            "hostile summary must be rejected"
        );
    }
}

#[test]
fn audio_less_sources_and_early_cancellation_are_typed_failures() {
    let Some(dir) = stereo_source().and_then(|path| path.parent().map(Path::to_path_buf)) else {
        return;
    };
    let video = testkit::oracle::generate(
        &dir,
        "waveform peaks",
        "video-only.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=black:s=32x32:r=10:d=1",
            "-c:v",
            "ffv1",
        ],
    )
    .unwrap();
    assert!(matches!(
        peak_summary(&asset(&video, 2), Arc::new(AtomicBool::new(false))),
        Err(MediaError::NoAudioStream)
    ));

    let Some(path) = stereo_source() else {
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let peaks = peak_summary(&asset(&path, 4), cancel.clone()).unwrap();
    assert!(peaks.total_samples() > 0);
    cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        peak_summary(&asset(&path, 4), cancel),
        Err(MediaError::Cancelled)
    ));
}

#[test]
#[ignore = "two-hour decode; run manually for streaming evidence"]
fn two_hour_source_stays_bounded() {
    if !testkit::oracle::require("waveform peaks") {
        return;
    }
    let Some(path) = testkit::oracle::generate(
        &testkit::fixture::dir("waveform-peaks"),
        "waveform peaks",
        "waveform_2h.m4a",
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
    ) else {
        return;
    };
    let started = std::time::Instant::now();
    let peaks = summary(&path);
    let elapsed = started.elapsed();
    let encoded = peaks.encode().unwrap();
    println!(
        "waveform two hours: samples={} buckets={} summary_bytes={} elapsed={elapsed:?}",
        peaks.total_samples(),
        peaks.bucket_count(),
        encoded.len()
    );
    assert!(
        peaks.total_samples() >= 7_200 * 48_000 - 8_000,
        "decoded sample count"
    );
    assert!(encoded.len() <= PEAK_SUMMARY_MAX_BYTES);
}
