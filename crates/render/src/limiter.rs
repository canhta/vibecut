use crate::oversample::{Interpolator, PolyphaseBank, true_peak_factor};
use std::collections::VecDeque;

/// Linked-channel look-ahead dynamics with a final sample-peak safety ceiling.
pub(crate) struct Limiter {
    bank: PolyphaseBank,
    interp: Vec<Interpolator>,
    delay: Vec<Vec<f32>>,
    position: usize,
    look: usize,
    minimum: VecDeque<(u64, f32)>,
    envelope: f32,
    attack: f32,
    release: f32,
    index: u64,
    ceiling: f32,
}

impl Limiter {
    pub(crate) fn new(rate: u32, channels: u16) -> Self {
        let bank = PolyphaseBank::new(true_peak_factor(f64::from(rate)).max(2));
        let look = (rate as usize * 5 / 1000).max(1);
        let latency = look + bank.delay();
        Self {
            bank,
            interp: vec![Interpolator::default(); usize::from(channels)],
            delay: vec![vec![0.0; latency + 1]; usize::from(channels)],
            position: 0,
            look,
            minimum: VecDeque::with_capacity(look + 2),
            envelope: 1.0,
            attack: (-1.0 / (0.001 * rate as f32)).exp(),
            release: (-1.0 / (0.120 * rate as f32)).exp(),
            index: 0,
            ceiling: 10f32.powf(-1.0 / 20.0),
        }
    }

    pub(crate) fn latency(&self) -> usize {
        self.look + self.bank.delay()
    }

    pub(crate) fn process(&mut self, planes: &mut [Vec<f32>]) {
        let frames = planes.first().map_or(0, Vec::len);
        let length = self.latency() + 1;
        for n in 0..frames {
            let aligned = (self.position + length - self.bank.delay()) % length;
            let oldest = (self.position + 1) % length;
            let mut peak = 0.0f32;
            for ((plane, delay), interpolator) in
                planes.iter().zip(&mut self.delay).zip(&mut self.interp)
            {
                let x = plane[n];
                delay[self.position] = x;
                peak = peak
                    .max(delay[aligned].abs())
                    .max(interpolator.push_peak(&self.bank, x));
            }
            // 20:1 hard-knee curve, with linked stereo detection.
            let required = if peak > self.ceiling {
                (self.ceiling / peak).powf(1.0 - 1.0 / 20.0)
            } else {
                1.0
            };
            while self
                .minimum
                .back()
                .is_some_and(|&(_, gain)| gain >= required)
            {
                self.minimum.pop_back();
            }
            self.minimum.push_back((self.index, required));
            while self
                .minimum
                .front()
                .is_some_and(|&(index, _)| self.index - index > self.look as u64)
            {
                self.minimum.pop_front();
            }
            let target = self.minimum.front().map_or(1.0, |&(_, gain)| gain);
            let coefficient = if target < self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = target + coefficient * (self.envelope - target);
            for (plane, delay) in planes.iter_mut().zip(&self.delay) {
                // Hard clamp guarantees the sample ceiling even on impulses.
                plane[n] =
                    (delay[oldest] * self.envelope).clamp(-self.ceiling, self.ceiling) * 0.98;
            }
            self.position = (self.position + 1) % length;
            self.index += 1;
        }
    }
}
