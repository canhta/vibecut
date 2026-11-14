//! The notification sound a finished export plays when the Edit settings ask for it.
//! Two short synthesized tones on the default output device; no sound asset is shipped.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Pitch of the two tones, Hz, played one after the other.
const TONES_HZ: [f32; 2] = [880.0, 1320.0];
/// Length of each tone in seconds.
const TONE_SECONDS: f32 = 0.12;
/// Peak level, well below full scale.
const PEAK: f32 = 0.2;

/// The chime as mono samples at `rate`: each tone decays linearly to silence, so the
/// sound ends without a click.
fn samples(rate: u32) -> Vec<f32> {
    let per_tone = (rate as f32 * TONE_SECONDS) as usize;
    let mut out = Vec::with_capacity(per_tone * TONES_HZ.len());
    for hz in TONES_HZ {
        for n in 0..per_tone {
            let t = n as f32 / rate as f32;
            let fade = 1.0 - n as f32 / per_tone as f32;
            out.push(PEAK * fade * (std::f32::consts::TAU * hz * t).sin());
        }
    }
    out
}

/// Play the chime on a detached thread. A host with no usable output device stays silent.
pub(in crate::editor) fn play() {
    std::thread::spawn(|| {
        let _ = play_blocking();
    });
}

fn play_blocking() -> Option<()> {
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return None;
    }
    let channels = usize::from(config.channels());
    let tune = Arc::new(samples(config.sample_rate()));
    let position = Arc::new(AtomicUsize::new(0));
    let length = tune.len();
    let (reader, cursor) = (tune.clone(), position.clone());
    let stream = device
        .build_output_stream(
            config.config(),
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let at = cursor.fetch_add(1, Ordering::Relaxed);
                    let value = reader.get(at).copied().unwrap_or(0.0);
                    frame.fill(value);
                }
            },
            |_| {},
            Some(Duration::from_secs(1)),
        )
        .ok()?;
    stream.play().ok()?;
    let seconds = length as f32 / config.sample_rate() as f32;
    std::thread::sleep(Duration::from_secs_f32(seconds + 0.1));
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chime_is_two_quiet_tones_that_end_in_silence() {
        let rate = 48_000;
        let tune = samples(rate);
        assert_eq!(tune.len(), 2 * (rate as f32 * TONE_SECONDS) as usize);
        assert!(tune.iter().all(|sample| sample.abs() <= PEAK));
        assert!(tune.iter().any(|sample| sample.abs() > PEAK / 2.0));
        assert!(tune.last().is_some_and(|sample| sample.abs() < 0.001));
    }
}
