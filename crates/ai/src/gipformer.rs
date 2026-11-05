//! Local Vietnamese recognition, isolated from native diagnostics and failures.
//! Engine owns model transfers and windows beyond this adapter's invocation bound.
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Operation,
    ProviderConfig, RecognitionSegment, Request, Scope, Stage, Support, Transcriber,
    contracts::checkpoint,
    models::{Model, storage_error, verify_file},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
    time::Duration,
};

mod native;

pub const SAMPLE_RATE: u32 = 16_000;
/// Five-minute safety ceiling. Native decoding uses bounded 20 s chunks.
pub const MAX_SAMPLES: u64 = 300 * SAMPLE_RATE as u64;
const CHUNK_SAMPLES: usize = 20 * SAMPLE_RATE as usize;
const MAX_REPLY: u64 = 16 * 1024 * 1024;
const MAX_HEADER: usize = 16384;
pub const MODEL_ID: &str = "gipformer-68m-rnnt-int8";

/// Explicit hardware request, never an ASR-provider fallback.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Cpu,
    CoreMl,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    directory: PathBuf,
    device: Device,
    threads: u8,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioRequest {
    range: crate::SampleRange,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
enum Reply {
    Ready,
    Recognition(Vec<RecognitionSegment>),
    Unsupported,
    Integrity,
    Failed,
}

/// Reusable model in an isolated runner. One invocation per adapter at a time;
/// engine owns scheduling. Native diagnostics/failures stay outside the app process.
pub struct Gipformer {
    config: ProviderConfig,
    directory: PathBuf,
    device: Device,
    threads: u8,
    languages: Vec<captions::LanguageTag>,
    runner: PathBuf,
    native: Mutex<Pool>,
}
impl Gipformer {
    /// Verify the pinned catalog model before use. The cache directory must remain
    /// immutable. `runner` is the absolute path to the shipped vibecut-local-asr
    /// executable, selected by the composition root, never downloaded as a model.
    /// The released static runtime supports CPU. CoreML is refused with Unsupported
    /// rather than accepting its silent CPU fallback. No model download or provider fallback is performed here.
    pub fn load(
        model: &Model,
        directory: &Path,
        runner: &Path,
        device: Device,
        threads: u8,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        checkpoint(hooks, Stage::Transcribe)?;
        if !runner.is_absolute() || !runner.is_file() {
            return Err(Error::Unsupported);
        }
        validate_options(directory, device, threads)?;
        let pinned = pinned_model()?;
        if *model != pinned {
            return Err(Error::Unsupported);
        }
        verify_model(&pinned, directory, hooks)?;
        let hashes = pinned
            .files
            .iter()
            .map(|f| f.sha256.as_str())
            .collect::<Vec<_>>()
            .join(":");
        let config = ProviderConfig::new("local-gipformer".into(), MODEL_ID.into(),
            format!("sherpa-onnx-2.0.1:{hashes}:greedy:threads{threads}:{device:?}:chunks20s:no-invented-ranges:v1").into(),
            Location::Local { backend: "sherpa-onnx".into(), model_ref: hashes.into() })?;
        let settings = Invocation {
            directory: directory.to_path_buf(),
            device,
            threads,
        };
        let native = Native::start(runner, &settings, hooks)?;
        Ok(Self {
            config,
            directory: directory.to_path_buf(),
            device,
            threads,
            languages: pinned.languages,
            runner: runner.to_path_buf(),
            native: Mutex::new(Pool {
                busy: false,
                idle: Some(native),
            }),
        })
    }
}
impl Transcriber for Gipformer {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: "transcribe".into(),
            operation: Operation::Transcribe,
            languages: self.languages.clone(),
            target_languages: Vec::new(),
            timing: BTreeSet::new(),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: Some(MAX_SAMPLES * 4),
            max_duration_micros: Some(MAX_SAMPLES * 1_000_000 / u64::from(SAMPLE_RATE)),
            evidence: Evidence::Advertised,
            revision: "sherpa-onnx-2.0.1-start-only".into(),
        }
    }
    fn transcribe(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        checkpoint(hooks, Stage::Transcribe)?;
        audio.validate(request.scope)?;
        if request.scope.provider != self.config || audio.sample_rate != SAMPLE_RATE {
            return Err(Error::Unsupported);
        }
        request.prepare(
            &self.capabilities(request.scope),
            &CostEstimate::Free,
            Operation::Transcribe,
        )?;
        if !request.scope.texts.is_empty() {
            return Err(Error::Invalid("transcription text payload"));
        }
        if audio.range.len() > MAX_SAMPLES {
            return Err(Error::Payload);
        }
        let mut lease = Lease::acquire(self)?;
        if lease.native.is_none() {
            lease.native = Some(Native::start(
                &self.runner,
                &Invocation {
                    directory: self.directory.clone(),
                    device: self.device,
                    threads: self.threads,
                },
                hooks,
            )?);
        }
        let native = lease.native.as_mut().ok_or(Error::Inference)?;
        let bytes = native.exchange(&AudioRequest { range: audio.range }, audio.samples, hooks)?;
        native.healthy = false;
        match serde_json::from_slice(&bytes).map_err(|_| Error::Invalid("native response"))? {
            Reply::Recognition(segments) => {
                native.healthy = true;
                Ok(segments)
            }
            Reply::Unsupported => Err(Error::Unsupported),
            Reply::Integrity => Err(Error::ModelIntegrity),
            Reply::Failed | Reply::Ready => Err(Error::Inference),
        }
    }
}

fn pinned_model() -> Result<Model, Error> {
    crate::models::catalog()?
        .into_iter()
        .find(|m| m.id == MODEL_ID)
        .ok_or(Error::Unsupported)
}
fn validate_options(directory: &Path, device: Device, threads: u8) -> Result<(), Error> {
    if !directory.is_absolute()
        || threads == 0
        || threads > 32
        || directory.to_str().is_none_or(|s| s.contains('\0'))
        || matches!(device, Device::CoreMl)
    {
        return Err(Error::Unsupported);
    }
    Ok(())
}
fn verify_model(model: &Model, directory: &Path, hooks: &dyn ExecutionHooks) -> Result<(), Error> {
    for file in &model.files {
        let mut input = std::fs::File::open(directory.join(&file.name)).map_err(storage_error)?;
        verify_file(&mut input, file, hooks)?;
    }
    Ok(())
}

struct Pool {
    busy: bool,
    idle: Option<Native>,
}
struct Lease<'a> {
    owner: &'a Gipformer,
    native: Option<Native>,
}
impl<'a> Lease<'a> {
    fn acquire(owner: &'a Gipformer) -> Result<Self, Error> {
        let mut pool = owner.native.lock().map_err(|_| Error::Inference)?;
        if pool.busy {
            return Err(Error::Unsupported);
        }
        pool.busy = true;
        let native = pool.idle.take();
        Ok(Self { owner, native })
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        // Dispose of failed native work before taking the short pool lock.
        let native = self.native.take().filter(|n| n.healthy);
        if let Ok(mut pool) = self.owner.native.lock() {
            pool.idle = native;
            pool.busy = false;
        }
    }
}
struct Native {
    child: Child,
    input: std::process::ChildStdin,
    output: std::process::ChildStdout,
    healthy: bool,
}
impl Drop for Native {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
// Lives inside the scope body so unwinding kills the child before scope joins IO.
struct StopOnDrop<'a> {
    child: &'a mut Child,
    armed: bool,
}
impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
impl Native {
    fn start(
        runner: &Path,
        settings: &Invocation,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        let mut child = Command::new(runner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::Inference)?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Inference);
        };
        let mut native = Self {
            child,
            input,
            output,
            healthy: false,
        };
        let bytes = native.exchange(settings, &[], hooks)?;
        match serde_json::from_slice(&bytes).map_err(|_| Error::Invalid("native response"))? {
            Reply::Ready => Ok(native),
            Reply::Unsupported => Err(Error::Unsupported),
            Reply::Integrity => Err(Error::ModelIntegrity),
            _ => Err(Error::Inference),
        }
    }
    fn exchange(
        &mut self,
        message: &impl Serialize,
        samples: &[f32],
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<u8>, Error> {
        let header =
            serde_json::to_vec(message).map_err(|_| Error::Invalid("native invocation"))?;
        if header.len() > MAX_HEADER {
            return Err(Error::Payload);
        }
        self.healthy = false;
        let Self {
            child,
            input,
            output,
            healthy: _,
        } = self;
        let result = std::thread::scope(move |scope| {
            let mut stop = StopOnDrop { child, armed: true };
            let writer = scope.spawn(move || -> std::io::Result<()> {
                write_frame(input, &header)?;
                let mut buffer = [0u8; 4096];
                for chunk in samples.chunks(1024) {
                    for (sample, bytes) in chunk.iter().zip(buffer.as_chunks_mut::<4>().0) {
                        bytes.copy_from_slice(&sample.to_le_bytes());
                    }
                    input.write_all(
                        buffer
                            .get(..chunk.len() * 4)
                            .ok_or_else(|| std::io::Error::other("sample bound"))?,
                    )?;
                }
                input.flush()
            });
            let (sender, receiver) = mpsc::sync_channel(1);
            let reader = scope.spawn(move || {
                let _ = sender.send(read_frame(output, MAX_REPLY));
            });
            let bytes = loop {
                if hooks.cancelled() {
                    hooks.cancelling();
                    let _ = stop.child.kill();
                    let _ = stop.child.wait();
                    break Err(Error::Cancelled);
                }
                match receiver.recv_timeout(Duration::from_millis(2)) {
                    Ok(reply) => break reply.and_then(|r| r.ok_or(Error::Inference)),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break Err(Error::Inference),
                }
            };
            if bytes.is_err() {
                let _ = stop.child.kill();
                let _ = stop.child.wait();
            }
            let written = writer.join().map_err(|_| Error::Inference)?;
            reader.join().map_err(|_| Error::Inference)?;
            let bytes = bytes?;
            written.map_err(|_| Error::Inference)?;
            checkpoint(hooks, Stage::Transcribe)?;
            stop.armed = false;
            Ok(bytes)
        });
        self.healthy = result.is_ok();
        result
    }
}
fn write_frame(output: &mut dyn Write, bytes: &[u8]) -> std::io::Result<()> {
    let count = u32::try_from(bytes.len()).map_err(|_| std::io::Error::other("frame bound"))?;
    output.write_all(&count.to_le_bytes())?;
    output.write_all(bytes)?;
    output.flush()
}
fn read_frame(input: &mut dyn Read, limit: u64) -> Result<Option<Vec<u8>>, Error> {
    let mut count = [0u8; 4];
    if input
        .read(count.get_mut(..1).ok_or(Error::Payload)?)
        .map_err(|_| Error::Inference)?
        == 0
    {
        return Ok(None);
    }
    input
        .read_exact(count.get_mut(1..).ok_or(Error::Payload)?)
        .map_err(|_| Error::Invalid("native frame"))?;
    let count = u32::from_le_bytes(count) as u64;
    if count > limit {
        return Err(Error::Payload);
    }
    let mut bytes = vec![0; usize::try_from(count).map_err(|_| Error::Payload)?];
    input
        .read_exact(&mut bytes)
        .map_err(|_| Error::Invalid("native frame"))?;
    Ok(Some(bytes))
}

/// Entry point for the shipped native runner executable. Bounded framed stdin and
/// stdout are a private protocol; the adapter discards native stderr entirely.
#[doc(hidden)]
pub fn run_native() -> Result<(), Error> {
    native::serve(&mut std::io::stdin().lock(), &mut std::io::stdout().lock())
}

#[cfg(all(test, unix))]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "offline process fixture constructors"
)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Temp(PathBuf);
    impl Temp {
        fn new(script: &str) -> (Self, PathBuf) {
            Self::at(
                script,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            )
        }
        fn at(script: &str, stamp: u128) -> (Self, PathBuf) {
            // Clock readings can repeat across parallel tests, even with nanosecond units.
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = loop {
                let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "vibecut asr ế 中-{}-{stamp}-{sequence}",
                    std::process::id()
                ));
                match std::fs::create_dir(&root) {
                    Ok(()) => break root,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("fixture directory: {error}"),
                }
            };
            let runner = root.join("runner");
            std::fs::write(&runner, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o700)).unwrap();
            (Self(root), runner)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn process_fixtures_are_unique_even_with_an_identical_clock_reading() {
        let (first, first_runner) = Temp::at("exit 0", 0);
        let (second, second_runner) = Temp::at("exit 1", 0);
        assert_ne!(first.0, second.0);
        assert_eq!(
            std::fs::read_to_string(&first_runner).unwrap(),
            "#!/bin/sh\nexit 0\n"
        );
        assert_eq!(
            std::fs::read_to_string(&second_runner).unwrap(),
            "#!/bin/sh\nexit 1\n"
        );
    }
    struct Hooks {
        start: std::time::Instant,
        cancel: bool,
        ack: std::sync::atomic::AtomicUsize,
    }
    impl Hooks {
        fn new(cancel: bool) -> Self {
            Self {
                start: std::time::Instant::now(),
                cancel,
                ack: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            self.cancel && self.start.elapsed() > Duration::from_millis(30)
        }
        fn stage(&self, _: Stage) {}
        fn cancelling(&self) {
            self.ack.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn settings() -> Invocation {
        Invocation {
            directory: PathBuf::from("/fixture"),
            device: Device::Cpu,
            threads: 4,
        }
    }
    fn audio() -> AudioRequest {
        AudioRequest {
            range: crate::SampleRange::new(0, MAX_SAMPLES).unwrap(),
        }
    }
    fn emit(reply: &Reply) -> String {
        let bytes = serde_json::to_vec(reply).unwrap();
        let mut framed = Vec::new();
        write_frame(&mut framed, &bytes).unwrap();
        format!(
            "printf '{}'",
            framed
                .iter()
                // POSIX printf reads octal as \0NNN; dash rejects a bare \NNN.
                .map(|b| format!("\\0{b:o}"))
                .collect::<String>()
        )
    }
    #[test]
    fn process_failures_and_malformed_replies_are_typed() {
        for script in ["exit 7".to_owned(), "printf garbage".to_owned()] {
            let (_temp, runner) = Temp::new(&script);
            assert!(Native::start(&runner, &settings(), &Hooks::new(false)).is_err());
        }
        let (_temp, runner) = Temp::new(&format!(
            "{}; {}; exec sleep 30",
            emit(&Reply::Ready),
            emit(&Reply::Recognition(Vec::new()))
        ));
        let mut native = Native::start(&runner, &settings(), &Hooks::new(false)).unwrap();
        let bytes = native
            .exchange(
                &AudioRequest {
                    range: crate::SampleRange::new(0, 1).unwrap(),
                },
                &[0.0],
                &Hooks::new(false),
            )
            .unwrap();
        assert!(
            matches!(serde_json::from_slice::<Reply>(&bytes).unwrap(), Reply::Recognition(v) if v.is_empty())
        );
    }
    #[test]
    fn cancellation_kills_native_work_with_blocked_stdin_and_joins_pipes() {
        let (_temp, runner) = Temp::new(&format!("{}; exec sleep 30", emit(&Reply::Ready)));
        let mut native = Native::start(&runner, &settings(), &Hooks::new(false)).unwrap();
        let hooks = Hooks::new(true);
        assert!(matches!(
            native.exchange(&audio(), &vec![0.0; MAX_SAMPLES as usize], &hooks),
            Err(Error::Cancelled)
        ));
        assert_eq!(hooks.ack.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(hooks.start.elapsed() < Duration::from_secs(2));
        assert!(!native.healthy);
    }
    #[test]
    fn hook_unwind_reaps_native_process_before_joining_blocked_pipes() {
        struct Panic(std::time::Instant);
        impl ExecutionHooks for Panic {
            fn cancelled(&self) -> bool {
                assert!(
                    self.0.elapsed() < Duration::from_millis(30),
                    "synthetic hook failure"
                );
                false
            }
            fn stage(&self, _: Stage) {}
        }
        let (_temp, runner) = Temp::new(&format!("{}; exec sleep 30", emit(&Reply::Ready)));
        let mut native = Native::start(&runner, &settings(), &Hooks::new(false)).unwrap();
        let start = std::time::Instant::now();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| native.exchange(
                &audio(),
                &vec![0.0; MAX_SAMPLES as usize],
                &Panic(start)
            )))
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!native.healthy);
    }
}
