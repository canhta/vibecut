#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::common::*;
use render::*;
use std::{
    ffi::OsStr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use time::{Tick, TimeRange};
fn generate(path: &Path, args: &[&str]) {
    let mut all: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    all.push(path.as_os_str());
    ffmpeg(&all);
}
fn video_project(path: &Path) -> project::Project {
    let asset = media::SourceAsset::import(media::AssetId(1), path.to_owned()).unwrap();
    let mut d = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    let s = asset
        .probe()
        .streams
        .iter()
        .find_map(|s| s.video.as_ref())
        .unwrap();
    d.canvas.width = s.width;
    d.canvas.height = s.height;
    d.canvas.fps = asset
        .probe()
        .streams
        .iter()
        .find_map(|s| s.frame_rate)
        .unwrap();
    let duration = asset.probe().duration.unwrap();
    d.assets.push(asset);
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
            }),
        }],
    });
    project::Project::new(d).unwrap()
}
fn av_fixture(path: &Path) {
    generate(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x96:rate=30:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=997:sample_rate=44100:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "30",
            "-bf",
            "2",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-c:a",
            "aac",
            "-ac",
            "1",
            "-shortest",
        ],
    );
}
fn pcm(path: &Path, filter: Option<&str>) -> Vec<f32> {
    let mut command = tool_command("ffmpeg");
    command.args(["-v", "error", "-i"]).arg(path);
    let filter = filter.map_or_else(
        || "pan=mono|c0=c0".to_owned(),
        |f| format!("{f},pan=mono|c0=c0"),
    );
    command.args(["-af", &filter]);
    let out = command
        .args(["-f", "f32le", "-ar", "48000", "-ac", "1", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    out.stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}
#[test]
fn exact_non_keyframe_cut_preserves_pcm_sync_and_frame_bounds() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("源 video.mp4");
    av_fixture(&input);
    let p = video_project(&input);
    let h = Hooks::default();
    let range = TimeRange::new(tick(12, 30), tick(24, 30)).unwrap();
    let mut settings = ExportSettings::from_project(&p);
    if cfg!(windows) {
        assert!(matches!(
            settings.encoder,
            media::EncodePreference::Software
        ));
    }
    settings.mode = ExportMode::Encode;
    settings.audio_block = 1031;
    let output = root().join("non-keyframe-cut.mp4");
    settings.overwrite = project::atomic::Overwrite::Replace;
    let receipt = export(&p, range, &settings, &output, &h).unwrap();
    assert_eq!(receipt.video_frames, 12);
    assert_eq!(receipt.audio_samples, 19200);
    let info = probe(&output);
    let duration: f64 = info["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((duration - 0.4).abs() < 1.0 / 30.0 + 0.0001);
    let external_reference = pcm(
        &input,
        Some("aresample=48000,atrim=start_sample=19200:end_sample=38400"),
    );
    let reference = mix_audio(&p, 19200, 19200, 1, &mut FileSources::default(), &h)
        .unwrap()
        .planes()[0]
        .clone();
    assert_eq!(reference.len(), external_reference.len());
    let input_mse = reference
        .iter()
        .zip(&external_reference)
        .map(|(a, b)| f64::from(*a - *b * 0.98).powi(2))
        .sum::<f64>()
        / reference.len() as f64;
    assert!(
        input_mse < 0.00001,
        "absolute source mapping MSE {input_mse}"
    );
    let actual = pcm(&output, None);
    let partitioned = root().join("non-keyframe-cut-odd.mp4");
    settings.audio_block = 257;
    export(&p, range, &settings, &partitioned, &h).unwrap();
    assert_eq!(
        actual,
        pcm(&partitioned, None),
        "encoded PCM changes with block size"
    );
    assert!(actual.len().abs_diff(reference.len()) < 1024);
    let common = actual.len().min(reference.len());
    let mut best = (f64::INFINITY, 0i32);
    for lag in -128i32..=128 {
        let mut error = 0.0;
        let mut count = 0;
        for (n, sample) in reference
            .iter()
            .enumerate()
            .take(common.saturating_sub(256))
            .skip(256)
        {
            let at = n as i64 + i64::from(lag);
            if at >= 0 && at < actual.len() as i64 {
                error += f64::from(actual[at as usize] - *sample).powi(2);
                count += 1;
            }
        }
        let mse = error / count.max(1) as f64;
        if mse < best.0 {
            best = (mse, lag);
        }
    }
    assert!(best.1.abs() < 16, "audio offset {} samples", best.1);
    assert!(best.0 < 0.0002, "PCM MSE {}", best.0);
    let raw = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&input)
        .args([
            "-vf",
            "select=eq(n\\,12)",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output()
        .unwrap();
    assert!(raw.status.success());
    let decoded = rgba(&output);
    assert_eq!(raw.stdout.len(), decoded.len());
    let mse = raw
        .stdout
        .iter()
        .zip(decoded)
        .map(|(a, b)| (f64::from(*a) - f64::from(b)).powi(2))
        .sum::<f64>()
        / raw.stdout.len() as f64;
    assert!(mse < 150.0, "first decoded frame MSE {mse}");
}
#[test]
fn resampling_split_trim_offset_and_irregular_blocks_are_identical() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut d = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    for (n, rate) in [(1, 44100), (2, 48000)] {
        let path = dir.path().join(format!("impulses-{rate}.wav"));
        let signal = format!(
            "aevalsrc=if(eq(n\\,0)+eq(n\\,{})+eq(n\\,123)+eq(n\\,24000)\\,0.5\\,0):s={rate}:d=1",
            rate / 2
        );
        generate(&path, &["-f", "lavfi", "-i", &signal, "-c:a", "pcm_f32le"]);
        d.assets
            .push(media::SourceAsset::import(media::AssetId(n), path).unwrap());
        let offset = if n == 1 { tick(103, 48000) } else { Tick::ZERO };
        let source_in = if n == 1 { tick(123, 44100) } else { Tick::ZERO };
        let end = tick(1, 1) - source_in;
        let mut items = vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(n * 10),
            source_asset: Some(media::AssetId(n)),
            source_range: TimeRange::new(source_in, tick(1, 1)).unwrap(),
            start: offset,
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: -3.0,
                muted: false,
                source: Default::default(),
            }),
        }];
        if n == 2 {
            items[0].source_range = TimeRange::new(Tick::ZERO, tick(1, 2)).unwrap();
            items.push(project::Item {
                blend_mode: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: project::ItemId(21),
                source_asset: Some(media::AssetId(2)),
                source_range: TimeRange::new(tick(1, 2), tick(1, 1)).unwrap(),
                start: tick(1, 2),
                content: project::ItemParams::Audio(project::AudioParams {
                    fades: Default::default(),
                    volume: -3.0,
                    muted: false,
                    source: Default::default(),
                }),
            });
        }
        assert!(end > Tick::ZERO);
        d.scene_mut().timeline.insert_track(project::Track {
            id: project::TrackId(n),
            name: "".into(),
            kind: project::TrackKind::Audio,
            muted: false,
            hidden: false,
            locked: false,
            items,
        });
    }
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    let mut source = FileSources::default();
    let whole = mix_audio(&p, 0, 48000, 2, &mut source, &h).unwrap();
    let mut parts = vec![Vec::new(), Vec::new()];
    let mut realtime = AudioMixer::new(&p, 2, 0, &h).unwrap();
    let pattern = [1, 7, 1031, 4093, 5, 23999];
    let mut at = 0usize;
    let mut index = 0;
    while at < 48000 {
        let n = pattern[index % pattern.len()].min(48000 - at);
        let b = realtime.mix(at as i64, n, &mut source, &h).unwrap();
        for (dst, src) in parts.iter_mut().zip(b.planes()) {
            dst.extend_from_slice(src);
        }
        at += n;
        index += 1;
    }
    assert_eq!(whole.planes(), parts);
    assert!(whole.planes()[0].iter().any(|v| *v != 0.0));
    assert_eq!(whole.planes()[0], whole.planes()[1]);
    assert!(
        whole.planes()[0][100..107].iter().any(|v| v.abs() > 0.1),
        "trim/offset impulse vanished"
    );
    assert!(
        whole.planes()[0][23998..24003]
            .iter()
            .any(|v| v.abs() > 0.1),
        "split impulse vanished"
    );
}
#[test]
fn portrait_center_and_manual_crop_and_missing_source_admission() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("color bands.mp4");
    generate(
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=red:s=192x108:r=30:d=1,drawbox=x=64:y=0:w=64:h=108:color=green:t=fill,drawbox=x=128:y=0:w=64:h=108:color=blue:t=fill",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ],
    );
    let p = video_project(&input);
    let mut s = ExportSettings::from_project(&p);
    s.render.width = 72;
    s.render.height = 128;
    s.include_audio = false;
    s.overwrite = project::atomic::Overwrite::Replace;
    let h = Hooks::default();
    let center = root().join("portrait-center.mp4");
    export(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        &s,
        &center,
        &h,
    )
    .unwrap();
    let bytes = rgba(&center);
    let pixel = &bytes[(64 * 72 + 36) * 4..][..3];
    assert!(
        pixel[1] > pixel[0] * 2 && pixel[1] > pixel[2] * 2,
        "center {pixel:?}"
    );
    s.render.placements.insert(
        project::ItemId(0),
        Placement {
            crop: Rect {
                x: 2.0 / 3.0,
                y: 0.0,
                width: 1.0 / 3.0,
                height: 1.0,
            },
            ..Default::default()
        },
    );
    let manual = root().join("portrait-manual.mp4");
    export(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        &s,
        &manual,
        &Hooks::default(),
    )
    .unwrap();
    let bytes = rgba(&manual);
    let pixel = &bytes[(64 * 72 + 36) * 4..][..3];
    assert!(
        pixel[2] > 200 && pixel[0] < 20 && pixel[1] < 20,
        "manual {pixel:?}"
    );
    let output = dir.path().join("preserve.mp4");
    std::fs::write(&output, b"previous").unwrap();
    std::fs::remove_file(&input).unwrap();
    assert!(matches!(
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &s,
            &output,
            &Hooks::default()
        ),
        Err(Error::Unavailable(_))
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"previous");
    s.allow_degraded = true;
    for accepted in [
        vec![],
        vec![Unavailable {
            asset: media::AssetId(999),
            item: project::ItemId(0),
            range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            reason: UnavailableReason::Missing,
        }],
    ] {
        s.degraded_consent = Some(accepted);
        assert!(matches!(
            export(
                &p,
                TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
                &s,
                &output,
                &Hooks::default()
            ),
            Err(Error::Unavailable(_))
        ));
        assert_eq!(std::fs::read(&output).unwrap(), b"previous");
    }
    s.degraded_consent = Some(vec![Unavailable {
        asset: media::AssetId(1),
        item: project::ItemId(0),
        range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        reason: UnavailableReason::Missing,
    }]);
    let receipt = export(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        &s,
        &output,
        &Hooks::default(),
    )
    .unwrap();
    assert!(!receipt.degraded.is_empty());
    // An accepted offline source must not hide a separate render failure on retry.
    let previous = std::fs::read(&output).unwrap();
    for cancelled in [false, true] {
        let hooks = Hooks {
            event: Some(Arc::new(move |stage| {
                if stage == Stage::Render {
                    if cancelled {
                        Err(Error::Cancelled)
                    } else {
                        Err(Error::Unsupported("injected render failure"))
                    }
                } else {
                    Ok(())
                }
            })),
            ..Default::default()
        };
        let error = export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &s,
            &output,
            &hooks,
        )
        .unwrap_err();
        assert!(if cancelled {
            matches!(error, Error::Cancelled)
        } else {
            matches!(error, Error::Unsupported(_))
        });
        assert_eq!(std::fs::read(&output).unwrap(), previous);
    }
}
#[test]
fn changed_hash_and_mid_job_disappearance_refuse_publication() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.mp4");
    av_fixture(&path);
    let p = video_project(&path);
    let output = dir.path().join("out.mp4");
    std::fs::write(&output, b"previous").unwrap();
    let mut s = ExportSettings::from_project(&p);
    s.mode = ExportMode::Encode;
    s.overwrite = project::atomic::Overwrite::Replace;
    let once = Arc::new(AtomicBool::new(false));
    let input = path.clone();
    let h = Hooks {
        event: Some(Arc::new(move |stage| {
            if stage == Stage::Render && !once.swap(true, Ordering::SeqCst) {
                std::fs::remove_file(&input)?;
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert!(matches!(
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &s,
            &output,
            &h
        ),
        Err(Error::Unavailable(_))
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"previous");
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
    av_fixture(&path);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    std::io::Write::write_all(&mut file, b"changed content").unwrap();
    assert!(
        matches!(export(&p,TimeRange::new(Tick::ZERO,tick(1,1)).unwrap(),&s,&output,&Hooks::default()),Err(Error::Unavailable(v)) if v.iter().any(|e|e.reason==UnavailableReason::Changed))
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"previous");
}
#[test]
fn hdr_passthrough_retains_tags_and_unmodified_sdr_video_avoids_encode() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let hdr = dir.path().join("hdr.mp4");
    generate(
        &hdr,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25",
            "-frames:v",
            "3",
            "-an",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p10le",
            "-x265-params",
            "log-level=error:colorprim=9:transfer=16:colormatrix=9",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
        ],
    );
    let p = video_project(&hdr);
    let range = TimeRange::new(Tick::ZERO, p.duration()).unwrap();
    let mut s = ExportSettings::from_project(&p);
    s.include_audio = false;
    let output = root().join("hdr-passthrough.mp4");
    s.overwrite = project::atomic::Overwrite::Replace;
    assert!(matches!(
        export(&p, range, &s, &output, &Hooks::default()),
        Err(Error::Unsupported(_))
    ));
    s.hdr = HdrPolicy::Passthrough;
    let result = export(&p, range, &s, &output, &Hooks::default()).unwrap();
    assert!(result.stream_copied);
    let info = probe(&output);
    assert_eq!(info["streams"][0]["color_transfer"], "smpte2084");
    assert_eq!(info["streams"][0]["pix_fmt"], "yuv420p10le");
    let input = dir.path().join("sdr.mp4");
    av_fixture(&input);
    let p = video_project(&input);
    let mut s = ExportSettings::from_project(&p);
    s.overwrite = project::atomic::Overwrite::Replace;
    let output = root().join("hybrid-copy-video-mix-audio.mp4");
    let result = export(
        &p,
        TimeRange::new(Tick::ZERO, p.duration()).unwrap(),
        &s,
        &output,
        &Hooks::default(),
    )
    .unwrap();
    assert!(result.stream_copied);
    let info = probe(&output);
    assert_eq!(info["streams"][1]["sample_rate"], "48000");
    assert_eq!(info["streams"][1]["channels"], 2);
    let a = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&input)
        .args(["-map", "0:v", "-c", "copy", "-f", "hash", "-"])
        .output()
        .unwrap();
    let b = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-map", "0:v", "-c", "copy", "-f", "hash", "-"])
        .output()
        .unwrap();
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
}
#[test]
fn missing_audio_only_is_required_only_when_audio_is_requested() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("audio.wav");
    generate(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-c:a",
            "pcm_f32le",
        ],
    );
    let asset = media::SourceAsset::import(media::AssetId(3), path.clone()).unwrap();
    let mut d = solid_project(1, 64, 64, [20, 60, 90, 255]).into_data();
    d.assets.push(asset);
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "".into(),
        kind: project::TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(3)),
            source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    let p = project::Project::new(d).unwrap();
    std::fs::remove_file(path).unwrap();
    let range = TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap();
    let output = dir.path().join("out.mp4");
    std::fs::write(&output, b"previous").unwrap();
    let mut settings = ExportSettings::from_project(&p);
    settings.overwrite = project::atomic::Overwrite::Replace;
    assert!(
        matches!(export(&p,range,&settings,&output,&Hooks::default()),Err(Error::Unavailable(v)) if v.iter().all(|u|u.asset==media::AssetId(3)))
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"previous");
    settings.include_audio = false;
    assert!(export(&p, range, &settings, &output, &Hooks::default()).is_ok());
}
#[test]
fn display_transform_is_preserved_by_passthrough_and_refused_for_new_pixels() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("anamorphic.mp4");
    generate(
        &source,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x96:rate=30:duration=1,setsar=4/3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
    );
    let p = video_project(&source);
    let range = TimeRange::new(Tick::ZERO, p.duration()).unwrap();
    let output = root().join("anamorphic-passthrough.mp4");
    let mut settings = ExportSettings::from_project(&p);
    settings.include_audio = false;
    settings.overwrite = project::atomic::Overwrite::Replace;
    assert!(
        export(&p, range, &settings, &output, &Hooks::default())
            .unwrap()
            .stream_copied
    );
    assert_eq!(probe(&output)["streams"][0]["sample_aspect_ratio"], "4:3");
    settings.mode = ExportMode::Encode;
    assert!(matches!(
        export(
            &p,
            range,
            &settings,
            &dir.path().join("refused.mp4"),
            &Hooks::default()
        ),
        Err(Error::Unsupported(_))
    ));
    let square = dir.path().join("square.mp4");
    av_fixture(&square);
    let rotated = dir.path().join("rotated.mp4");
    let args: Vec<&OsStr> = vec![
        "-display_rotation:v:0".as_ref(),
        "90".as_ref(),
        "-i".as_ref(),
        square.as_os_str(),
        "-map".as_ref(),
        "0:v".as_ref(),
        "-an".as_ref(),
        "-c".as_ref(),
        "copy".as_ref(),
        rotated.as_os_str(),
    ];
    ffmpeg(&args);
    let p = video_project(&rotated);
    let mut settings = ExportSettings::from_project(&p);
    settings.overwrite = project::atomic::Overwrite::Replace;
    settings.include_audio = false;
    let copied = root().join("rotation-passthrough.mp4");
    assert!(
        export(&p, range, &settings, &copied, &Hooks::default())
            .unwrap()
            .stream_copied
    );
    assert!(
        probe(&copied)["streams"][0]["side_data_list"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["rotation"].as_i64().is_some_and(|n| n.abs() == 90))
    );
    settings.mode = ExportMode::Encode;
    settings.include_audio = false;
    assert!(matches!(
        export(
            &p,
            range,
            &settings,
            &dir.path().join("rotation-refused.mp4"),
            &Hooks::default()
        ),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn explicitly_accepted_keyframe_cut_copies_video_and_reencodes_exact_audio_window() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("closed-gop.mp4");
    generate(
        &source,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x96:rate=30:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-g",
            "15",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-c:a",
            "aac",
            "-ac",
            "2",
            "-shortest",
        ],
    );
    let p = video_project(&source);
    let range = TimeRange::new(tick(1, 2), tick(1, 1)).unwrap();
    let mut settings = ExportSettings::from_project(&p);
    settings.mode = ExportMode::StreamCopy;
    settings.accept_keyframe_cuts = true;
    settings.overwrite = project::atomic::Overwrite::Replace;
    let output = root().join("accepted-keyframe-cut.mp4");
    let receipt = export(&p, range, &settings, &output, &Hooks::default()).unwrap();
    assert!(receipt.stream_copied);
    assert_eq!(receipt.video_frames, 15);
    assert_eq!(receipt.audio_samples, 24000);
    let mut before = media::PacketReader::open(&source).unwrap();
    let mut expected = Vec::new();
    while let Some(packet) = before.next_packet().unwrap() {
        if packet.stream() == 0 && packet.pts().unwrap().is_some_and(|t| range.contains(t)) {
            expected.push(packet.data().to_vec());
        }
    }
    let mut after = media::PacketReader::open(&output).unwrap();
    let mut copied = Vec::new();
    while let Some(packet) = after.next_packet().unwrap() {
        if packet.stream() == 0 {
            copied.push(packet.data().to_vec());
        }
    }
    assert_eq!(expected, copied);
    let original = pcm(&source, Some("atrim=start_sample=24000:end_sample=48000"));
    let result = pcm(&output, None);
    let n = result.len().min(original.len());
    let mse = result
        .iter()
        .take(n)
        .zip(&original)
        .map(|(a, b)| f64::from(a - b).powi(2))
        .sum::<f64>()
        / n as f64;
    assert!(mse < 0.0002, "key-cut PCM MSE {mse}");
    settings.accept_keyframe_cuts = false;
    assert!(matches!(
        export(
            &p,
            range,
            &settings,
            &dir.path().join("unaccepted.mp4"),
            &Hooks::default()
        ),
        Err(Error::Unsupported(_))
    ));
    settings.accept_keyframe_cuts = true;
    assert!(matches!(
        export(
            &p,
            TimeRange::new(tick(4, 10), tick(8, 10)).unwrap(),
            &settings,
            &dir.path().join("non-key.mp4"),
            &Hooks::default()
        ),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn copy_publishes_only_the_streams_selected_by_decode() {
    if !tools() {
        return;
    }
    let input = root().join("alternate-streams.mp4");
    generate(
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=red:size=64x64:rate=30:duration=1",
            "-f",
            "lavfi",
            "-i",
            "color=blue:size=64x64:rate=30:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=997:sample_rate=48000:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=443:sample_rate=48000:duration=1",
            "-map",
            "0:v",
            "-map",
            "1:v",
            "-map",
            "2:a",
            "-map",
            "3:a",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ],
    );
    let p = video_project(&input);
    let mut settings = ExportSettings::from_project(&p);
    settings.mode = ExportMode::StreamCopy;
    settings.overwrite = project::atomic::Overwrite::Replace;
    for (container, extension, video_codec, audio_codec) in [
        (media::Container::Mp4, "mp4", "h264", "aac"),
        (media::Container::Mov, "mov", "h264", "aac"),
        (media::Container::WebM, "webm", "vp9", "opus"),
    ] {
        settings.container = container;
        settings.mode = ExportMode::StreamCopy;
        let output = root().join(format!("selected-streams.{extension}"));
        let range = TimeRange::new(Tick::ZERO, p.duration()).unwrap();
        if matches!(container, media::Container::WebM) {
            std::fs::write(&output, b"previous").unwrap();
            assert!(matches!(
                export(&p, range, &settings, &output, &Hooks::default()),
                Err(Error::Unsupported(_))
            ));
            assert_eq!(std::fs::read(&output).unwrap(), b"previous");
            settings.mode = ExportMode::Auto;
        } else {
            settings.mode = ExportMode::StreamCopy;
        }
        let receipt = export(&p, range, &settings, &output, &Hooks::default()).unwrap();
        assert_eq!(
            receipt.stream_copied,
            !matches!(container, media::Container::WebM)
        );
        assert_eq!(receipt.video_frames, 30);
        let info = probe(&output);
        assert_eq!(info["streams"].as_array().unwrap().len(), 2);
        assert_eq!(info["streams"][0]["codec_name"], video_codec);
        assert_eq!(info["streams"][1]["codec_name"], audio_codec);
        assert!(
            info["format"]["format_name"]
                .as_str()
                .unwrap()
                .contains(if extension == "webm" { "webm" } else { "mov" })
        );
        let pixel = rgba(&output);
        assert!(pixel[0] > 200 && pixel[2] < 10);
        if receipt.stream_copied {
            let payloads = |path: &Path, stream: usize| {
                let mut r = media::PacketReader::open(path).unwrap();
                let mut packets = Vec::new();
                while let Some(p) = r.next_packet().unwrap() {
                    if p.stream() == stream {
                        packets.push(p.data().to_vec());
                    }
                }
                packets
            };
            assert_eq!(payloads(&input, 0), payloads(&output, 0));
            assert!(
                payloads(&input, 2) != payloads(&output, 1),
                "audio bypassed mastering"
            );
            let reference = pcm(&input, None);
            let actual = pcm(&output, None);
            let common = reference.len().min(actual.len());
            let mse = reference[..common]
                .iter()
                .zip(&actual[..common])
                .map(|(a, b)| f64::from(*a * 0.98 - *b).powi(2))
                .sum::<f64>()
                / common as f64;
            assert!(mse < 0.0002, "selected audio PCM MSE {mse}");
        }
        // The dialog always applies its quality by encoding; verify the same target with/without audio.
        settings.mode = ExportMode::Encode;
        for audio in [true, false] {
            settings.include_audio = audio;
            let encoded = root().join(format!("encoded-selected-{audio}.{extension}"));
            let receipt = export(&p, range, &settings, &encoded, &Hooks::default()).unwrap();
            assert!(!receipt.stream_copied);
            let info = probe(&encoded);
            assert_eq!(info["streams"][0]["codec_name"], video_codec);
            assert_eq!(
                info["streams"].as_array().unwrap().len(),
                if audio { 2 } else { 1 }
            );
        }
        settings.include_audio = true;
    }
}

#[test]
fn stereo_policy_keeps_headroom_and_export_limits_overload() {
    if !tools() {
        return;
    }
    let input = root().join("stereo-policy.wav");
    generate(
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.75|0.25:s=48000:d=1",
            "-c:a",
            "pcm_f32le",
        ],
    );
    let asset = media::SourceAsset::import(media::AssetId(7), input).unwrap();
    let mut d = solid_project(1, 64, 64, [0, 0, 0, 255]).into_data();
    d.assets.push(asset);
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(8),
        name: "".into(),
        kind: project::TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(7),
            source_asset: Some(media::AssetId(7)),
            source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    let p = project::Project::new(d.clone()).unwrap();
    let h = Hooks::default();
    let stereo = mix_audio(&p, 0, 32, 2, &mut FileSources::default(), &h).unwrap();
    assert_eq!(stereo.planes()[0], [0.75 * 0.98; 32]);
    assert_eq!(stereo.planes()[1], [0.25 * 0.98; 32]);
    let mono = mix_audio(&p, 0, 32, 1, &mut FileSources::default(), &h).unwrap();
    assert_eq!(mono.planes()[0], [0.5 * 0.98; 32]);
    let mut decoded = Vec::new();
    for (fade, block) in [(0, 2048), (10, 2048), (10, 137)] {
        let output = root().join(format!("cut-fade-{fade}-{block}.mp4"));
        let mut settings = ExportSettings::from_project(&p);
        settings.overwrite = project::atomic::Overwrite::Replace;
        settings.mode = ExportMode::Encode;
        settings.audio_cut_fade_ms = fade;
        settings.audio_block = block;
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &settings,
            &output,
            &h,
        )
        .unwrap();
        let pcm = tool_command("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-map", "0:a:0", "-f", "f32le", "-ac", "1", "-"])
            .output()
            .unwrap();
        assert!(pcm.status.success());
        decoded.push(
            pcm.stdout
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        decoded[1], decoded[2],
        "fade depends on audio block partition"
    );
    let rms = |samples: &[f32]| {
        (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
    };
    assert!(rms(&decoded[1][..240]) < rms(&decoded[0][..240]) * 0.7);
    // AAC encodes this DC fixture with a long transient; pin the plateau at
    // the shared graph boundary rather than assuming encoder DC preservation.
    let plain = AudioMixer::new(&p, 2, 0, &h)
        .unwrap()
        .mix(2400, 2400, &mut FileSources::default(), &h)
        .unwrap();
    let faded = AudioMixer::new(&p, 2, 10, &h)
        .unwrap()
        .mix(2400, 2400, &mut FileSources::default(), &h)
        .unwrap();
    assert_eq!(plain.planes(), faded.planes());
    // Use an oscillating overload for the encoded peak check: AAC's DC
    // reconstruction transient is not a property of the limiter's PCM ceiling.
    let overload_input = root().join("overload-sine.wav");
    generate(
        &overload_input,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.75*sin(2*PI*997*t)|0.25*sin(2*PI*997*t):s=48000:d=1",
            "-c:a",
            "pcm_f32le",
        ],
    );
    d.assets[0] = media::SourceAsset::import(media::AssetId(7), overload_input).unwrap();
    let mut extra = d.scene_mut().timeline.audio.last().unwrap().clone();
    extra.id = project::TrackId(9);
    extra.items[0].id = project::ItemId(8);
    d.scene_mut().timeline.insert_track(extra);
    let p = project::Project::new(d).unwrap();
    let mixed = mix_audio(&p, 0, 32, 2, &mut FileSources::default(), &h).unwrap();
    assert!(mixed.peak() <= 10f32.powf(-1.0 / 20.0) * 0.98);
    let output = root().join("limited-mix.mp4");
    let mut settings = ExportSettings::from_project(&p);
    settings.overwrite = project::atomic::Overwrite::Replace;
    export(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        &settings,
        &output,
        &h,
    )
    .unwrap();
    let decoded = pcm(&output, None);
    assert!(decoded.iter().all(|v| v.is_finite()));
    let reference = mix_audio(&p, 0, 48000, 2, &mut FileSources::default(), &h).unwrap();
    assert!(reference.peak() <= 10f32.powf(-1.0 / 20.0) * 0.98);
    // Lossy AAC can reconstruct peaks beyond the input sample ceiling. Compare
    // decoded PCM quality here; the limiter's hard bound is on its PCM output.
    let error = reference.planes()[0][2048..45952]
        .iter()
        .zip(&decoded[2048..45952])
        .map(|(a, b)| f64::from(*a - *b).powi(2))
        .sum::<f64>()
        / (45952 - 2048) as f64;
    assert!(error < 0.003, "overloaded export PCM MSE {error}");
}

struct FullSources(FileSources);
impl SourceProvider for FullSources {
    fn status(
        &mut self,
        a: &media::SourceAsset,
        hash: bool,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        self.0.status(a, hash, h)
    }
    fn begin_video(&mut self, active: &[project::ItemId]) {
        self.0.begin_video(active);
    }
    fn video(
        &mut self,
        a: &media::SourceAsset,
        id: project::ItemId,
        t: Tick,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        self.0.video(a, id, t, h)
    }
    fn audio(
        &mut self,
        a: &media::SourceAsset,
        id: project::ItemId,
        start: i64,
        n: usize,
        rate: u32,
        h: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        self.0.audio(a, id, start, n, rate, h)
    }
}
#[test]
fn windowed_sources_preserve_full_compositor_pixels_and_blur_reduction_fallbacks() {
    if !tools() {
        return;
    }
    let path = root().join("render-windowed-source.mp4");
    generate(
        &path,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=770x130:rate=30:duration=0.1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ],
    );
    let mut data = video_project(&path).into_data();
    data.canvas.background = project::Color([0; 4]);
    let project = project::Project::new(data).unwrap();
    let h = Hooks::default();
    for (width, height, fit, blur, crop) in [
        (113, 327, Fit::Cover, None, Rect::default()),
        (113, 327, Fit::Cover, Some(3), Rect::default()),
        (1, 513, Fit::Cover, None, Rect::default()),
        (30, 10, Fit::Cover, None, Rect::default()),
        (
            311,
            257,
            Fit::Contain,
            None,
            Rect {
                x: 0.127,
                y: 0.13,
                width: 0.191,
                height: 0.17,
            },
        ),
        (
            311,
            257,
            Fit::Stretch,
            None,
            Rect {
                x: 0.127,
                y: 0.13,
                width: 0.00091,
                height: 0.17,
            },
        ),
        (
            311,
            257,
            Fit::Cover,
            None,
            Rect {
                x: 0.127,
                y: 0.13,
                width: 0.691,
                height: 0.87,
            },
        ),
    ] {
        let mut settings = RenderSettings::from_project(&project);
        settings.width = width;
        settings.height = height;
        settings.background_blur = blur;
        settings.placements.insert(
            project::ItemId(0),
            Placement {
                destination: Rect {
                    x: -0.17,
                    y: 0.12,
                    width: 1.2,
                    height: 0.88,
                },
                crop,
                fit,
                opacity: 0.73,
            },
        );
        let mut reference = Renderer::new(&project, settings.clone()).unwrap();
        let mut actual = Renderer::new(&project, settings).unwrap();
        let mut full = FullSources(FileSources::default());
        let mut windowed = FileSources::default();
        for frame in [0, 1, 2, 0] {
            let at = Tick(time::TICKS_PER_SECOND / 30 * frame);
            let expected = reference.render_frame(at, &mut full, &h).unwrap();
            let result = actual.render_frame(at, &mut windowed, &h).unwrap();
            assert_eq!(
                result.pixels(),
                expected.pixels(),
                "{width}x{height} {fit:?} blur={blur:?}"
            );
            assert_eq!(
                actual
                    .preview_frame(at, &mut windowed, &h)
                    .unwrap()
                    .pixels(),
                expected.pixels()
            );
        }
    }
}

#[test]
fn visual_animation_disables_video_stream_copy() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("white.mp4");
    generate(
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=white:size=64x64:rate=30:duration=1.2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ],
    );
    let data = video_project(&input).into_data();
    for (path, start, end) in [
        ("opacity", 0.0, 1.0),
        ("transform.positionX", 16.0, 32.0),
        ("transform.positionY", 16.0, 32.0),
        ("transform.scaleX", 0.5, 1.0),
        ("transform.scaleY", 0.5, 1.0),
        ("transform.rotate", 45.0, 90.0),
    ] {
        let channel = project::Channel::new(
            project::ParamValue::Scalar(start),
            vec![
                project::Keyframe::new(Tick::ZERO, project::ParamValue::Scalar(start)),
                project::Keyframe::new(tick(1, 1), project::ParamValue::Scalar(end)),
            ],
            project::Extrapolation::Hold,
            project::Extrapolation::Hold,
        )
        .unwrap();
        for param in [
            project::Param::Value(channel),
            project::Param::new(project::ParamValue::Scalar(start)).unwrap(),
        ] {
            let mut candidate = data.clone();
            candidate.scene_mut().timeline.overlays[0].items[0]
                .animations
                .set(project::AnimationPath::new(path).unwrap(), param)
                .unwrap();
            let p = project::Project::new(candidate).unwrap();
            let range = TimeRange::new(Tick::ZERO, p.duration()).unwrap();
            let mut settings = ExportSettings::from_project(&p);
            settings.include_audio = false;
            let output = dir.path().join("auto.mp4");
            settings.overwrite = project::atomic::Overwrite::Replace;
            let receipt = export(&p, range, &settings, &output, &Hooks::default()).unwrap();
            assert!(
                !receipt.stream_copied,
                "Auto copied video without applying {path}"
            );
            let pixels = rgba(&output);
            assert!(
                pixels[0] <= 12,
                "{path}: first pixel must be black, got {}",
                pixels[0]
            );
            settings.mode = ExportMode::StreamCopy;
            let forced = dir.path().join("forced.mp4");
            assert!(matches!(
                export(&p, range, &settings, &forced, &Hooks::default()),
                Err(Error::Unsupported(_))
            ));
            assert!(!forced.exists());
        }
    }
}

#[test]
fn item_compositing_refuses_forced_copy_and_preserves_destination() {
    if !tools() {
        return;
    }
    let path = root().join("blur-copy-source.mp4");
    av_fixture(&path);
    let base = video_project(&path).into_data();
    for blended in [false, true] {
        let mut data = base.clone();
        if blended {
            data.scene_mut().timeline.overlays[0].items[0].blend_mode =
                project::BlendMode::Multiply;
        } else {
            data.scene_mut().timeline.overlays[0].items[0]
                .effects
                .push(project::EffectInstance {
                    id: "blur".into(),
                    enabled: true,
                    params: Default::default(),
                });
        }
        let project = project::Project::new(data).unwrap();
        let mut settings = ExportSettings::from_project(&project);
        settings.mode = ExportMode::StreamCopy;
        settings.overwrite = project::atomic::Overwrite::Replace;
        let destination = root().join("blur-copy-preserved.mp4");
        std::fs::write(&destination, b"keep old bytes").unwrap();
        assert!(matches!(
            export(
                &project,
                TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
                &settings,
                &destination,
                &Hooks::default()
            ),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(std::fs::read(&destination).unwrap(), b"keep old bytes");
    }
}

#[test]
fn export_item_mute_and_gain_change_samples() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("item-volume.mp4");
    av_fixture(&input);
    let base = video_project(&input);
    let mut data = base.into_data();
    let video_items = data.scene_mut().timeline.overlays.remove(0).items;
    data.scene_mut().timeline.main.items = video_items;
    let mut audio = data.scene().timeline.main.items[0].clone();
    audio.id = project::ItemId(1);
    if let project::ItemParams::Video(params) = &mut data.scene_mut().timeline.main.items[0].content
    {
        params.source_audio_enabled = false;
    }
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Audio".into(),
        kind: project::TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![audio],
    });
    for video_audio in [false, true] {
        for (volume, muted) in [(0.0, false), (-6.0, false), (0.0, true), (-6.0, true)] {
            if let Some(item) = data
                .scene_mut()
                .timeline
                .audio
                .first_mut()
                .and_then(|track| track.items.first_mut())
            {
                item.content = project::ItemParams::Audio(project::AudioParams {
                    fades: Default::default(),
                    volume,
                    muted,
                    source: Default::default(),
                });
            }
            data.scene_mut().timeline.audio[0].muted = video_audio;
            let project::ItemParams::Video(params) =
                &mut data.scene_mut().timeline.main.items[0].content
            else {
                panic!("video fixture")
            };
            params.source_audio_enabled = video_audio;
            params.volume = volume;
            params.muted = muted;
            let project = project::Project::new(data.clone()).unwrap();
            let output = dir
                .path()
                .join(format!("item-{video_audio}-{volume}-{muted}.mp4"));
            export(
                &project,
                TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
                &ExportSettings::from_project(&project),
                &output,
                &Hooks::default(),
            )
            .unwrap();
            let samples = pcm(&output, None);
            let peak = samples
                .iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            if muted {
                assert!(peak <= 1e-5, "muted gain={volume} peak={peak}");
            } else {
                assert!(peak > 0.05, "audible gain={volume} peak={peak}");
            }
        }
    }
}

#[test]
fn export_keeps_track_picture_and_audio_flags_independent() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("track-flags.mp4");
    av_fixture(&input);
    let base = video_project(&input);
    for (hidden, muted, audio_track) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let mut data = base.clone().into_data();
        data.scene_mut().timeline.main.items = data.scene_mut().timeline.overlays.remove(0).items;
        data.scene_mut().timeline.main.hidden = hidden;
        data.scene_mut().timeline.main.muted = muted;
        if audio_track {
            let mut audio = data.scene_mut().timeline.main.items[0].clone();
            audio.id = project::ItemId(1);
            audio.content = project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            });
            if let project::ItemParams::Video(params) =
                &mut data.scene_mut().timeline.main.items[0].content
            {
                params.source_audio_enabled = false;
            }
            data.scene_mut().timeline.insert_track(project::Track {
                id: project::TrackId(1),
                name: "Audio".into(),
                kind: project::TrackKind::Audio,
                muted,
                hidden,
                locked: false,
                items: vec![audio],
            });
        }
        let project = project::Project::new(data).unwrap();
        let hooks = Hooks::default();
        let output = dir
            .path()
            .join(format!("flags-{hidden}-{muted}-{audio_track}.mp4"));
        export(
            &project,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &ExportSettings::from_project(&project),
            &output,
            &hooks,
        )
        .unwrap();
        let bytes = rgba(&output);
        assert_eq!(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0] <= 2 && p[1] <= 2 && p[2] <= 2),
            hidden
        );
        let metadata = probe(&output);
        let has_audio = metadata["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["codec_type"] == "audio");
        if has_audio {
            let decoded = tool_command("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(&output)
                .args(["-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            let peak = decoded
                .stdout
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b).abs())
                .fold(0.0f32, f32::max);
            assert_eq!(
                peak <= 1e-5,
                muted,
                "export audio hidden={hidden} muted={muted} peak={peak}"
            );
        } else {
            assert!(muted);
        }
    }
}

#[test]
fn sound_library_attribution_and_licence_are_written_into_export_metadata() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("metadata-source.mp4");
    av_fixture(&input);
    let base = video_project(&input);
    let asset = base.data().assets[0].clone();
    let duration = asset.probe().duration.unwrap();
    let mut data = base.into_data();
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "Sound".into(),
        kind: project::TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: Some(asset.id()),
            source_range: TimeRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: project::AudioSource::Library {
                    sound: project::LibrarySound {
                        id: 42,
                        name: "Whoosh".into(),
                        author: "Freesound user".into(),
                        licence: "Attribution".into(),
                        attribution: "Whoosh by Freesound user (Attribution)".into(),
                        source_url: "https://freesound.org/s/42/".into(),
                        audio_url: "https://cdn.example/42.mp3".into(),
                        downloads: 10,
                        duration_ms: 1000,
                    },
                },
            }),
        }],
    });
    let p = project::Project::new(data).unwrap();
    let mut settings = ExportSettings::from_project(&p);
    settings.overwrite = project::atomic::Overwrite::Replace;
    let output = root().join("sound-metadata.mp4");
    export(
        &p,
        TimeRange::new(Tick::ZERO, p.duration()).unwrap(),
        &settings,
        &output,
        &Hooks::default(),
    )
    .unwrap();
    let info = probe(&output);
    let comment = info["format"]["tags"]["comment"]
        .as_str()
        .expect("licence comment tag");
    assert!(comment.contains("Whoosh by Freesound user (Attribution)"));
    assert!(comment.contains("https://freesound.org/s/42/"));
    let published = std::fs::read(&output).unwrap();
    for limit in ["bytes", "records"] {
        let mut data = p.data().clone();
        if limit == "bytes" {
            let project::ItemParams::Audio(params) =
                &mut data.scene_mut().timeline.audio[0].items[0].content
            else {
                panic!("audio item");
            };
            let project::AudioSource::Library { sound } = &mut params.source else {
                panic!("library sound");
            };
            sound.attribution = "a".repeat(4096).into();
            sound.source_url = format!("https://freesound.org/s/42/?padding={}", "x".repeat(5000));
        } else {
            let original = data.scene().timeline.audio[0].clone();
            for id in 100..164 {
                let mut track = original.clone();
                track.id = project::TrackId(id);
                track.items[0].id = project::ItemId(id);
                let project::ItemParams::Audio(params) = &mut track.items[0].content else {
                    panic!("audio item");
                };
                let project::AudioSource::Library { sound } = &mut params.source else {
                    panic!("library sound");
                };
                sound.id = id;
                sound.source_url = format!("https://freesound.org/s/{id}/");
                data.scene_mut().timeline.audio.push(track);
            }
        }
        let oversized = project::Project::new(data).unwrap();
        assert!(
            matches!(
                export(
                    &oversized,
                    TimeRange::new(Tick::ZERO, oversized.duration()).unwrap(),
                    &settings,
                    &output,
                    &Hooks::default(),
                ),
                Err(render::Error::Media(media::MediaError::TooLarge(_)))
            ),
            "{limit} must refuse rather than omit attribution"
        );
        assert_eq!(std::fs::read(&output).unwrap(), published);
    }
}

#[test]
fn clip_fades_export_matches_the_preview_mix_across_block_partitions() {
    if !tools() {
        return;
    }
    let input = root().join("fade-tone.wav");
    generate(
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.5*sin(2*PI*997*t)|0.5*sin(2*PI*997*t):s=48000:d=1",
            "-c:a",
            "pcm_f32le",
        ],
    );
    let asset = media::SourceAsset::import(media::AssetId(7), input).unwrap();
    let mut d = solid_project(1, 64, 64, [0, 0, 0, 255]).into_data();
    d.assets.push(asset);
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(8),
        name: "".into(),
        kind: project::TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(7),
            source_asset: Some(media::AssetId(7)),
            source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Audio(project::AudioParams {
                fades: project::Fades {
                    fade_in: project::Fade {
                        duration: tick(12_000, 48_000),
                        offset: Tick::ZERO,
                        curve: project::FadeCurve::EqualPower,
                    },
                    fade_out: project::Fade {
                        duration: tick(12_000, 48_000),
                        offset: Tick::ZERO,
                        curve: project::FadeCurve::Exponential,
                    },
                },
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    let mut decoded = Vec::new();
    for block in [2048, 137] {
        let output = root().join(format!("clip-fade-{block}.mp4"));
        let mut settings = ExportSettings::from_project(&p);
        settings.overwrite = project::atomic::Overwrite::Replace;
        settings.mode = ExportMode::Encode;
        settings.audio_block = block;
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &settings,
            &output,
            &h,
        )
        .unwrap();
        let pcm = tool_command("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-map", "0:a:0", "-f", "f32le", "-ac", "2", "-"])
            .output()
            .unwrap();
        assert!(pcm.status.success());
        decoded.push(
            pcm.stdout
                .as_chunks::<4>()
                .0
                .iter()
                .step_by(2)
                .map(|b| f32::from_le_bytes(*b))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(decoded[0], decoded[1], "export depends on audio block size");
    // The preview mixer and the encoded export share one envelope (AAC tolerance).
    let preview = AudioMixer::new(&p, 2, 0, &h)
        .unwrap()
        .mix(0, 48_000, &mut FileSources::default(), &h)
        .unwrap();
    let rms = |samples: &[f32], centre: usize| {
        let w = &samples[centre - 240..centre + 240];
        (w.iter().map(|x| x * x).sum::<f32>() / w.len() as f32).sqrt()
    };
    for centre in [
        300, 3_000, 6_000, 9_000, 24_000, 39_000, 42_000, 45_000, 47_700,
    ] {
        let want = rms(&preview.planes()[0], centre);
        let got = rms(&decoded[0], centre);
        assert!(
            (want - got).abs() < 0.05,
            "{centre}: preview {want} export {got}"
        );
    }
}
