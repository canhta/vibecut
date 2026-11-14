#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Flicker removal: the gain derived from the stored brightness, its recorded oracles on the
//! footage clean-up study's fixtures (modelled as the measured series), its encoding, and the
//! picture render draws from it.
use crate::support::{Hooks, tick};
use render::flicker::{FlickerError, FlickerTrack, GAIN_RANGE};
use render::{
    AudioBlock, ExecutionHooks, Image, MemoryKind, RenderSettings, Renderer, Result,
    SourceProvider, UnavailableReason,
};
use std::sync::Arc;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

const FPS: i64 = 30;
/// The window in frames at 30 fps: one second, made odd.
const WINDOW: usize = 31;

/// Bounds the footage clean-up study (`210-footage-cleanup.md`, Remove flicker) recorded for a
/// 1 s Hann window on its fixtures: interior RMS and maximum of `ln(corrected / truth)` after
/// removing the constant, in percent, excluding one window at each end.
const FLICKER_RMS: f64 = 0.6;
const FLICKER_MAX: f64 = 2.0;
/// A real fade must not be flattened beyond this RMS.
const FADE_RMS: f64 = 0.6;
/// A hard cut with flicker, with the cut reset.
const CUT_RMS: f64 = 0.5;

fn times(count: usize) -> Vec<Tick> {
    (0..count as i64)
        .map(|n| Tick(n * TICKS_PER_SECOND / FPS))
        .collect()
}
fn seconds(count: usize) -> Vec<f64> {
    (0..count).map(|n| n as f64 / FPS as f64).collect()
}
/// `1 + 0.08 sin(2 pi f t)`: the study's flicker, 8 % deep.
fn flicker(clean: &[f64], hertz: f64) -> Vec<f64> {
    clean
        .iter()
        .zip(seconds(clean.len()))
        .map(|(value, t)| value * (1.0 + 0.08 * (2.0 * std::f64::consts::PI * hertz * t).sin()))
        .collect()
}
fn track(means: &[f64], cuts: &[usize]) -> FlickerTrack {
    FlickerTrack::new(
        times(means.len()),
        means.iter().map(|m| *m as f32).collect(),
        (0..means.len()).map(|n| cuts.contains(&n)).collect(),
    )
    .unwrap()
}
/// Interior RMS and maximum of `ln(observed * gain / truth)` in percent, constant removed,
/// excluding one window at each end.
fn residual(track: &FlickerTrack, observed: &[f64], truth: &[f64]) -> (f64, f64) {
    let errors: Vec<f64> = observed
        .iter()
        .zip(truth)
        .enumerate()
        .map(|(n, (o, t))| (o * f64::from(track.gain(n)) / t).ln())
        .collect();
    let interior = &errors[WINDOW..errors.len() - WINDOW];
    let mean = interior.iter().sum::<f64>() / interior.len() as f64;
    let rms =
        (interior.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / interior.len() as f64).sqrt();
    let max = interior
        .iter()
        .map(|e| (e - mean).abs())
        .fold(0.0, f64::max);
    (100.0 * rms, 100.0 * max)
}
/// A shot whose brightness drifts slowly (17 %/s at its steepest), 10 s at 30 fps.
fn drifting() -> Vec<f64> {
    seconds(300)
        .into_iter()
        .map(|t| 0.35 + 0.05 * (2.0 * std::f64::consts::PI * t / 5.3).sin())
        .collect()
}

#[test]
fn mains_flicker_falls_below_the_recorded_bound_at_both_the_fast_alias_and_the_slow_beat() {
    let clean = drifting();
    let uncorrected =
        |observed: &[f64]| residual(&track(&vec![0.35; observed.len()], &[]), observed, &clean).0;
    // 100 Hz mains aliased by 30 fps to a 2.97-frame period, and the 17.6-frame beat.
    for hertz in [10.09, 1.7] {
        let observed = flicker(&clean, hertz);
        let gains = track(&observed, &[]);
        let (rms, max) = residual(&gains, &observed, &clean);
        println!(
            "FLICKER {hertz} Hz: rms {rms:.3} % max {max:.3} % (uncorrected {:.2} %)",
            uncorrected(&observed)
        );
        assert!(rms <= FLICKER_RMS, "{hertz} Hz rms {rms}");
        assert!(max <= FLICKER_MAX, "{hertz} Hz max {max}");
        // The frame-to-frame luma variation falls well under what it was.
        assert!(rms * 10.0 < uncorrected(&observed), "{hertz} Hz");
    }
}

#[test]
fn a_real_fade_is_not_flattened_beyond_the_recorded_tolerance() {
    // The study's fade: a 70 % brightness change over the clip, with the 10.09 Hz flicker on top.
    let clean: Vec<f64> = drifting()
        .iter()
        .zip(seconds(300))
        .map(|(value, t)| value * (1.0 - 0.06 * t))
        .collect();
    let observed = flicker(&clean, 10.09);
    let gains = track(&observed, &[]);
    let (rms, max) = residual(&gains, &observed, &clean);
    println!("FLICKER fade: rms {rms:.3} % max {max:.3} %");
    assert!(rms <= FADE_RMS, "fade rms {rms}");
    // The fade itself survives: corrected frames still fall with the truth.
    let first = observed[40] * f64::from(gains.gain(40));
    let last = observed[260] * f64::from(gains.gain(260));
    assert!(last < 0.7 * first, "{first} -> {last}");
}

#[test]
fn a_cut_resets_the_window_so_flicker_does_not_pump_across_it() {
    // Two shots of different brightness, each drifting 7 %/s at most, flickering at 10.09 Hz.
    let clean: Vec<f64> = seconds(300)
        .into_iter()
        .enumerate()
        .map(|(n, t)| {
            let level = if n < 150 { 0.30 } else { 0.55 };
            level + 0.02 * (2.0 * std::f64::consts::PI * t / 6.0).sin()
        })
        .collect();
    let observed = flicker(&clean, 10.09);
    let (rms, max) = residual(&track(&observed, &[150]), &observed, &clean);
    println!("FLICKER cut: rms {rms:.3} % max {max:.3} %");
    assert!(rms <= CUT_RMS, "cut rms {rms}");
    // Without the reset the reference averages two shots and pumps for half a window each side.
    let (unreset, worst) = residual(&track(&observed, &[]), &observed, &clean);
    assert!(unreset > 5.0 * CUT_RMS && worst > 10.0, "{unreset} {worst}");
}

#[test]
fn a_constant_series_and_a_dark_frame_take_no_gain_and_the_gain_is_bounded() {
    let flat = track(&[0.4; 90], &[]);
    assert!((0..90).all(|n| (flat.gain(n) - 1.0).abs() < 1e-6));
    // A black frame has nothing to scale.
    let mut means = vec![0.4; 90];
    means[45] = 0.0;
    assert_eq!(track(&means, &[]).gain(45), 1.0);
    // A flash far above its neighbours is clamped, never driven to zero.
    let mut means = vec![0.2; 90];
    means[44..=46].fill(1.0);
    let flash = track(&means, &[]);
    assert!((0..90).all(|n| (GAIN_RANGE[0]..=GAIN_RANGE[1]).contains(&flash.gain(n))));
    assert_eq!(flash.gain(45), GAIN_RANGE[0]);
}

#[test]
fn the_window_follows_the_frame_rate_and_a_lookup_is_by_source_time() {
    // At 60 fps the window is 61 frames, so a 1.7 Hz beat is still removed.
    let count = 600;
    let clean: Vec<f64> = vec![0.4; count];
    let observed: Vec<f64> = clean
        .iter()
        .enumerate()
        .map(|(n, v)| v * (1.0 + 0.08 * (2.0 * std::f64::consts::PI * 1.7 * n as f64 / 60.0).sin()))
        .collect();
    let gains = FlickerTrack::new(
        (0..count as i64)
            .map(|n| Tick(n * TICKS_PER_SECOND / 60))
            .collect(),
        observed.iter().map(|m| *m as f32).collect(),
        vec![false; count],
    )
    .unwrap();
    let errors: Vec<f64> = (61..count - 61)
        .map(|n| 100.0 * (observed[n] * f64::from(gains.gain(n)) / 0.4).ln())
        .collect();
    assert!(errors.iter().all(|e| e.abs() < 1.0), "{errors:?}");
    // A time inside a frame takes that frame's gain; before the first, the first's.
    let at = Tick(10 * TICKS_PER_SECOND / 60 + 7);
    assert_eq!(gains.gain_at(at), gains.gain(10));
    assert_eq!(gains.gain_at(Tick(-5)), gains.gain(0));
    assert_eq!(gains.gain_at(Tick(i64::MAX / 2)), gains.gain(count - 1));
}

#[test]
fn a_track_encodes_to_the_same_bytes_reads_back_and_refuses_what_new_refuses() {
    let means: Vec<f64> = (0..40).map(|n| 0.2 + 0.01 * f64::from(n % 7)).collect();
    let original = track(&means, &[20]);
    let bytes = original.encode();
    assert_eq!(bytes, original.encode());
    assert_eq!(FlickerTrack::decode(&bytes).unwrap(), original);
    assert!(original.is_cut(20) && !original.is_cut(0) && !original.is_cut(21));
    assert_eq!(bytes.len(), 12 + 13 * 40);
    // Format, length, order, value and flag refusals.
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert_eq!(FlickerTrack::decode(&bad), Err(FlickerError("format")));
    assert_eq!(
        FlickerTrack::decode(&bytes[..bytes.len() - 1]),
        Err(FlickerError("length"))
    );
    let mut order = bytes.clone();
    order[12..20].copy_from_slice(&i64::MAX.to_le_bytes());
    assert_eq!(
        FlickerTrack::decode(&order),
        Err(FlickerError("frame order"))
    );
    let mut nan = bytes.clone();
    nan[12 + 8 * 40..12 + 8 * 40 + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(FlickerTrack::decode(&nan), Err(FlickerError("mean")));
    let mut flag = bytes.clone();
    *flag.last_mut().unwrap() = 2;
    assert_eq!(FlickerTrack::decode(&flag), Err(FlickerError("cut flag")));
    let mut first = bytes.clone();
    first[12 + 12 * 40] = 1;
    assert_eq!(FlickerTrack::decode(&first), Err(FlickerError("cut flag")));
    assert!(FlickerTrack::new(vec![], vec![], vec![]).is_err());
    assert!(FlickerTrack::new(vec![Tick::ZERO], vec![0.5, 0.5], vec![false]).is_err());
}

/// A source of `SIZE` square frames whose picture is an opaque diagonal ramp.
const SIZE: u32 = 64;
struct Ramp;
impl SourceProvider for Ramp {
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
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        let bytes: Vec<u8> = (0..SIZE * SIZE)
            .flat_map(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let v = (40 + x + y) as u8;
                [v, 255 - v, v / 2, 255]
            })
            .collect();
        Ok(Some(Image::from_rgba8(
            SIZE, SIZE, &bytes, false, 13, hooks,
        )?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, hooks)
    }
}
fn scene(remove_flicker: bool) -> project::Project {
    let asset = testkit::source::asset(1, std::env::temp_dir().join("render-flicker"))
        .format("synthetic")
        .duration(tick(2, 1))
        .stream(
            testkit::source::video()
                .codec("rgba")
                .time_base(media::TimeBase::new(1, 30).unwrap())
                .duration(tick(2, 1))
                .start(Tick::ZERO)
                .frame_count(60)
                .variable_frame_rate(false)
                .size(SIZE, SIZE)
                .build(),
        )
        .build();
    let mut data = project::Project::empty(project::ProjectId(7), "flicker".into())
        .unwrap()
        .into_data();
    data.canvas.width = SIZE;
    data.canvas.height = SIZE;
    data.assets.push(asset);
    let mut item = testkit::item::video(project::ItemId(1))
        .asset(media::AssetId(1))
        .source(SourceRange::new(Tick::ZERO, tick(2, 1)).unwrap())
        .build();
    let project::ItemParams::Video(params) = &mut item.content else {
        panic!("video item");
    };
    params.remove_flicker = remove_flicker;
    data.scene_mut().timeline.main.items.push(item);
    project::Project::new(data).unwrap()
}
/// 60 frames at a steady 0.3 with one flash at frame 3.
fn flash() -> Arc<FlickerTrack> {
    let mut means = vec![0.3; 60];
    means[3] = 0.36;
    Arc::new(track(&means, &[]))
}

#[test]
fn a_flicker_item_renders_the_stored_gain_the_same_in_preview_and_export_and_a_missing_track_renders_uncorrected()
 {
    let hooks = Hooks::default();
    let at = Tick(3 * TICKS_PER_SECOND / FPS);
    let render = |project: &project::Project, stored: Option<Arc<FlickerTrack>>, preview: bool| {
        let mut settings = RenderSettings::from_project(project);
        if let Some(stored) = stored {
            settings.flicker.insert(project::ItemId(1), stored);
        }
        let mut renderer = Renderer::new(project, settings).unwrap();
        if preview {
            renderer.preview_frame(at, &mut Ramp, &hooks).unwrap()
        } else {
            renderer.render_frame(at, &mut Ramp, &hooks).unwrap()
        }
    };
    let plain = render(&scene(false), None, false);
    // Switched on with no stored track: never an error, and the picture is the uncorrected one.
    assert_eq!(render(&scene(true), None, false).pixels(), plain.pixels());
    // A track for an item whose switch is off changes nothing.
    assert_eq!(
        render(&scene(false), Some(flash()), false).pixels(),
        plain.pixels()
    );
    // Switched on with a track: every colour channel is scaled by the frame's gain, alpha is
    // kept, and preview equals export.
    let gain = flash().gain(3);
    assert!((0.5..0.9).contains(&gain), "{gain}");
    let export = render(&scene(true), Some(flash()), false);
    let preview = render(&scene(true), Some(flash()), true);
    assert_eq!(export.pixels(), preview.pixels());
    assert_ne!(export.pixels(), plain.pixels());
    for (corrected, original) in export.pixels().iter().zip(plain.pixels()) {
        for channel in 0..3 {
            let expected = (original[channel] * gain).min(1.0);
            assert!(
                (corrected[channel] - expected).abs() < 1e-5,
                "{corrected:?} vs {original:?} x {gain}"
            );
        }
        assert_eq!(corrected[3], original[3]);
    }
    // Two renders from the same inputs are byte-equal.
    assert_eq!(
        render(&scene(true), Some(flash()), false).pixels(),
        export.pixels()
    );
    // A frame whose gain is the identity renders as the uncorrected picture.
    let steady = Arc::new(track(&[0.3; 60], &[]));
    let identity = render(&scene(true), Some(steady), false);
    for (corrected, original) in identity.pixels().iter().zip(plain.pixels()) {
        for channel in 0..4 {
            assert!((corrected[channel] - original[channel]).abs() < 1e-5);
        }
    }
}
