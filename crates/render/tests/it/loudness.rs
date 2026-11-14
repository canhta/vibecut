#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use crate::support::*;
use project::LoudnessTarget;
use render::*;
use std::{ffi::OsStr, path::Path};
use time::{Tick, TimeRange};

/// Stereo test signal: a 997 Hz sine at `level` plus optional single-sample
/// spikes every `spike_every` frames, deterministic on the absolute sample index.
struct Tone {
    level: f32,
    spike: Option<(i64, f32)>,
}
impl SourceProvider for Tone {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                let at = start + n as i64;
                *sample = self.level
                    * (2.0 * std::f64::consts::PI * 997.0 * at as f64 / f64::from(rate)).sin()
                        as f32;
                if let Some((every, height)) = self.spike
                    && at % every == 0
                {
                    *sample = height;
                }
            }
        }
        Ok(out)
    }
}

fn audio_asset(id: u64, path: std::path::PathBuf, seconds: i64, rate: u32) -> media::SourceAsset {
    let duration = tick(seconds * i64::from(rate), i64::from(rate));
    testkit::source::asset(id, path)
        .format("wav")
        .duration(duration)
        .content_hash(media::ContentHash::from_bytes(&[id as u8]))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, rate as i32).unwrap())
                .duration(duration)
                .sample_rate(rate)
                .build(),
        )
        .build()
}

/// One audio clip per `(asset, start_seconds, seconds)` on one audio track.
fn project_of(
    assets: Vec<media::SourceAsset>,
    clips: &[(u64, i64, i64)],
    clip: Option<LoudnessTarget>,
    master: Option<LoudnessTarget>,
) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(1), "".into())
        .unwrap()
        .into_data();
    data.assets = assets;
    data.settings.loudness.clip = clip;
    data.settings.loudness.master = master;
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: clips
            .iter()
            .enumerate()
            .map(|(n, &(asset, start, seconds))| {
                testkit::item::audio(project::ItemId(n as u64 + 1))
                    .asset(media::AssetId(asset))
                    .source(TimeRange::new(Tick::ZERO, tick(seconds, 1)).unwrap())
                    .start(tick(start, 1))
                    .build()
            })
            .collect(),
    });
    project::Project::new(data).unwrap()
}

const TARGET: LoudnessTarget = LoudnessTarget::DEFAULT;

fn tone_project(clip: Option<LoudnessTarget>, master: Option<LoudnessTarget>) -> project::Project {
    project_of(
        vec![audio_asset(
            1,
            std::env::temp_dir().join("tone.wav"),
            6,
            48000,
        )],
        &[(1, 0, 6)],
        clip,
        master,
    )
}

fn db(linear: f32) -> f64 {
    20.0 * f64::from(linear).log10()
}

/// ffmpeg's ebur128 summary: integrated LUFS and true peak dBTP.
fn oracle(path: &Path) -> (f64, f64) {
    let out = tool_command("ffmpeg")
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(path)
        .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stderr);
    let summary = &text[text.rfind("Summary:").unwrap()..];
    let value = |label: &str| -> f64 {
        let line = summary
            .lines()
            .find(|l| l.trim_start().starts_with(label))
            .unwrap();
        line.split_whitespace().nth(1).unwrap().parse().unwrap()
    };
    (value("I:"), value("Peak:"))
}

#[test]
fn measurement_matches_ffmpeg_ebur128_on_three_fixtures() {
    if !tools() {
        return;
    }
    let dir = root().join("loudness");
    std::fs::create_dir_all(&dir).unwrap();
    let make = |name: &str, input: &[&str], filter: &str| {
        let path = dir.join(name);
        let mut args: Vec<&OsStr> = Vec::new();
        for part in input {
            args.push(OsStr::new(part));
        }
        let filter_flag = if filter.contains("concat") {
            "-filter_complex"
        } else {
            "-af"
        };
        args.extend([
            OsStr::new(filter_flag),
            OsStr::new(filter),
            OsStr::new("-ar"),
            OsStr::new("48000"),
            OsStr::new("-ac"),
            OsStr::new("2"),
            OsStr::new("-c:a"),
            OsStr::new("pcm_f32le"),
            path.as_os_str(),
        ]);
        ffmpeg(&args);
        path
    };
    // Steady tone, broadband noise, and a gated programme (loud, silence, quiet).
    let fixtures = [
        make(
            "tone.wav",
            &["-f", "lavfi", "-i", "sine=f=997:d=6:r=48000"],
            "volume=-23dB",
        ),
        make(
            "noise.wav",
            &[
                "-f",
                "lavfi",
                "-i",
                "anoisesrc=d=8:c=pink:r=48000:a=0.3:seed=7",
            ],
            "anull",
        ),
        make(
            "gated.wav",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:d=3:r=48000",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=f=1500:d=3:r=48000",
            ],
            "[0:a]volume=-6dB,aformat=channel_layouts=stereo[a];\
             [2:a]volume=-35dB,aformat=channel_layouts=stereo[c];\
             [a][1:a][c]concat=n=3:v=0:a=1",
        ),
    ];
    let h = Hooks::default();
    for (n, path) in fixtures.iter().enumerate() {
        let asset = media::SourceAsset::import(media::AssetId(1), path.clone()).unwrap();
        let seconds = asset.probe().duration.unwrap().seconds().ceil() as i64;
        let p = project_of(vec![asset], &[(1, 0, seconds)], Some(TARGET), None);
        let report = measure_loudness(&p, &mut FileSources::default(), &h).unwrap();
        let (_, measured) = report.clips[0];
        let (lufs, peak) = oracle(path);
        let got = measured.integrated_lufs.unwrap();
        eprintln!(
            "fixture {n}: ours {got:.2} LUFS {:.2} dBTP, ffmpeg {lufs} LUFS {peak} dBTP",
            measured.true_peak_db.unwrap()
        );
        assert!(
            (got - lufs).abs() <= 0.5,
            "fixture {n}: integrated {got} vs ffmpeg {lufs}"
        );
        let got_peak = measured.true_peak_db.unwrap();
        assert!(
            (got_peak - peak).abs() <= 0.1,
            "fixture {n}: true peak {got_peak} vs ffmpeg {peak}"
        );
    }
}

#[test]
fn reference_tone_measures_its_defined_loudness() {
    // BS.1770: a 997 Hz sine in both channels reads its own peak level in LUFS.
    let h = Hooks::default();
    let mut tone = Tone {
        level: 0.1,
        spike: None,
    };
    let report = measure_loudness(&tone_project(Some(TARGET), None), &mut tone, &h).unwrap();
    let m = report.clips[0].1;
    assert!((m.integrated_lufs.unwrap() + 20.0).abs() < 0.05);
    assert!((m.true_peak_db.unwrap() + 20.0).abs() < 0.05);
    // The -20 LUFS tone moves to the default target.
    let gain = f64::from(TARGET.target_lufs) + 20.0;
    assert!((f64::from(report.gains.clip[0].1) - gain).abs() < 0.05);
}

#[test]
fn normalization_is_identity_for_silence_and_normalized_material() {
    let h = Hooks::default();
    for (level, scope) in [
        (0.0, "silence"),
        (10f32.powf(TARGET.target_lufs / 20.0), "normalized"),
    ] {
        let mut tone = Tone { level, spike: None };
        let report =
            measure_loudness(&tone_project(Some(TARGET), Some(TARGET)), &mut tone, &h).unwrap();
        assert!(report.gains.clip.is_empty(), "{scope}");
        assert_eq!(report.gains.master_db, 0.0, "{scope}");
    }
}

#[test]
fn gain_never_crosses_the_true_peak_ceiling() {
    // Quiet tone with one loud spike per second: the target is unreachable.
    let h = Hooks::default();
    let mut tone = Tone {
        level: 0.03,
        spike: Some((48000, 0.5)),
    };
    let report = measure_loudness(&tone_project(None, Some(TARGET)), &mut tone, &h).unwrap();
    let m = report.master.unwrap();
    let gain = f64::from(report.gains.master_db);
    assert!(gain > 0.0);
    assert!(
        m.integrated_lufs.unwrap() + gain < -14.0 - 0.1,
        "limited by peak"
    );
    assert!(m.true_peak_db.unwrap() + gain <= -1.0 + 0.01);
}

#[test]
fn clip_and_master_compose_without_double_normalizing() {
    // Two quiet clips back to back, each normalized alone; the mix is then
    // already at target, so the master pass adds nothing.
    let h = Hooks::default();
    let p = project_of(
        vec![audio_asset(
            1,
            std::env::temp_dir().join("tone.wav"),
            4,
            48000,
        )],
        &[(1, 0, 4), (1, 4, 4)],
        Some(TARGET),
        Some(TARGET),
    );
    let mut tone = Tone {
        level: 0.05,
        spike: None,
    };
    let report = measure_loudness(&p, &mut tone, &h).unwrap();
    assert_eq!(report.gains.clip.len(), 2);
    assert!(report.master.unwrap().integrated_lufs.unwrap() + 14.0 < 0.1);
    assert_eq!(report.gains.master_db, 0.0);
}

#[test]
fn applied_gains_scale_preview_and_export_mix_identically() {
    let h = Hooks::default();
    let mut tone = Tone {
        level: 0.05,
        spike: None,
    };
    let p = tone_project(
        Some(TARGET),
        Some(LoudnessTarget {
            target_lufs: -20.0,
            true_peak_db: -1.0,
        }),
    );
    let report = measure_loudness(&p, &mut tone, &h).unwrap();
    let plain = mix_audio(&p, 12000, 4800, 2, &mut tone, &h).unwrap();
    let mut data = p.into_data();
    data.settings.loudness.measured = Some(report.gains.clone());
    let p = project::Project::new(data).unwrap();
    let realtime = mix_audio(&p, 12000, 4800, 2, &mut tone, &h).unwrap();
    let mut offline = AudioMixer::new(&p, 2, 0, &h).unwrap();
    let export = offline.mix(12000, 4800, &mut tone, &h).unwrap();
    assert_eq!(realtime.planes(), export.planes());
    let total = f64::from(report.gains.clip[0].1) + f64::from(report.gains.master_db);
    let ratio = db(realtime.peak()) - db(plain.peak());
    assert!((ratio - total).abs() < 0.01, "{ratio} vs {total}");
}

#[test]
fn measurement_reports_progress_and_stops_on_cancel() {
    let stages = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = stages.clone();
    let h = Hooks {
        event: Some(std::sync::Arc::new(move |stage| {
            if stage == Stage::Mix {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(())
        })),
        ..Hooks::default()
    };
    let mut tone = Tone {
        level: 0.1,
        spike: None,
    };
    measure_loudness(&tone_project(Some(TARGET), None), &mut tone, &h).unwrap();
    assert!(stages.load(std::sync::atomic::Ordering::SeqCst) > 2);
    let cancelled = Hooks::default();
    cancelled
        .state
        .store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        measure_loudness(&tone_project(Some(TARGET), None), &mut tone, &cancelled),
        Err(Error::Cancelled)
    ));
}
