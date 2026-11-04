//! Opt-in local smoke/benchmark: MODEL PATH-TO-MONO-16KHZ-F32LE LANGUAGE [cpu].
//! Inputs stay local; model must already match the pinned catalog. JSON goes to stdout.
#[cfg(feature = "whisper")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use ai::{
        AudioWindow, ExecutionHooks, Operation, Request, SampleRange, Scope, Stage, Transcriber,
        whisper::{Device, MAX_SAMPLES, SAMPLE_RATE, Whisper},
    };
    use std::{io::Read, path::Path};
    struct Hooks;
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn stage(&self, _: Stage) {}
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let model_path = args.first().ok_or("missing model")?;
    let audio_path = args.get(1).ok_or("missing mono 16k f32le audio")?;
    let language = args
        .get(2)
        .and_then(|s| s.to_str())
        .ok_or("missing language")?;
    let device = if args.get(3).is_some_and(|s| s == "cpu") {
        Device::Cpu
    } else {
        Device::Gpu
    };
    let model = ai::models::catalog()?.remove(0);
    let load = std::time::Instant::now();
    let adapter = Whisper::load(&model, Path::new(model_path), device, 4, &Hooks)?;
    eprintln!("load_seconds={:.3}", load.elapsed().as_secs_f64());
    let mut input = std::fs::File::open(audio_path)?;
    let mut offset = 160_000u64; // Exercise nonzero source coordinates.
    let mut bytes = vec![0u8; usize::try_from(MAX_SAMPLES * 4)?];
    let start = std::time::Instant::now();
    loop {
        let mut count = 0;
        while count < bytes.len() {
            let n = input.read(bytes.get_mut(count..).ok_or("input bound")?)?;
            if n == 0 {
                break;
            }
            count += n;
        }
        if count == 0 {
            break;
        }
        if count % 4 != 0 {
            return Err("truncated float sample".into());
        }
        let pcm = bytes
            .get(..count)
            .ok_or("input bound")?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect::<Vec<_>>();
        let range = SampleRange::new(
            offset,
            offset
                .checked_add(u64::try_from(pcm.len())?)
                .ok_or("offset")?,
        )?;
        let scope = Scope {
            provider: adapter.config().clone(),
            operation: Operation::Transcribe,
            payload_fingerprint: "synthetic-benchmark-caller".into(),
            mode: "transcribe".into(),
            texts: Vec::new(),
            source_language: captions::LanguageTag::parse(language)?,
            target_language: None,
            source_samples: range,
            sample_rate: SAMPLE_RATE,
            max_bytes: u64::try_from(count)?,
        };
        let request = Request {
            scope: &scope,
            consent: None,
        };
        let audio = AudioWindow {
            samples: &pcm,
            range,
            sample_rate: SAMPLE_RATE,
        };
        let recognition = ai::transcribe(&adapter, audio, &request, None, &Hooks)?;
        recognition.to_transcript()?; // Shared conversion must accept native segment timings.
        println!("{}", serde_json::to_string(recognition.segments())?);
        offset = range.end();
    }
    eprintln!(
        "inference_seconds={:.3} audio_seconds={:.3}",
        start.elapsed().as_secs_f64(),
        (offset - 160_000) as f64 / f64::from(SAMPLE_RATE)
    );
    Ok(())
}
#[cfg(not(feature = "whisper"))]
fn main() {}
