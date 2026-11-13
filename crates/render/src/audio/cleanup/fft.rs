//! In-place radix-2 complex FFT over one planned power-of-two size.
//!
//! The iterative decimation-in-time butterfly is the textbook Cooley-Tukey algorithm
//! (Cooley and Tukey, "An algorithm for the machine calculation of complex Fourier series",
//! Mathematics of Computation 19, 1965). The twiddle layout (one contiguous run per stage) is
//! adapted from the structure reference's `audio-dsp` FFT.

use std::f64::consts::PI;

/// A planned transform of `size` complex points, `size` a power of two of at least 2.
#[derive(Clone)]
pub(super) struct Fft {
    size: usize,
    /// The twiddles of the stage with half-length `h` are the run `h - 1..2 * h - 1`.
    twiddles: Vec<(f32, f32)>,
    reversed: Vec<u32>,
}

impl Fft {
    pub(super) fn new(size: usize) -> Self {
        let bits = size.trailing_zeros();
        let reversed = (0..size as u32)
            .map(|index| index.reverse_bits().checked_shr(32 - bits).unwrap_or(0))
            .collect();
        let mut twiddles = Vec::with_capacity(size);
        let mut half = 1usize;
        while half < size {
            for k in 0..half {
                let angle = -PI * k as f64 / half as f64;
                twiddles.push((angle.cos() as f32, angle.sin() as f32));
            }
            half <<= 1;
        }
        Self {
            size,
            twiddles,
            reversed,
        }
    }

    /// Transforms `re` and `im` in place, unscaled. The inverse uses the conjugate twiddles, so
    /// a forward then an inverse pass multiplies the input by `size`. Shorter slices are left
    /// untouched.
    pub(super) fn transform(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.size;
        let (Some(re), Some(im)) = (re.get_mut(..n), im.get_mut(..n)) else {
            return;
        };
        for (index, &partner) in self.reversed.iter().enumerate() {
            let partner = partner as usize;
            if partner > index {
                re.swap(index, partner);
                im.swap(index, partner);
            }
        }
        let sign = if inverse { -1.0f32 } else { 1.0 };
        let mut half = 1usize;
        while half < n {
            let Some(stage) = self.twiddles.get(half - 1..2 * half - 1) else {
                return;
            };
            for (r, m) in re
                .chunks_exact_mut(2 * half)
                .zip(im.chunks_exact_mut(2 * half))
            {
                let (ra, rb) = r.split_at_mut(half);
                let (ia, ib) = m.split_at_mut(half);
                for ((((ra, ia), rb), ib), &(c, s)) in ra
                    .iter_mut()
                    .zip(ia.iter_mut())
                    .zip(rb.iter_mut())
                    .zip(ib.iter_mut())
                    .zip(stage)
                {
                    let s = sign * s;
                    let xr = *rb * c - *ib * s;
                    let xi = *rb * s + *ib * c;
                    *rb = *ra - xr;
                    *ib = *ia - xi;
                    *ra += xr;
                    *ia += xi;
                }
            }
            half <<= 1;
        }
    }
}
