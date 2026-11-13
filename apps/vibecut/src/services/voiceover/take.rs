//! Runs one take: the bounded queue, the encoder worker, stop, cancel and promotion.

use super::{Failure, InputHost, Opened, Shared, Sink};
use media::{AUDIO_FILE_BLOCK, AudioFileWriter, MediaError};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

/// Blocks the callback may queue before the take fails as an overrun.
pub const QUEUE_BLOCKS: usize = 64;
/// A take ends by itself at this length, which bounds disk use (about 57 MB/hour mono AAC).
pub const MAX_TAKE_SECONDS: u64 = 2 * 60 * 60;
const POLL: Duration = Duration::from_millis(20);

/// A promoted take: the file exists only because encoding finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub path: PathBuf,
    pub frames: u64,
    pub sample_rate: u32,
}

/// One running take. Dropping it cancels: the device stops, the worker is joined and
/// the unfinished file is removed.
pub struct Recorder {
    shared: Arc<Shared>,
    capture: Option<Box<dyn Send>>,
    worker: Option<JoinHandle<Result<u64, Failure>>>,
    part: PathBuf,
    path: PathBuf,
    sample_rate: u32,
}

impl Recorder {
    /// Open the device and start encoding into `directory`. Nothing reaches the final
    /// name until [`Recorder::stop`] succeeds.
    pub fn start(
        host: &dyn InputHost,
        device: Option<&str>,
        directory: &Path,
    ) -> Result<Self, Failure> {
        fs::create_dir_all(directory).map_err(|_| Failure::Storage)?;
        let path = free_path(directory)?;
        let part = part_path(&path);
        let shared = Arc::new(Shared::new());
        let (blocks, queue) = mpsc::sync_channel(QUEUE_BLOCKS);
        let Opened {
            sample_rate,
            channels,
            capture,
        } = host.open(device, Sink::new(blocks, shared.clone()))?;
        if channels == 0 || !(8_000..=192_000).contains(&sample_rate) {
            return Err(Failure::Unsupported);
        }
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = {
            let shared = shared.clone();
            let part = part.clone();
            std::thread::Builder::new()
                .name("voiceover-encoder".into())
                .spawn(move || work(&queue, &shared, &part, sample_rate, channels, &ready_tx))
                .map_err(|_| Failure::Storage)?
        };
        let mut recorder = Self {
            shared,
            capture: Some(capture),
            worker: Some(worker),
            part,
            path,
            sample_rate,
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(recorder),
            Ok(Err(failure)) => {
                recorder.discard();
                Err(failure)
            }
            Err(_) => {
                recorder.discard();
                Err(Failure::Storage)
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Frames encoded so far; elapsed time is this over [`Recorder::sample_rate`].
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Acquire)
    }

    /// Largest sample magnitude since the previous call (0.0 to above 1.0 when clipping).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.peak.swap(0, Ordering::AcqRel))
    }

    /// The first failure the device, queue or worker reported.
    pub fn failure(&self) -> Option<Failure> {
        self.shared.failure()
    }

    /// The take reached [`MAX_TAKE_SECONDS`]; the caller stops it.
    pub fn full(&self) -> bool {
        self.shared.full.load(Ordering::Acquire)
    }

    /// Stop the device, finish the file and promote it. A failed take leaves no file.
    pub fn stop(mut self) -> Result<Finished, Failure> {
        self.shared.stop.store(true, Ordering::Release);
        drop(self.capture.take());
        let frames = match self.join() {
            Ok(frames) => frames,
            Err(failure) => {
                let _ = fs::remove_file(&self.part);
                return Err(failure);
            }
        };
        if frames == 0 {
            let _ = fs::remove_file(&self.part);
            return Err(Failure::Empty);
        }
        if fs::rename(&self.part, &self.path).is_err() {
            let _ = fs::remove_file(&self.part);
            return Err(Failure::Storage);
        }
        Ok(Finished {
            path: self.path.clone(),
            frames,
            sample_rate: self.sample_rate,
        })
    }

    /// Stop the device, join the worker and delete the unfinished file.
    pub fn cancel(mut self) {
        self.discard();
    }

    fn discard(&mut self) {
        self.shared.cancel.store(true, Ordering::Release);
        drop(self.capture.take());
        let _ = self.join();
        let _ = fs::remove_file(&self.part);
    }

    fn join(&mut self) -> Result<u64, Failure> {
        match self.worker.take() {
            Some(worker) => worker.join().unwrap_or(Err(Failure::Encode)),
            None => Err(Failure::Encode),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.discard();
        }
    }
}

fn free_path(directory: &Path) -> Result<PathBuf, Failure> {
    (1_u32..10_000)
        .map(|n| directory.join(format!("voiceover-{n}.m4a")))
        .find(|path| !path.exists() && !part_path(path).exists())
        .ok_or(Failure::Storage)
}

fn part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_os_string();
    part.push(".part");
    PathBuf::from(part)
}

fn classify(error: &MediaError) -> Failure {
    match error {
        MediaError::Io(_) => Failure::Storage,
        _ => Failure::Encode,
    }
}

/// The encoder worker. It owns the writer, so the muxer never crosses threads.
fn work(
    queue: &Receiver<Vec<f32>>,
    shared: &Shared,
    part: &Path,
    sample_rate: u32,
    channels: u16,
    ready: &SyncSender<Result<(), Failure>>,
) -> Result<u64, Failure> {
    let mut writer = match AudioFileWriter::create(part, sample_rate, 1) {
        Ok(writer) => {
            let _ = ready.send(Ok(()));
            writer
        }
        Err(error) => {
            let failure = classify(&error);
            let _ = ready.send(Err(failure));
            return Err(failure);
        }
    };
    let channels = usize::from(channels);
    let cap = u64::from(sample_rate) * MAX_TAKE_SECONDS;
    let mut mono = Vec::new();
    loop {
        if shared.cancel.load(Ordering::Acquire) {
            return Ok(0);
        }
        if let Some(failure) = shared.failure() {
            return Err(failure);
        }
        match queue.recv_timeout(POLL) {
            Ok(block) => encode(&mut writer, shared, &block, channels, cap, &mut mono)?,
            Err(RecvTimeoutError::Timeout) => {
                if shared.stop.load(Ordering::Acquire) {
                    while let Ok(block) = queue.try_recv() {
                        encode(&mut writer, shared, &block, channels, cap, &mut mono)?;
                    }
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Some(failure) = shared.failure() {
        return Err(failure);
    }
    if shared.frames.load(Ordering::Acquire) == 0 {
        // Nothing to finish; the caller reports an empty take and removes the file.
        return Ok(0);
    }
    writer.finish().map_err(|error| classify(&error))?;
    Ok(shared.frames.load(Ordering::Acquire))
}

/// Downmix one interleaved block to mono and append it, up to the take cap.
fn encode(
    writer: &mut AudioFileWriter,
    shared: &Shared,
    block: &[f32],
    channels: usize,
    cap: u64,
    mono: &mut Vec<f32>,
) -> Result<(), Failure> {
    let done = shared.frames.load(Ordering::Acquire);
    if done >= cap {
        shared.full.store(true, Ordering::Release);
        return Ok(());
    }
    mono.clear();
    mono.extend(block.chunks_exact(channels).map(|frame| {
        let sum: f32 = frame
            .iter()
            .map(|sample| if sample.is_finite() { *sample } else { 0.0 })
            .sum();
        (sum / channels as f32).clamp(-1.0, 1.0)
    }));
    mono.truncate(usize::try_from(cap - done).unwrap_or(usize::MAX));
    for plane in mono.chunks(AUDIO_FILE_BLOCK) {
        writer
            .write(&[plane.to_vec()])
            .map_err(|error| classify(&error))?;
        shared
            .frames
            .fetch_add(plane.len() as u64, Ordering::AcqRel);
    }
    Ok(())
}
