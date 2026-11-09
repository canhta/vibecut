//! App-owned cpal preview sink. Decoding and device lifetime stay on one
//! worker thread; the engine still owns the play/stop/cancel contract.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine::SoundError;
use libraries::sounds::{PreviewClip, PreviewSink};
use media::{AudioDecoder, AudioResampler, AudioSampleFormat, PacketReader, StreamKind};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

/// Preview clips are short; bound decoded PCM so a hostile file cannot grow the
/// editor process without limit.
const MAX_PREVIEW_SECONDS: u32 = 120;

enum Command {
    Play {
        bytes: Vec<u8>,
        reply: mpsc::SyncSender<std::result::Result<(), SoundError>>,
    },
    Stop,
    Shutdown,
}

/// A `PreviewSink` that owns one cpal stream on a dedicated thread, so the
/// platform stream never crosses a thread boundary after creation.
pub struct OutputSink {
    commands: Mutex<Option<Sender<Command>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl OutputSink {
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }

    fn sender(&self) -> std::result::Result<Sender<Command>, SoundError> {
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| SoundError::NoAudioDevice)?;
        if let Some(sender) = commands.as_ref() {
            return Ok(sender.clone());
        }
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("vibecut-sounds-sink".into())
            .spawn(move || run(receiver))
            .map_err(|_| SoundError::NoAudioDevice)?;
        *self.worker.lock().map_err(|_| SoundError::NoAudioDevice)? = Some(worker);
        *commands = Some(sender.clone());
        Ok(sender)
    }
}

impl Default for OutputSink {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewSink for OutputSink {
    fn play(&self, clip: PreviewClip<'_>) -> std::result::Result<(), SoundError> {
        let sender = self.sender()?;
        let (reply, answer) = mpsc::sync_channel(1);
        sender
            .send(Command::Play {
                bytes: clip.bytes.to_vec(),
                reply,
            })
            .map_err(|_| SoundError::NoAudioDevice)?;
        answer.recv().map_err(|_| SoundError::NoAudioDevice)?
    }

    fn stop(&self) -> std::result::Result<(), SoundError> {
        if let Ok(commands) = self.commands.lock()
            && let Some(sender) = commands.as_ref()
        {
            let _ = sender.send(Command::Stop);
        }
        Ok(())
    }
}

impl Drop for OutputSink {
    fn drop(&mut self) {
        if let Ok(commands) = self.commands.get_mut()
            && let Some(sender) = commands.take()
        {
            let _ = sender.send(Command::Shutdown);
        }
        if let Ok(worker) = self.worker.get_mut()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn run(receiver: Receiver<Command>) {
    let mut stream: Option<cpal::Stream> = None;
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Play { bytes, reply } => {
                if let Some(old) = stream.take() {
                    drop(old);
                }
                let result = play_bytes(&bytes, &mut stream);
                let _ = reply.send(result);
            }
            Command::Stop => {
                if let Some(old) = stream.take() {
                    drop(old);
                }
            }
            Command::Shutdown => break,
        }
    }
}

fn play_bytes(
    bytes: &[u8],
    stream: &mut Option<cpal::Stream>,
) -> std::result::Result<(), SoundError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(SoundError::NoAudioDevice)?;
    let supported = device
        .supported_output_configs()
        .map_err(|_| SoundError::NoAudioDevice)?
        .filter(|config| {
            config.channels() == 2 && config.sample_format() == cpal::SampleFormat::F32
        })
        .find_map(|config| config.try_with_sample_rate(cpal::SAMPLE_RATE_48K))
        .or_else(|| {
            device
                .default_output_config()
                .ok()
                .filter(|config| config.channels() == 2)
        })
        .ok_or(SoundError::NoAudioDevice)?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(SoundError::NoAudioDevice);
    }
    let rate = supported.sample_rate();
    let samples = decode(bytes, rate)?;
    let cursor = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_cursor = cursor.clone();
    let output = device
        .build_output_stream(
            supported.config(),
            move |data: &mut [f32], _| {
                let start = callback_cursor.load(std::sync::atomic::Ordering::Acquire);
                for (index, sample) in data.iter_mut().enumerate() {
                    *sample = samples
                        .get(start.saturating_add(index))
                        .copied()
                        .unwrap_or(0.0);
                }
                callback_cursor.store(
                    start.saturating_add(data.len()),
                    std::sync::atomic::Ordering::Release,
                );
            },
            |_| {},
            Some(Duration::from_secs(1)),
        )
        .map_err(|_| SoundError::NoAudioDevice)?;
    output.play().map_err(|_| SoundError::NoAudioDevice)?;
    *stream = Some(output);
    Ok(())
}

/// Decode a preview into interleaved stereo f32 at the device rate. The cache
/// stores complete audio, so this is a pure read: no network or project change.
fn decode(bytes: &[u8], rate: u32) -> std::result::Result<Vec<f32>, SoundError> {
    if bytes.is_empty() {
        return Err(SoundError::InvalidResponse);
    }
    let path = temp_path();
    let result = (|| {
        let mut file = std::fs::File::create(&path).map_err(SoundError::Io)?;
        file.write_all(bytes).map_err(SoundError::Io)?;
        file.sync_all().map_err(SoundError::Io)?;
        let mut reader = PacketReader::open(&path).map_err(SoundError::Media)?;
        let stream = reader
            .probe()
            .streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Audio)
            .cloned()
            .ok_or(SoundError::InvalidResponse)?;
        let mut decoder = AudioDecoder::new(&stream).map_err(SoundError::Media)?;
        let mut resampler: Option<AudioResampler> = None;
        let limit = usize::try_from(rate)
            .unwrap_or(48_000)
            .saturating_mul(2)
            .saturating_mul(MAX_PREVIEW_SECONDS as usize);
        let mut output = Vec::new();
        let mut push = |frame: &media::AudioFrame, output: &mut Vec<f32>| {
            if resampler.is_none() {
                resampler = Some(
                    AudioResampler::new(frame, rate, 2, AudioSampleFormat::PlanarF32)
                        .map_err(SoundError::Media)?,
                );
            }
            let frames = resampler
                .as_mut()
                .ok_or(SoundError::InvalidResponse)?
                .convert(frame)
                .map_err(SoundError::Media)?;
            for frame in frames {
                let planes = frame.planar_f32().map_err(SoundError::Media)?;
                let left = planes.first().map(Vec::as_slice).unwrap_or_default();
                let right = planes.get(1).map(Vec::as_slice).unwrap_or(left);
                for index in 0..left.len().max(right.len()) {
                    if output.len() >= limit {
                        return Err(SoundError::Limit);
                    }
                    output.push(left.get(index).copied().unwrap_or(0.0));
                    output.push(right.get(index).copied().unwrap_or(0.0));
                }
            }
            Ok(())
        };
        let mut eof = false;
        loop {
            while let Some(frame) = decoder.pull_frame().map_err(SoundError::Media)? {
                push(&frame, &mut output)?;
            }
            if eof {
                break;
            }
            match reader.next_packet().map_err(SoundError::Media)? {
                Some(packet) if packet.stream() == stream.index => {
                    decoder.push_packet(&packet).map_err(SoundError::Media)?;
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
            let planes = frame.planar_f32().map_err(SoundError::Media)?;
            let left = planes.first().map(Vec::as_slice).unwrap_or_default();
            let right = planes.get(1).map(Vec::as_slice).unwrap_or(left);
            for index in 0..left.len().max(right.len()) {
                if output.len() >= limit {
                    return Err(SoundError::Limit);
                }
                output.push(left.get(index).copied().unwrap_or(0.0));
                output.push(right.get(index).copied().unwrap_or(0.0));
            }
        }
        if output.is_empty() {
            return Err(SoundError::InvalidResponse);
        }
        Ok(output)
    })();
    let _ = std::fs::remove_file(&path);
    result
}

fn temp_path() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "vibecut-sound-preview-{}-{}.audio",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}
