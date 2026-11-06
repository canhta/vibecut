#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use crate::support::*;
use render::*;
use time::{Speed, Tick, TimeRange};

const RATE: i64 = 48000;
const TONE: f64 = 1000.0;
const LEVEL: f32 = 0.5;

/// A stereo sine at an absolute source sample index; records every request.
struct Tone {
    hertz: f64,
    sweep: f64,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
    requests: Vec<(i64, usize)>,
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
        self.requests.push((start, frames));
        let mut out = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                let t = (start + n as i64) as f64 / f64::from(rate);
                *sample = LEVEL
                    * (std::f64::consts::TAU * (self.hertz * t + 0.5 * self.sweep * t * t)).sin()
                        as f32;
            }
        }
        if let Some(cancel) = &self.cancel {
            cancel.store(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(out)
    }
}

/// One audio item that plays `output_seconds` of a long tone at `thousandths` speed.
fn fixture(thousandths: u16, maintain_pitch: bool, output_seconds: f64) -> project::Project {
    let speed = Speed::from_thousandths(thousandths).unwrap();
    let source_ticks = (output_seconds * speed.as_f64() * time::TICKS_PER_SECOND as f64) as i64;
    let long = tick(60 * RATE, RATE);
    let mut data = project::Project::empty(project::ProjectId(1), "".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("retime.wav"))
            .format("wav")
            .duration(long)
            .content_hash(media::ContentHash::from_bytes(&[7]))
            .stream(
                testkit::source::audio()
                    .codec("pcm_f32le")
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(long)
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![project::Item {
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, Tick(source_ticks)).unwrap(),
            start: Tick::ZERO,
            retime: project::Retime {
                window: None,
                speed,
                maintain_pitch,
            },
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    project::Project::new(data).unwrap()
}

fn render(p: &project::Project, source: &mut Tone, blocks: &[usize]) -> Vec<f32> {
    let h = Hooks::default();
    let total = p
        .duration()
        .to_units(RATE, time::Rounding::Nearest)
        .unwrap() as usize;
    let mut mixer = AudioMixer::new(p, 2, 0, &h).unwrap();
    let mut out = Vec::with_capacity(total);
    let mut at = 0;
    let mut index = 0;
    while at < total {
        let size = blocks[index % blocks.len()].min(total - at);
        let block = mixer.mix(at as i64, size, source, &h).unwrap();
        out.extend_from_slice(&block.planes()[0]);
        at += size;
        index += 1;
    }
    out
}

/// Radix-2 complex FFT; `n` is a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Frequency of the strongest bin of a Hann-windowed 16384-sample span, and
/// that peak's magnitude relative to all other energy (dB).
fn peak(samples: &[f32]) -> (f64, f64) {
    let n = 16384;
    let mut re: Vec<f64> = samples
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, v)| {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            f64::from(*v) * w
        })
        .collect();
    assert_eq!(re.len(), n);
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    let power: Vec<f64> = (0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect();
    let (bin, _) =
        power.iter().enumerate().skip(1).fold(
            (0, 0.0),
            |best, (k, p)| if *p > best.1 { (k, *p) } else { best },
        );
    let near: f64 = power[bin.saturating_sub(4)..(bin + 5).min(n / 2)]
        .iter()
        .sum();
    let rest: f64 = power.iter().sum::<f64>() - near;
    (
        bin as f64 * RATE as f64 / n as f64,
        10.0 * (near / rest.max(1e-30)).log10(),
    )
}

#[test]
fn pitch_follows_speed_or_is_preserved_by_the_time_stretch() {
    // (speed in thousandths, maintain_pitch, expected output Hz)
    for (speed, maintain, expected) in [
        (500, false, 500.0),
        (500, true, 1000.0),
        (1500, false, 1500.0),
        (1500, true, 1000.0),
        (2000, false, 2000.0),
        (2000, true, 1000.0),
        (5000, false, 5000.0),
        (5000, true, 1000.0),
    ] {
        let p = fixture(speed, maintain, 1.0);
        let mut source = Tone {
            hertz: TONE,
            sweep: 0.0,
            cancel: None,
            requests: Vec::new(),
        };
        let out = render(&p, &mut source, &[65536]);
        assert_eq!(out.len(), RATE as usize);
        let (hertz, clean) = peak(&out[8192..8192 + 16384]);
        assert!(
            (hertz - expected).abs() <= 6.0,
            "speed {speed} maintain {maintain}: peak {hertz} Hz, expected {expected}"
        );
        assert!(
            clean > 25.0,
            "speed {speed} maintain {maintain}: tone only {clean:.1} dB above the rest"
        );
        let level = out[8192..24576].iter().map(|v| v.abs()).fold(0.0, f32::max);
        assert!(
            (0.40..=0.50).contains(&level),
            "speed {speed} maintain {maintain}: level {level}"
        );
    }
}

#[test]
fn block_boundaries_do_not_change_or_click_the_retimed_audio() {
    for (speed, maintain) in [
        (500, true),
        (2000, true),
        (5000, true),
        (500, false),
        (2500, false),
    ] {
        let p = fixture(speed, maintain, 1.0);
        let whole = render(
            &p,
            &mut Tone {
                hertz: TONE,
                sweep: 0.0,
                cancel: None,
                requests: Vec::new(),
            },
            &[65536],
        );
        let mut cut = Tone {
            hertz: TONE,
            sweep: 0.0,
            cancel: None,
            requests: Vec::new(),
        };
        let pieces = render(&p, &mut cut, &[1, 960, 13, 4096, 1023, 7777]);
        assert_eq!(
            whole, pieces,
            "speed {speed} maintain {maintain}: block partition changed the audio"
        );
        // A pure tone cannot step faster than its own slope; a click would.
        let hertz = if maintain {
            TONE
        } else {
            TONE * f64::from(speed) / 1000.0
        };
        let slope = f64::from(LEVEL) * std::f64::consts::TAU * hertz / RATE as f64;
        let worst = whole
            .windows(2)
            .skip(512)
            .take(whole.len() - 1024)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .fold(0.0, f64::max);
        assert!(
            worst <= slope * 1.5 + 1e-3,
            "speed {speed} maintain {maintain}: step {worst:.5} vs tone slope {slope:.5}"
        );
        // Source reads only move forward, so a file cursor never restarts.
        let mut next = 0;
        for (start, frames) in &cut.requests {
            assert!(
                *start >= next,
                "speed {speed} maintain {maintain}: read went back to {start} < {next}"
            );
            next = start + *frames as i64;
        }
    }
}

#[test]
fn a_seek_replays_to_the_same_audio_and_item_edges_keep_unity_gain() {
    let p = fixture(2000, true, 1.0);
    let whole = render(
        &p,
        &mut Tone {
            hertz: TONE,
            sweep: 0.0,
            cancel: None,
            requests: Vec::new(),
        },
        &[65536],
    );
    let h = Hooks::default();
    let mut mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
    let mut source = Tone {
        hertz: TONE,
        sweep: 0.0,
        cancel: None,
        requests: Vec::new(),
    };
    mixer.mix(30000, 100, &mut source, &h).unwrap();
    let back = mixer.mix(11111, 2000, &mut source, &h).unwrap();
    assert_eq!(back.planes()[0], whole[11111..13111]);
    // The first and last 5 ms of the clip are neither faded nor silent.
    let head = whole[..240].iter().map(|v| v.abs()).fold(0.0, f32::max);
    let tail = whole[whole.len() - 240..]
        .iter()
        .map(|v| v.abs())
        .fold(0.0, f32::max);
    assert!(head > 0.3 && tail > 0.3, "head {head} tail {tail}");
}

#[test]
fn retime_chirp_preserves_local_pitch_and_exact_duration() {
    for (speed, keep) in [(500, true), (2000, true), (5000, true), (2000, false)] {
        let p = fixture(speed, keep, 2.0);
        let mut source = Tone {
            hertz: 300.0,
            sweep: 150.0,
            cancel: None,
            requests: vec![],
        };
        let out = render(&p, &mut source, &[4096]);
        assert_eq!(out.len(), 2 * RATE as usize);
        for t in [0.5, 1.5] {
            let start = ((t - 0.05) * RATE as f64) as usize;
            let end = start + RATE as usize / 10;
            let crossings = out[start..end]
                .windows(2)
                .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                .count();
            let measured = crossings as f64 * 10.0;
            let rate = f64::from(speed) / 1000.0;
            let expected = (300.0 + 150.0 * rate * t) * if keep { 1.0 } else { rate };
            assert!(
                (measured - expected).abs() <= 30.0,
                "speed {speed}, keep {keep}, t {t}: measured {measured}, expected {expected}"
            );
        }
    }
}

#[test]
fn retime_stretch_cancels_after_source_pull_and_recovers_for_retry() {
    let p = fixture(500, true, 2.0);
    let h = Hooks::default();
    let mut mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
    let mut source = Tone {
        hertz: TONE,
        sweep: 0.0,
        cancel: Some(h.state.clone()),
        requests: vec![],
    };
    assert!(matches!(
        mixer.mix(0, 48000, &mut source, &h),
        Err(Error::Cancelled)
    ));
    assert_eq!(source.requests.len(), 1);
    source.cancel = None;
    h.state.store(0, std::sync::atomic::Ordering::SeqCst);
    let retried = mixer.mix(0, 1024, &mut source, &h).unwrap();
    let reference = render(&p, &mut source, &[4096]);
    assert_eq!(retried.planes()[0], reference[..1024]);
}

#[test]
fn retime_large_follow_block_respects_working_set_budget() {
    // At 5x, one maximum block retains 327,680 source frames in stereo plus
    // a mono search plane, before its output planes: more than this 4 MiB budget.
    let p = fixture(5000, false, 2.0);
    let h = Hooks {
        limit: 4 << 20,
        ..Hooks::default()
    };
    let mut mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
    let mut source = Tone {
        hertz: TONE,
        sweep: 0.0,
        cancel: None,
        requests: vec![],
    };
    assert!(matches!(
        mixer.mix(0, 65536, &mut source, &h),
        Err(Error::Budget { .. })
    ));
    drop(mixer);
    assert_eq!(h.used.load(std::sync::atomic::Ordering::SeqCst), 0);
}
