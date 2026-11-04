use crate::{Error, ExecutionHooks, Stage, contracts::checkpoint};
use rustfft::{FftPlanner, num_complex::Complex};
const FFT: usize = 400;
const BINS: usize = 201;
const HOP: usize = 160;
const FRAMES: usize = 3000;
fn hz_to_mel(f: f64) -> f64 {
    if f >= 1000. {
        15. + (f / 1000.).ln() / ((6.4f64).ln() / 27.)
    } else {
        f / (200. / 3.)
    }
}
fn mel_to_hz(m: f64) -> f64 {
    if m >= 15. {
        1000. * (((6.4f64).ln() / 27.) * (m - 15.)).exp()
    } else {
        m * (200. / 3.)
    }
}
/// Periodic Hann, centered reflect padding, Slaney filters and 30s zero padding.
/// Centering contributes no source-coordinate delay; input PCM is already resampled.
pub(super) fn log_mel(
    audio: &[f32],
    bands: usize,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<f32>, Error> {
    if audio.len() < FFT / 2 + 1
        || audio.len() > 480000
        || ![80, 128].contains(&bands)
        || audio.iter().any(|v| !v.is_finite())
    {
        return Err(Error::Invalid("alignment PCM"));
    }
    let mut padded = vec![0.; 480000];
    padded
        .get_mut(..audio.len())
        .ok_or(Error::Payload)?
        .copy_from_slice(audio);
    let melpoints: Vec<_> = (0..bands + 2)
        .map(|i| mel_to_hz(hz_to_mel(8000.) * i as f64 / (bands + 1) as f64))
        .collect();
    let mut filters = vec![0.; bands * BINS];
    for m in 0..bands {
        let a = *melpoints.get(m).ok_or(Error::Inference)?;
        let b = *melpoints.get(m + 1).ok_or(Error::Inference)?;
        let c = *melpoints.get(m + 2).ok_or(Error::Inference)?;
        for k in 0..BINS {
            let f = k as f64 * 16000. / FFT as f64;
            *filters.get_mut(m * BINS + k).ok_or(Error::Inference)? =
                (((f - a) / (b - a)).min((c - f) / (c - b)).max(0.) * 2. / (c - a)) as f32;
        }
    }
    let hann: Vec<_> = (0..FFT)
        .map(|i| (0.5 - 0.5 * (2. * std::f64::consts::PI * i as f64 / FFT as f64).cos()) as f32)
        .collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT);
    let mut buffer = vec![Complex::new(0., 0.); FFT];
    let mut scratch = vec![Complex::new(0., 0.); fft.get_inplace_scratch_len()];
    let mut out = vec![0.; bands * FRAMES];
    for frame in 0..FRAMES {
        if frame % 32 == 0 {
            checkpoint(hooks, Stage::Align)?;
        }
        for (i, v) in buffer.iter_mut().enumerate() {
            let at = super::path::reflect(
                (frame * HOP + i) as isize - (FFT / 2) as isize,
                padded.len(),
            );
            *v = Complex::new(
                padded.get(at).ok_or(Error::Inference)? * hann.get(i).ok_or(Error::Inference)?,
                0.,
            );
        }
        fft.process_with_scratch(&mut buffer, &mut scratch);
        let power: Vec<_> = buffer.iter().take(BINS).map(|v| v.norm_sqr()).collect();
        for (m, row) in filters.as_chunks::<BINS>().0.iter().enumerate() {
            let energy = row.iter().zip(&power).map(|(a, b)| a * b).sum::<f32>();
            if !energy.is_finite() {
                return Err(Error::Invalid("alignment PCM energy"));
            }
            *out.get_mut(m * FRAMES + frame).ok_or(Error::Inference)? = energy.max(1e-10).log10();
        }
    }
    let max = out.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    for v in &mut out {
        *v = (v.max(max - 8.) + 4.) / 4.;
    }
    Ok(out)
}
