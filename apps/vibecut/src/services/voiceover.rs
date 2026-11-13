//! Captures one voice-over take from an injected input host into a promoted audio file.
//!
//! The host is the native boundary: production composes [`CpalHost`], tests inject a
//! scripted host. Everything after the device callback is shared, so the bounded queue,
//! the encoder worker, cancellation and promotion are exercised without a microphone.

mod device;
mod take;

pub use device::CpalHost;
pub use take::{Finished, MAX_TAKE_SECONDS, QUEUE_BLOCKS, Recorder, TakeWriter, WriterFactory};

use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    mpsc::{SyncSender, TrySendError},
};

/// Largest slice one queued block carries; a bigger callback is split so one
/// allocation in the audio callback stays small.
pub(crate) const MAX_BLOCK_SAMPLES: usize = 16_384;

/// Every way a take can fail; the dialog words each one for the creator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The operating system refused microphone access.
    #[error("microphone access was denied")]
    PermissionDenied,
    /// The host reports no input device, or the named one is gone.
    #[error("no input device")]
    NoDevice,
    /// The device disappeared, was taken by another app or stopped delivering mid-take.
    #[error("input device became unavailable")]
    Unavailable,
    /// The device offers no 32-bit float input this build can read.
    #[error("input format is unsupported")]
    Unsupported,
    /// The encoder fell behind and the bounded queue filled; a take with a gap is never kept.
    #[error("recording could not keep up")]
    Overrun,
    /// The take file could not be created, written or promoted (disk full, no access).
    #[error("take storage failed")]
    Storage,
    /// The audio encoder refused the take.
    #[error("take encoding failed")]
    Encode,
    /// Stopped with no sample captured.
    #[error("nothing was recorded")]
    Empty,
}

/// What [`InputHost::open`] returns: the negotiated format and the running capture.
/// Dropping `capture` stops the device and releases its callback (and with it the sink).
pub struct Opened {
    pub sample_rate: u32,
    pub channels: u16,
    pub capture: Box<dyn Send>,
}

/// The device seam. Implementations deliver interleaved f32 samples to the [`Sink`].
pub trait InputHost: Send + Sync {
    /// Input device names, default first. Empty means no input exists.
    fn devices(&self) -> Result<Vec<String>, Failure>;
    /// Start capture on `device` (`None` is the host default).
    fn open(&self, device: Option<&str>, sink: Sink) -> Result<Opened, Failure>;
}

/// State the callback, worker and dialog share. Atomics only on the callback path.
pub(crate) struct Shared {
    /// f32 bits of the largest magnitude since the last [`Recorder::level`] read;
    /// non-negative floats order like their bits.
    peak: AtomicU32,
    /// Frames the worker has encoded.
    frames: AtomicU64,
    overrun: AtomicBool,
    stop: AtomicBool,
    cancel: AtomicBool,
    full: AtomicBool,
    failure: Mutex<Option<Failure>>,
}

impl Shared {
    pub(crate) fn new() -> Self {
        Self {
            peak: AtomicU32::new(0),
            frames: AtomicU64::new(0),
            overrun: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            full: AtomicBool::new(false),
            failure: Mutex::new(None),
        }
    }

    pub(crate) fn fail(&self, failure: Failure) {
        let mut slot = self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        slot.get_or_insert(failure);
    }

    pub(crate) fn failure(&self) -> Option<Failure> {
        let slot = *self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        slot.or_else(|| {
            self.overrun
                .load(Ordering::Acquire)
                .then_some(Failure::Overrun)
        })
    }
}

/// The callback side of the bounded queue. [`Sink::push`] never blocks and never
/// waits on the worker: a full queue records [`Failure::Overrun`] instead.
#[derive(Clone)]
pub struct Sink {
    blocks: SyncSender<Vec<f32>>,
    shared: Arc<Shared>,
}

impl Sink {
    pub(crate) fn new(blocks: SyncSender<Vec<f32>>, shared: Arc<Shared>) -> Self {
        Self { blocks, shared }
    }

    /// Hand one interleaved callback buffer of `channels` channels to the worker.
    pub fn push(&self, samples: &[f32], channels: u16) {
        self.push_as(samples, channels, |sample| sample);
    }

    /// Signed 16-bit input, scaled to -1.0..1.0.
    pub fn push_i16(&self, samples: &[i16], channels: u16) {
        self.push_as(samples, channels, |sample| f32::from(sample) / 32_768.0);
    }

    /// Unsigned 16-bit input (offset binary), scaled to -1.0..1.0.
    pub fn push_u16(&self, samples: &[u16], channels: u16) {
        self.push_as(samples, channels, |sample| {
            (f32::from(sample) - 32_768.0) / 32_768.0
        });
    }

    /// Signed 32-bit input, scaled to -1.0..1.0.
    pub fn push_i32(&self, samples: &[i32], channels: u16) {
        self.push_as(samples, channels, |sample| sample as f32 / 2_147_483_648.0);
    }

    /// Convert straight into the queued block, so each callback allocates only that
    /// block. Splits fall on whole frames so the worker never sees a torn frame.
    fn push_as<T: Copy>(&self, samples: &[T], channels: u16, convert: impl Fn(T) -> f32) {
        let channels = usize::from(channels);
        if channels == 0 {
            return;
        }
        let step = (MAX_BLOCK_SAMPLES / channels).max(1) * channels;
        let mut peak = 0.0_f32;
        for chunk in samples.chunks(step) {
            let block: Vec<f32> = chunk
                .iter()
                .map(|sample| {
                    let value = convert(*sample);
                    peak = peak.max(value.abs());
                    value
                })
                .collect();
            if let Err(TrySendError::Full(_)) = self.blocks.try_send(block) {
                self.shared.overrun.store(true, Ordering::Release);
            }
        }
        let peak = if peak.is_finite() { peak } else { 1.0 };
        self.shared.peak.fetch_max(peak.to_bits(), Ordering::AcqRel);
    }

    /// Report that the device failed (the stream's error callback).
    pub fn fail(&self, failure: Failure) {
        self.shared.fail(failure);
    }
}
