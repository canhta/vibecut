use ai::{
    Aligner, AudioWindow, ExecutionHooks, Operation, RecognitionSegment, Request, SampleRange,
    Scope, Stage, alignment::CandleAligner, models,
};
use captions::LanguageTag;
use std::{io::Read, path::PathBuf, time::Instant};
struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 6 {
        return Err(
            "expected MODEL_DIR PCM_F32LE LANGUAGE SEGMENTS_JSON OUTPUT_JSON SOURCE_OFFSET_SAMPLES"
                .into(),
        );
    }
    let get = |i| args.get(i).ok_or("missing argument");
    let language = LanguageTag::parse(get(2)?.to_str().ok_or("invalid language")?)?;
    let offset: u64 = get(5)?.to_str().ok_or("invalid offset")?.parse()?;
    let mut bytes = Vec::new();
    std::fs::File::open(PathBuf::from(get(1)?))?
        .take(480000 * 4 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() % 4 != 0 || bytes.len() > 480000 * 4 {
        return Err("invalid PCM size".into());
    }
    let samples: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let mut text = Vec::new();
    std::fs::File::open(PathBuf::from(get(3)?))?
        .take(65537)
        .read_to_end(&mut text)?;
    if text.len() > 65536 {
        return Err("invalid segment file".into());
    }
    let segments: Vec<RecognitionSegment> =
        serde_json::from_slice(&text).map_err(|_| "invalid segment JSON")?;
    let loaded = Instant::now();
    let aligner =
        CandleAligner::load(&models::alignment_model()?, &PathBuf::from(get(0)?), &Hooks)?;
    let load_ms = loaded.elapsed().as_millis();
    let range = SampleRange::new(
        offset,
        offset
            .checked_add(samples.len() as u64)
            .ok_or("offset overflow")?,
    )?;
    let scope = Scope {
        provider: aligner.config().clone(),
        operation: Operation::Align,
        payload_fingerprint: "local-evaluation".into(),
        mode: ai::alignment::MODE.into(),
        texts: segments.iter().map(|s| s.text.clone()).collect(),
        source_language: language,
        target_language: None,
        source_samples: range,
        sample_rate: 16000,
        max_bytes: samples.len() as u64 * 4,
    };
    let start = Instant::now();
    let result = aligner.align(
        AudioWindow {
            samples: &samples,
            range,
            sample_rate: 16000,
        },
        &segments,
        &Request {
            scope: &scope,
            consent: None,
        },
        &Hooks,
    )?;
    let output = serde_json::json!({"load_ms":load_ms,"inference_ms":start.elapsed().as_millis(),"source_offset_samples":offset,"sample_count":samples.len(),"segments":result});
    std::fs::write(PathBuf::from(get(4)?), serde_json::to_vec_pretty(&output)?)?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("alignment failed: {e}");
        std::process::exit(1);
    }
}
