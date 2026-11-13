//! The cpal input host, the one native microphone boundary.
//!
//! The stream is built, played and dropped on one capture thread, so the native handle
//! never crosses threads. Opening waits for that thread and surfaces the operating
//! system's permission and device refusals as typed failures.

use super::{Failure, InputHost, Opened, Sink};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread::JoinHandle,
    time::Duration,
};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// The operating system's default audio host.
pub struct CpalHost;

fn open_failure(error: &cpal::Error) -> Failure {
    match error.kind() {
        cpal::ErrorKind::PermissionDenied => Failure::PermissionDenied,
        cpal::ErrorKind::UnsupportedConfig | cpal::ErrorKind::UnsupportedOperation => {
            Failure::Unsupported
        }
        cpal::ErrorKind::DeviceBusy => Failure::Unavailable,
        _ => Failure::NoDevice,
    }
}

/// `None` for notices that leave the take intact (a rerouted default device).
fn stream_failure(error: &cpal::Error) -> Option<Failure> {
    match error.kind() {
        cpal::ErrorKind::PermissionDenied => Some(Failure::PermissionDenied),
        cpal::ErrorKind::DeviceNotAvailable
        | cpal::ErrorKind::DeviceBusy
        | cpal::ErrorKind::HostUnavailable
        | cpal::ErrorKind::StreamInvalidated => Some(Failure::Unavailable),
        _ => None,
    }
}

fn name(device: &cpal::Device) -> Option<String> {
    device
        .description()
        .ok()
        .map(|description| description.name().to_owned())
}

impl InputHost for CpalHost {
    fn devices(&self) -> Result<Vec<String>, Failure> {
        let host = cpal::default_host();
        let default = host.default_input_device().and_then(|device| name(&device));
        let mut names: Vec<String> = host
            .input_devices()
            .map_err(|error| open_failure(&error))?
            .filter_map(|device| name(&device))
            .collect();
        if let Some(default) = default {
            names.retain(|candidate| candidate != &default);
            names.insert(0, default);
        }
        names.dedup();
        Ok(names)
    }

    fn open(&self, device: Option<&str>, sink: Sink) -> Result<Opened, Failure> {
        let wanted = device.map(str::to_owned);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("voiceover-capture".into())
            .spawn(move || run(wanted.as_deref(), sink, &ready_tx, &stop_rx))
            .map_err(|_| Failure::Unavailable)?;
        match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok((sample_rate, channels))) => Ok(Opened {
                sample_rate,
                channels,
                capture: Box::new(Capture {
                    stop: Some(stop_tx),
                    thread: Some(thread),
                }),
            }),
            Ok(Err(failure)) => {
                let _ = thread.join();
                Err(failure)
            }
            // The thread sees the closed stop channel when it finishes and exits.
            Err(_) => Err(Failure::Unavailable),
        }
    }
}

type Ready = SyncSender<Result<(u32, u16), Failure>>;

fn run(wanted: Option<&str>, sink: Sink, ready: &Ready, stop: &Receiver<()>) {
    match build(wanted, sink) {
        Ok((stream, sample_rate, channels)) => {
            let _ = ready.send(Ok((sample_rate, channels)));
            // A message or a closed channel both mean stop.
            let _ = stop.recv();
            drop(stream);
        }
        Err(failure) => {
            let _ = ready.send(Err(failure));
        }
    }
}

fn build(wanted: Option<&str>, sink: Sink) -> Result<(cpal::Stream, u32, u16), Failure> {
    let host = cpal::default_host();
    let device = match wanted {
        None => host.default_input_device(),
        Some(wanted) => host
            .input_devices()
            .map_err(|error| open_failure(&error))?
            .find(|device| name(device).is_some_and(|name| name == wanted)),
    }
    .ok_or(Failure::NoDevice)?;
    let supported = device
        .supported_input_configs()
        .map_err(|error| open_failure(&error))?
        .filter(|config| FORMATS.contains(&config.sample_format()))
        .filter_map(|config| config.try_with_sample_rate(cpal::SAMPLE_RATE_48K))
        .min_by_key(|config| FORMATS.iter().position(|f| *f == config.sample_format()))
        .or_else(|| {
            device
                .default_input_config()
                .ok()
                .filter(|config| FORMATS.contains(&config.sample_format()))
        })
        .ok_or(Failure::Unsupported)?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate();
    let config = supported.config();
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => stream(&device, config, sink, move |sink, data: &[f32]| {
            sink.push(data, channels)
        }),
        cpal::SampleFormat::I16 => stream(&device, config, sink, move |sink, data: &[i16]| {
            sink.push_i16(data, channels)
        }),
        cpal::SampleFormat::U16 => stream(&device, config, sink, move |sink, data: &[u16]| {
            sink.push_u16(data, channels)
        }),
        cpal::SampleFormat::I32 => stream(&device, config, sink, move |sink, data: &[i32]| {
            sink.push_i32(data, channels)
        }),
        _ => Err(Failure::Unsupported),
    }?;
    stream.play().map_err(|error| open_failure(&error))?;
    Ok((stream, sample_rate, channels))
}

/// Sample formats read, best first; each converts to f32 inside the callback.
const FORMATS: [cpal::SampleFormat; 4] = [
    cpal::SampleFormat::F32,
    cpal::SampleFormat::I16,
    cpal::SampleFormat::I32,
    cpal::SampleFormat::U16,
];

fn stream<T: cpal::SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sink: Sink,
    mut push: impl FnMut(&Sink, &[T]) + Send + 'static,
) -> Result<cpal::Stream, Failure> {
    let failing = sink.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| push(&sink, data),
            move |error: cpal::Error| {
                if let Some(failure) = stream_failure(&error) {
                    failing.fail(failure);
                }
            },
            Some(OPEN_TIMEOUT),
        )
        .map_err(|error| open_failure(&error))
}

/// Stops the capture thread (which drops the stream) when dropped.
struct Capture {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
