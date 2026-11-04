//! Explicit local benchmark: MODEL-DIRECTORY RUNNER cpu|coreml THREADS AUDIO-F32LE...
//! Weights/audio stay local. Recognition JSONL on stdout is requested output.
#[cfg(feature = "gipformer")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use ai::{
        AudioWindow, Error, ExecutionHooks, Operation, Request, SampleRange, Scope, Stage,
        Transcriber,
        gipformer::{Device, Gipformer, MAX_SAMPLES, SAMPLE_RATE},
    };
    use std::{io::Read, path::Path};
    struct Hooks;
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn stage(&self, _: Stage) {}
    }
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let directory = args.first().ok_or("missing model directory")?;
    let runner = args.get(1).ok_or("missing native runner")?;
    let device = match args.get(2).and_then(|s| s.to_str()) {
        Some("cpu") => Device::Cpu,
        Some("coreml") => Device::CoreMl,
        _ => return Err("specify cpu or coreml".into()),
    };
    let threads: u8 = args
        .get(3)
        .and_then(|s| s.to_str())
        .ok_or("missing thread count")?
        .parse()?;
    let inputs = args
        .get(4..)
        .filter(|inputs| !inputs.is_empty())
        .ok_or("missing mono 16k f32le audio")?;
    let language = captions::LanguageTag::parse("vi")?;
    let model = ai::models::suggested_transcriber(&language)?;
    let load = std::time::Instant::now();
    let adapter = Gipformer::load(
        &model,
        Path::new(directory),
        Path::new(runner),
        device,
        threads,
        &Hooks,
    )?;
    eprintln!("load_seconds={:.6}", load.elapsed().as_secs_f64());
    for (index, input) in inputs.iter().enumerate() {
        let mut bytes = Vec::new();
        std::fs::File::open(input)?
            .take(MAX_SAMPLES * 4 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_SAMPLES * 4 || bytes.len() % 4 != 0 {
            return Err("empty, truncated or excessive audio".into());
        }
        let pcm = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect::<Vec<_>>();
        let range = SampleRange::new(160000, 160000 + pcm.len() as u64)?;
        let scope = Scope {
            provider: adapter.config().clone(),
            operation: Operation::Transcribe,
            payload_fingerprint: format!("explicit-local-benchmark:{index}").into(),
            mode: "transcribe".into(),
            texts: Vec::new(),
            source_language: language.clone(),
            target_language: None,
            source_samples: range,
            sample_rate: SAMPLE_RATE,
            max_bytes: bytes.len() as u64,
        };
        let start = std::time::Instant::now();
        let result = ai::transcribe(
            &adapter,
            AudioWindow {
                samples: &pcm,
                range,
                sample_rate: SAMPLE_RATE,
            },
            &Request {
                scope: &scope,
                consent: None,
            },
            None,
            &Hooks,
        )?;
        if !result.segments().is_empty()
            && !matches!(result.to_transcript(), Err(Error::TimingUnavailable))
        {
            return Err("unexpected native timing availability".into());
        }
        let elapsed = start.elapsed().as_secs_f64();
        println!(
            "{}",
            serde_json::json!({ "index": index, "samples": pcm.len(), "inference_seconds": elapsed, "segments": result.segments() })
        );
    }
    Ok(())
}
#[cfg(not(feature = "gipformer"))]
fn main() {}
