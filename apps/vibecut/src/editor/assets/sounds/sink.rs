//! Owns cancellable sound-device playback on the caller's retained preview worker.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine::{ByteBudget, ByteLease, ByteOwner, SoundError};
use libraries::sounds::{PreviewClip, PreviewSink, Sound};
use media::{AudioDecoder, AudioResampler, AudioSampleFormat, PacketReader, StreamKind};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_PREVIEW_SECONDS: usize = 120;
const MAX_ENCODED_BYTES: usize = 32 << 20;
const POLL: Duration = Duration::from_millis(10);
const DEVICE_TIMEOUT: Duration = Duration::from_secs(1);

// The native stream stays on the same calling worker that created it.
trait Playing {
    fn now(&self) -> cpal::StreamInstant;
}
impl Playing for cpal::Stream {
    fn now(&self) -> cpal::StreamInstant {
        StreamTrait::now(self)
    }
}
trait Device: Send + Sync {
    fn start(&self, decode: &mut dyn FnMut(u32) -> Sound<Render>) -> Sound<Box<dyn Playing>>;
}
struct Native;
impl Device for Native {
    fn start(&self, decode: &mut dyn FnMut(u32) -> Sound<Render>) -> Sound<Box<dyn Playing>> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(SoundError::NoAudioDevice)?;
        let supported = device
            .supported_output_configs()
            .map_err(|_| SoundError::NoAudioDevice)?
            .filter(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
            .find_map(|c| c.try_with_sample_rate(cpal::SAMPLE_RATE_48K))
            .or_else(|| {
                device
                    .default_output_config()
                    .ok()
                    .filter(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
            })
            .ok_or(SoundError::NoAudioDevice)?;
        let mut render = decode(supported.sample_rate())?;
        if render.cancel.load(Ordering::Acquire) {
            return Err(SoundError::Cancelled);
        }
        let status = render.status.clone();
        let stream = device
            .build_output_stream(
                supported.config(),
                move |data: &mut [f32], info| render.fill(data, info.timestamp()),
                move |_| {
                    status.failed.store(true, Ordering::Release);
                },
                Some(DEVICE_TIMEOUT),
            )
            .map_err(|_| SoundError::NoAudioDevice)?;
        stream.play().map_err(|_| SoundError::NoAudioDevice)?;
        Ok(Box::new(stream))
    }
}

#[derive(Default)]
struct Requests {
    active: Option<Arc<AtomicBool>>,
    latest: Option<Arc<AtomicBool>>,
}
/// Blocking terminal preview sink. One caller owns the device; only its newest
/// replacement may wait, borrowing its caller's bytes without copying them.
/// Stop invalidates both immediately. The caller must retain/join its play worker.
pub struct OutputSink {
    budget: ByteBudget,
    device: Arc<dyn Device>,
    requests: Mutex<Requests>,
    wake: Condvar,
}
impl OutputSink {
    /// Use the existing shared budget for encoded copies (<=32 MiB) and stereo
    /// f32 PCM (<=120 s at 8..=192 kHz). Refusal is typed Limit before allocation.
    pub fn new(budget: ByteBudget) -> Self {
        Self {
            budget,
            device: Arc::new(Native),
            requests: Mutex::new(Requests::default()),
            wake: Condvar::new(),
        }
    }
    fn enter(&self, cancelled: &dyn Fn() -> bool) -> Sound<Playback<'_>> {
        let cancel = Arc::new(AtomicBool::new(false));
        let playback = Playback {
            sink: self,
            cancel: cancel.clone(),
        };
        {
            let mut requests = self
                .requests
                .lock()
                .map_err(|_| SoundError::NoAudioDevice)?;
            if let Some(old) = requests.latest.replace(cancel.clone()) {
                old.store(true, Ordering::Release);
            }
        }
        self.wake.notify_all();
        loop {
            playback.check(cancelled)?;
            let mut requests = self
                .requests
                .lock()
                .map_err(|_| SoundError::NoAudioDevice)?;
            if cancel.load(Ordering::Acquire) {
                return Err(SoundError::Cancelled);
            }
            if requests.active.is_none() {
                requests.active = Some(cancel.clone());
                return Ok(playback);
            }
            // Releases the state lock. No state lock spans decode, device or disk IO.
            requests = self
                .wake
                .wait_timeout(requests, POLL)
                .map_err(|_| SoundError::NoAudioDevice)?
                .0;
            drop(requests);
        }
    }
    fn play_clip(&self, clip: PreviewClip<'_>) -> Sound<()> {
        if (clip.cancelled)() {
            return Err(SoundError::Cancelled);
        }
        let playback = self.enter(clip.cancelled)?;
        playback.check(clip.cancelled)?;
        if clip.bytes.is_empty() {
            return Err(SoundError::InvalidResponse);
        }
        if clip.bytes.len() > MAX_ENCODED_BYTES {
            return Err(SoundError::Limit);
        }
        let _encoded = reserve(&self.budget, ByteOwner::CacheReads, clip.bytes.len())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(clip.bytes.len())
            .map_err(|_| SoundError::Limit)?;
        for part in clip.bytes.chunks(64 << 10) {
            playback.check(clip.cancelled)?;
            bytes.extend_from_slice(part);
        }
        let status = Arc::new(Status::default());
        let mut pcm = None; // Retain storage/lease until after the native stream is destroyed.
        let stream = self.device.start(&mut |rate| {
            playback.check(clip.cancelled)?;
            let samples = Arc::new(decode(&bytes, rate, &self.budget, &|| {
                playback.check(clip.cancelled)
            })?);
            pcm = Some(samples.clone());
            Ok(Render {
                samples,
                status: status.clone(),
                cancel: playback.cancel.clone(),
                cursor: 0,
                rate,
            })
        })?;
        let mut callbacks = 0;
        let mut last_callback = Instant::now();
        let result = loop {
            if let Err(error) = playback.check(clip.cancelled) {
                break Err(error);
            }
            if status.failed.load(Ordering::Acquire) {
                break Err(SoundError::NoAudioDevice);
            }
            let end = status.end.load(Ordering::Acquire);
            let now = stream.now();
            // The DAC timestamp is predicted by CPAL, not proof of physical audibility.
            if end != 0 && now.as_nanos() >= u128::from(end) {
                break Ok(());
            }
            let next = status.callbacks.load(Ordering::Acquire);
            if next != callbacks {
                callbacks = next;
                last_callback = Instant::now();
            }
            if last_callback.elapsed() > DEVICE_TIMEOUT {
                break Err(SoundError::NoAudioDevice);
            }
            let requests = self
                .requests
                .lock()
                .map_err(|_| SoundError::NoAudioDevice)?;
            drop(
                self.wake
                    .wait_timeout(requests, POLL)
                    .map_err(|_| SoundError::NoAudioDevice)?,
            );
        };
        drop(stream);
        drop(pcm);
        result
    }
}
impl PreviewSink for OutputSink {
    fn play(&self, clip: PreviewClip<'_>) -> Sound<()> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.play_clip(clip)))
            .unwrap_or(Err(SoundError::NoAudioDevice))
    }
    fn stop(&self) -> Sound<()> {
        let mut requests = self
            .requests
            .lock()
            .map_err(|_| SoundError::NoAudioDevice)?;
        if let Some(latest) = requests.latest.take() {
            latest.store(true, Ordering::Release);
        }
        if let Some(active) = &requests.active {
            active.store(true, Ordering::Release);
        }
        self.wake.notify_all();
        Ok(())
    }
}
struct Playback<'a> {
    sink: &'a OutputSink,
    cancel: Arc<AtomicBool>,
}
impl Playback<'_> {
    fn check(&self, cancelled: &dyn Fn() -> bool) -> Sound<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(SoundError::Cancelled);
        }
        let external = cancelled();
        if external || self.cancel.load(Ordering::Acquire) {
            Err(SoundError::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Drop for Playback<'_> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Ok(mut requests) = self.sink.requests.lock() {
            if requests
                .active
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &self.cancel))
            {
                requests.active = None;
            }
            if requests
                .latest
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &self.cancel))
            {
                requests.latest = None;
            }
        }
        self.sink.wake.notify_all();
    }
}
#[derive(Default)]
struct Status {
    end: AtomicU64,
    failed: AtomicBool,
    callbacks: AtomicU64,
}
struct Pcm {
    samples: Vec<f32>,
    _lease: ByteLease,
}
struct Render {
    samples: Arc<Pcm>,
    status: Arc<Status>,
    cancel: Arc<AtomicBool>,
    cursor: usize,
    rate: u32,
}
impl Render {
    fn fill(&mut self, data: &mut [f32], timestamp: cpal::OutputStreamTimestamp) {
        data.fill(0.0);
        self.status.callbacks.fetch_add(1, Ordering::Release);
        if self.cancel.load(Ordering::Acquire) {
            return;
        }
        let remaining = self.samples.samples.get(self.cursor..).unwrap_or_default();
        let count = data.len().min(remaining.len());
        if let Some(target) = data.get_mut(..count) {
            target.copy_from_slice(remaining.get(..count).unwrap_or_default());
        }
        self.cursor = self.cursor.saturating_add(count);
        if count != 0 && self.cursor == self.samples.samples.len() {
            let end = u64::try_from(count / 2)
                .ok()
                .and_then(|frames| frames.checked_mul(1_000_000_000))
                .map(|ns| ns.div_ceil(u64::from(self.rate)))
                .and_then(|ns| timestamp.playback.checked_add(Duration::from_nanos(ns)))
                .and_then(|end| u64::try_from(end.as_nanos()).ok());
            match end {
                Some(end) => self.status.end.store(end, Ordering::Release),
                None => self.status.failed.store(true, Ordering::Release),
            }
        }
    }
}
fn reserve(budget: &ByteBudget, owner: ByteOwner, bytes: usize) -> Sound<ByteLease> {
    budget
        .reserve(owner, u64::try_from(bytes).map_err(|_| SoundError::Limit)?)
        .map_err(|e| match e {
            engine::EditError::Io(e) => SoundError::Io(e),
            _ => SoundError::Limit,
        })
}

struct TempAudio(PathBuf, bool);
impl Drop for TempAudio {
    fn drop(&mut self) {
        if self.1 {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
fn temp_file() -> Sound<(TempAudio, std::fs::File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| SoundError::Limit)?;
    let path = std::env::temp_dir().join(format!(
        "vibecut-sound-preview-{}-{id}.audio",
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path).map_err(SoundError::Io)?;
    Ok((TempAudio(path, true), file))
}
fn decode(
    bytes: &[u8],
    rate: u32,
    budget: &ByteBudget,
    check: &dyn Fn() -> Sound<()>,
) -> Sound<Pcm> {
    if !(8_000..=192_000).contains(&rate) {
        return Err(SoundError::Limit);
    }
    let limit = usize::try_from(rate)
        .ok()
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_mul(MAX_PREVIEW_SECONDS))
        .ok_or(SoundError::Limit)?;
    let lease = reserve(
        budget,
        ByteOwner::DecodedFrames,
        limit
            .checked_mul(size_of::<f32>())
            .ok_or(SoundError::Limit)?,
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(limit)
        .map_err(|_| SoundError::Limit)?;
    check()?;
    let (mut temp, mut file) = temp_file()?;
    let result = (|| {
        for part in bytes.chunks(64 << 10) {
            check()?;
            file.write_all(part).map_err(SoundError::Io)?;
        }
        file.sync_all().map_err(SoundError::Io)?;
        drop(file);
        check()?;
        let mut reader = PacketReader::open(&temp.0).map_err(SoundError::Media)?;
        let stream = reader
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Audio)
            .cloned()
            .ok_or(SoundError::InvalidResponse)?;
        let mut decoder = AudioDecoder::new(&stream).map_err(SoundError::Media)?;
        let mut resampler = None;
        let mut append = |frame: &media::AudioFrame| -> Sound<()> {
            check()?;
            let count = frame.samples().checked_mul(2).ok_or(SoundError::Limit)?;
            if output.len().checked_add(count).is_none_or(|n| n > limit) {
                return Err(SoundError::Limit);
            }
            let _planes = reserve(
                budget,
                ByteOwner::DecodedFrames,
                count
                    .checked_mul(size_of::<f32>())
                    .ok_or(SoundError::Limit)?,
            )?;
            let planes = frame.planar_f32().map_err(SoundError::Media)?;
            let left = planes.first().map(Vec::as_slice).unwrap_or_default();
            let right = planes.get(1).map(Vec::as_slice).unwrap_or(left);
            for index in 0..left.len().max(right.len()) {
                if index % 4096 == 0 {
                    check()?;
                }
                if output.len().checked_add(2).is_none_or(|n| n > limit) {
                    return Err(SoundError::Limit);
                }
                output.push(left.get(index).copied().unwrap_or(0.0));
                output.push(right.get(index).copied().unwrap_or(0.0));
            }
            Ok(())
        };
        let mut eof = false;
        loop {
            check()?;
            while let Some(frame) = decoder.pull_frame().map_err(SoundError::Media)? {
                check()?;
                if resampler.is_none() {
                    resampler = Some(
                        AudioResampler::new(&frame, rate, 2, AudioSampleFormat::PlanarF32)
                            .map_err(SoundError::Media)?,
                    );
                }
                for frame in resampler
                    .as_mut()
                    .ok_or(SoundError::InvalidResponse)?
                    .convert(&frame)
                    .map_err(SoundError::Media)?
                {
                    append(&frame)?;
                }
            }
            if eof {
                break;
            }
            check()?;
            match reader.next_packet().map_err(SoundError::Media)? {
                Some(packet) if packet.stream() == stream.index => {
                    decoder.push_packet(&packet).map_err(SoundError::Media)?
                }
                Some(_) => {}
                None => {
                    eof = true;
                    decoder.finish_input().map_err(SoundError::Media)?;
                }
            }
        }
        if let Some(resampler) = resampler.as_mut()
            && let Some(frame) = resampler.flush().map_err(SoundError::Media)?
        {
            append(&frame)?;
        }
        check()?;
        if output.is_empty() {
            return Err(SoundError::InvalidResponse);
        }
        Ok(Pcm {
            samples: output,
            _lease: lease,
        })
    })();
    std::fs::remove_file(&temp.0).map_err(SoundError::Io)?;
    // Drop only cleans during unwinding; normal cleanup errors remain typed above.
    temp.1 = false;
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use libraries::sounds::{SoundDownloads, SoundLibrary, SoundPreview, SoundResult};
    use std::{
        sync::mpsc::{self, Receiver, SyncSender},
        thread::JoinHandle,
    };
    struct PumpDevice(SyncSender<Pump>);
    struct Pump {
        render: Arc<Mutex<Option<Render>>>,
        clock: Arc<AtomicU64>,
        closed: Receiver<()>,
        closing: Receiver<()>,
        polled: Receiver<u64>,
    }
    struct PumpStream {
        render: Arc<Mutex<Option<Render>>>,
        clock: Arc<AtomicU64>,
        closed: SyncSender<()>,
        closing: SyncSender<()>,
        polled: SyncSender<u64>,
    }
    impl Playing for PumpStream {
        fn now(&self) -> cpal::StreamInstant {
            let now = self.clock.load(Ordering::Acquire);
            let _ = self.polled.try_send(now);
            cpal::StreamInstant::from_nanos(now)
        }
    }
    impl Drop for PumpStream {
        fn drop(&mut self) {
            let _ = self.closing.send(());
            self.render.lock().unwrap().take();
            let _ = self.closed.send(());
        }
    }
    impl Device for PumpDevice {
        fn start(&self, decode: &mut dyn FnMut(u32) -> Sound<Render>) -> Sound<Box<dyn Playing>> {
            let render = Arc::new(Mutex::new(Some(decode(8_000)?)));
            let clock = Arc::new(AtomicU64::new(0));
            let (tx, closed) = mpsc::sync_channel(1);
            let (closing_tx, closing) = mpsc::sync_channel(1);
            let (polled_tx, polled) = mpsc::sync_channel(1);
            self.0
                .send(Pump {
                    render: render.clone(),
                    clock: clock.clone(),
                    closed,
                    closing,
                    polled,
                })
                .unwrap();
            Ok(Box::new(PumpStream {
                render,
                clock,
                closed: tx,
                closing: closing_tx,
                polled: polled_tx,
            }))
        }
    }
    impl Pump {
        fn observed(&self, clock: u64) {
            for _ in 0..2 {
                loop {
                    if self.polled.recv_timeout(Duration::from_secs(5)).unwrap() == clock {
                        break;
                    }
                }
            }
        }
        fn fill(&self, callback: u64, playback: u64) -> Vec<f32> {
            let mut data = vec![0.0; 256];
            self.render.lock().unwrap().as_mut().unwrap().fill(
                &mut data,
                cpal::OutputStreamTimestamp {
                    callback: cpal::StreamInstant::from_millis(callback),
                    playback: cpal::StreamInstant::from_millis(playback),
                },
            );
            data
        }
        fn fail(&self) {
            self.render
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .status
                .failed
                .store(true, Ordering::Release);
        }
        fn drain(&self) {
            self.clock.store(110_000_000, Ordering::Release);
        }
        fn closed(&self) {
            self.closed.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
    fn sink(limit: u64) -> (Arc<OutputSink>, Receiver<Pump>, ByteBudget) {
        let budget = ByteBudget::new(limit).unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let mut sink = OutputSink::new(budget.clone());
        sink.device = Arc::new(PumpDevice(tx));
        (Arc::new(sink), rx, budget)
    }
    fn wav(frames: usize) -> Vec<u8> {
        let len = u32::try_from(frames * 2).unwrap();
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(36 + len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16_u32.to_le_bytes());
        b.extend_from_slice(&1_u16.to_le_bytes());
        b.extend_from_slice(&1_u16.to_le_bytes());
        b.extend_from_slice(&8000_u32.to_le_bytes());
        b.extend_from_slice(&16000_u32.to_le_bytes());
        b.extend_from_slice(&2_u16.to_le_bytes());
        b.extend_from_slice(&16_u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&len.to_le_bytes());
        for _ in 0..frames {
            b.extend_from_slice(&8192_i16.to_le_bytes());
        }
        b
    }
    fn play(sink: Arc<OutputSink>, bytes: Vec<u8>) -> JoinHandle<Sound<()>> {
        std::thread::spawn(move || {
            sink.play(PreviewClip {
                name: "Preview",
                audio_url: "https://example.org/a.wav",
                bytes: &bytes,
                cancelled: &|| false,
            })
        })
    }
    fn pump(rx: &Receiver<Pump>) -> Pump {
        rx.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn preview_lifetime_waits_for_dac_drain_and_releases_bytes() {
        let (sink, rx, budget) = sink(16 << 20);
        let worker = play(sink, wav(80));
        let pump = pump(&rx);
        pump.observed(0);
        assert!(!worker.is_finished(), "startup is not completion");
        assert!(pump.fill(0, 100).iter().any(|s| *s > 0.0));
        pump.clock.store(109_999_999, Ordering::Release);
        pump.observed(109_999_999);
        assert!(
            !worker.is_finished(),
            "submitted PCM is still ahead of DAC time"
        );
        assert!(budget.snapshot().unwrap().used > 7_680_000);
        pump.drain();
        worker.join().unwrap().unwrap();
        pump.closed();
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
    #[test]
    fn preview_stop_and_device_failure_are_terminal_and_reusable() {
        let (sink, rx, budget) = sink(16 << 20);
        for failed in [false, true] {
            let worker = play(sink.clone(), wav(80));
            let pump = pump(&rx);
            pump.fill(0, 100);
            if failed {
                pump.fail();
            } else {
                sink.stop().unwrap();
                sink.stop().unwrap();
            }
            let result = worker.join().unwrap();
            if failed {
                assert!(matches!(result, Err(SoundError::NoAudioDevice)));
            } else {
                assert!(matches!(result, Err(SoundError::Cancelled)));
            }
            pump.closed();
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
        let worker = play(sink, wav(80));
        let pump = pump(&rx);
        pump.fill(0, 100);
        pump.drain();
        worker.join().unwrap().unwrap();
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
    #[test]
    fn preview_only_latest_replacement_survives_native_teardown() {
        let (sink, rx, budget) = sink(16 << 20);
        let first = play(sink.clone(), wav(80));
        let old = pump(&rx);
        // A native callback still owns the buffer while stream destruction starts.
        let callback = old.render.lock().unwrap();
        let charged = budget.snapshot().unwrap().used;
        let second = play(sink.clone(), wav(80));
        old.closing.recv_timeout(Duration::from_secs(5)).unwrap();
        let third = play(sink, wav(80));
        assert!(matches!(second.join().unwrap(), Err(SoundError::Cancelled)));
        assert_eq!(
            budget.snapshot().unwrap().used,
            charged,
            "pending requests copy no bytes"
        );
        assert!(
            rx.try_recv().is_err(),
            "old stream must finish destruction first"
        );
        drop(callback);
        assert!(matches!(first.join().unwrap(), Err(SoundError::Cancelled)));
        old.closed();
        let latest = pump(&rx);
        latest.fill(0, 100);
        latest.drain();
        third.join().unwrap().unwrap();
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
    #[test]
    fn preview_stop_interrupts_decode_before_device_start() {
        let (sink, rx, budget) = sink(16 << 20);
        let (entered, gate) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let playing = sink.clone();
        let accounting = budget.clone();
        let worker = std::thread::spawn(move || {
            let armed = AtomicBool::new(false);
            let bytes = wav(80);
            playing.play(PreviewClip {
                name: "Preview",
                audio_url: "https://example.org/a.wav",
                bytes: &bytes,
                cancelled: &|| {
                    if accounting.snapshot().unwrap().owners[0] > 7_680_000
                        && !armed.swap(true, Ordering::AcqRel)
                    {
                        entered.send(()).unwrap();
                        wait.recv().unwrap();
                    }
                    false
                },
            })
        });
        gate.recv_timeout(Duration::from_secs(5)).unwrap();
        sink.stop().unwrap();
        release.send(()).unwrap();
        assert!(matches!(worker.join().unwrap(), Err(SoundError::Cancelled)));
        assert!(rx.try_recv().is_err());
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
    struct Refuse(bool);
    impl Device for Refuse {
        fn start(&self, decode: &mut dyn FnMut(u32) -> Sound<Render>) -> Sound<Box<dyn Playing>> {
            if self.0 {
                let _render = decode(8_000)?;
                panic!("device startup panicked");
            }
            Err(SoundError::NoAudioDevice)
        }
    }
    #[test]
    fn preview_refusals_and_panics_leave_no_owned_buffers() {
        for (bytes, limit) in [
            (Vec::new(), 16 << 20),
            (vec![0; MAX_ENCODED_BYTES + 1], 16 << 20),
            (wav(80), 1),
            (wav(80), 1024),
            (vec![1, 2, 3], 16 << 20),
            (wav(121 * 8000), 16 << 20),
        ] {
            let (sink, rx, budget) = sink(limit);
            let result = play(sink, bytes.clone()).join().unwrap();
            if bytes.is_empty() {
                assert!(matches!(result, Err(SoundError::InvalidResponse)));
            } else if bytes.len() == 3 {
                assert!(matches!(result, Err(SoundError::Media(_))));
            } else {
                assert!(matches!(result, Err(SoundError::Limit)));
            }
            assert!(rx.try_recv().is_err());
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
        for panic in [false, true] {
            let budget = ByteBudget::new(16 << 20).unwrap();
            let mut sink = OutputSink::new(budget.clone());
            sink.device = Arc::new(Refuse(panic));
            let sink = Arc::new(sink);
            for _ in 0..2 {
                assert!(matches!(
                    play(sink.clone(), wav(80)).join().unwrap(),
                    Err(SoundError::NoAudioDevice)
                ));
            }
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
    }

    struct Offline;
    impl ai::transport::DownloadTransport for Offline {
        fn get(
            &self,
            _: &str,
            _: &[(String, String)],
        ) -> std::result::Result<ai::transport::HttpStreamResponse, ai::Error> {
            panic!("cached preview must stay offline")
        }
    }
    fn sound(id: u64) -> SoundResult {
        SoundResult {
            id,
            name: format!("Preview {id}"),
            author: "Author".into(),
            licence: "Creative Commons 0".into(),
            attribution: "Preview by Author".into(),
            source_url: "https://example.org/source".into(),
            audio_url: "https://example.org/a.wav".into(),
            duration_ms: 10,
            downloads: 0,
        }
    }
    fn until<T>(mut action: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = action() {
                return value;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn preview_real_sounds_tracks_terminal_state_replaces_latest_and_joins_close() {
        let root = tempfile::tempdir().unwrap();
        let library = Arc::new(SoundLibrary::open(root.path()).unwrap());
        for id in 1..=3 {
            library.save(&sound(id)).unwrap();
        }
        library.store_cached(&sound(1).audio_url, &wav(80)).unwrap();
        let downloads = Arc::new(SoundDownloads::new(library.clone(), Arc::new(Offline)));
        let (sink, rx, budget) = sink(16 << 20);
        let preview = Arc::new(SoundPreview::new(downloads.clone(), sink));
        let mut sounds =
            super::super::Sounds::with_parts(library, None, downloads, preview, budget.clone())
                .unwrap();
        sounds.play(1);
        let first = pump(&rx);
        sounds.tick().unwrap();
        assert_eq!(sounds.view().previewing, Some(1));
        first.fill(0, 100);
        sounds.tick().unwrap();
        assert_eq!(sounds.view().previewing, Some(1));
        first.drain();
        until(|| {
            sounds.tick().unwrap();
            (sounds.view().previewing.is_none()).then_some(())
        });
        assert_eq!(budget.snapshot().unwrap().used, 0);
        sounds.play(1);
        let first = pump(&rx);
        sounds.play(2);
        sounds.play(3);
        first.closed();
        let latest = until(|| {
            sounds.tick().unwrap();
            rx.try_recv().ok()
        });
        assert_eq!(sounds.view().previewing, Some(3));
        assert!(rx.try_recv().is_err(), "no obsolete intermediate playback");
        drop(sounds);
        latest.closed();
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
    #[test]
    fn preview_real_sound_owner_cancellation_before_entry_never_opens_device() {
        let root = tempfile::tempdir().unwrap();
        let library = Arc::new(SoundLibrary::open(root.path()).unwrap());
        let downloads = Arc::new(SoundDownloads::new(library, Arc::new(Offline)));
        let (sink, rx, budget) = sink(16 << 20);
        let preview = SoundPreview::new(downloads, sink);
        assert!(matches!(
            preview.play(&sound(1), &|| true),
            Err(SoundError::Cancelled)
        ));
        assert!(rx.try_recv().is_err());
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
