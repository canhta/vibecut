//! Bounded real/synthetic input measurement; transcript stays in local files.
use ai::clips::{ClipRole, ClipSourceSegment};
use captions::{LanguageRegistry, LayoutHooks, Transcript};
use shorts::clip_boundaries::{ClipBoundarySettings, snap_clip_boundaries};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};

fn main() {
    if let Err(error) = run() {
        eprintln!("clip boundary benchmark failed: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let media_path = PathBuf::from(args.next().ok_or("media path required")?);
    let transcript_path = PathBuf::from(args.next().ok_or("local transcript JSON required")?);
    let transcript: Transcript = serde_json::from_slice(&std::fs::read(transcript_path)?)?;
    let registry = LanguageRegistry::standard()?;
    let profile = registry.profile(&transcript.source_lang);
    let units = captions::source_speech_units(&transcript, profile, &LayoutHooks::default())?;
    let selected: Vec<_> = transcript
        .segments
        .iter()
        .filter(|s| s.range.duration() >= Tick(TICKS_PER_SECOND))
        .take(4)
        .collect();
    if selected.len() < 3 {
        return Err("at least three measured segments required".into());
    }
    let mut segments = Vec::new();
    // Fixed labelled injection: reverse source order; move each edge 70 ms into speech.
    let inset = Tick(7 * TICKS_PER_SECOND / 100);
    for (rank, segment) in selected.iter().enumerate() {
        let inset = Tick(inset.0.min(segment.range.duration().0 / 4));
        segments.push(ClipSourceSegment {
            range: SourceRange::new(segment.range.start() + inset, segment.range.end() - inset)?,
            role: ClipRole::Payoff,
            source_order: rank,
            repair: None,
        });
    }
    segments.reverse();
    if let Some(first) = segments.first_mut() {
        first.role = ClipRole::Hook;
    }
    let shorts = vec![segments];
    let source_range = SourceRange::new(Tick::ZERO, Tick(60 * TICKS_PER_SECOND))?;
    let settings = ClipBoundarySettings {
        source_range,
        frame_rate: FrameRate::FPS_30,
        tolerance: Tick(TICKS_PER_SECOND / 2),
        silence_rms: 0.01,
        min_duration: Tick(1),
        max_duration: Tick(60 * TICKS_PER_SECOND),
        min_segment_duration: Tick(1),
        max_segments: 4,
    };
    let asset = media::SourceAsset::import(media::AssetId(1), media_path)?;
    let start = Instant::now();
    let energy =
        media::analyze_speech_energy(&asset, source_range, Arc::new(AtomicBool::new(false)))?;
    let energy_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let output = snap_clip_boundaries(
        &shorts,
        &transcript,
        profile,
        &LayoutHooks::default(),
        &energy,
        &settings,
    )?;
    let cold_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(snap_clip_boundaries(
            &shorts,
            &transcript,
            profile,
            &LayoutHooks::default(),
            &energy,
            &settings,
        )?);
    }
    let warm_seconds = start.elapsed().as_secs_f64() / 1000.0;
    let unsafe_edge = |tick| {
        units
            .iter()
            .any(|u| u.range.start() < tick && tick < u.range.end())
    };
    let mut baseline_mid_speech = 0;
    let mut adjusted_mid_speech = 0;
    for short in &shorts {
        for segment in short {
            for edge in [segment.range.start(), segment.range.end()] {
                baseline_mid_speech +=
                    usize::from(unsafe_edge(settings.frame_rate.snap_nearest(edge)?));
            }
        }
    }
    for short in &output {
        for segment in &short.segments {
            for edge in [segment.range.start(), segment.range.end()] {
                adjusted_mid_speech += usize::from(unsafe_edge(edge));
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"energy_windows":energy.len(),"energy_seconds":energy_seconds,"cold_snap_seconds":cold_seconds,"warm_snap_seconds_per_run":warm_seconds,"warm_repetitions":1000,"baseline_mid_speech":baseline_mid_speech,"adjusted_mid_speech":adjusted_mid_speech,"shorts":output})
        )?
    );
    if adjusted_mid_speech != 0 {
        return Err("adjusted edge entered measured speech".into());
    }
    Ok(())
}
