//! Offline SFace smoke/latency and pair-score harness. Inputs are local prepared
//! little-endian f32 CHW tensors; output contains only indices, scores and timing.
use ai::{
    ExecutionHooks,
    vision::{IdentityFrame, TractFaceEmbedding},
};
use std::{path::PathBuf, time::Instant};
struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let weights = PathBuf::from(args.next().ok_or("weights directory required")?);
    let input = PathBuf::from(args.next().ok_or("tensor directory required")?);
    let destination = PathBuf::from(args.next().ok_or("output JSON required")?);
    let mut paths: Vec<_> = std::fs::read_dir(input)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    paths.sort();
    let started = Instant::now();
    let mut model = TractFaceEmbedding::load(&weights, 1, &Hooks)?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.;
    let mut embeddings = Vec::new();
    let mut times = Vec::new();
    for path in paths {
        let bytes = std::fs::read(&path)?;
        if bytes.len() != 3 * 112 * 112 * 4 {
            return Err("invalid tensor byte length".into());
        }
        let values: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                let mut raw = [0; 4];
                raw.copy_from_slice(b);
                f32::from_le_bytes(raw)
            })
            .collect();
        let frame = IdentityFrame::new(&values)?;
        let started = Instant::now();
        let embedding = model.embed(frame, &Hooks)?;
        times.push(started.elapsed().as_secs_f64() * 1000.);
        embeddings.push((
            path.file_stem()
                .ok_or("tensor name")?
                .to_string_lossy()
                .into_owned(),
            embedding,
        ));
    }
    let pairs: Vec<_> = embeddings
        .iter()
        .enumerate()
        .flat_map(|(i, (a, x))| {
            embeddings
                .iter()
                .skip(i + 1)
                .map(move |(b, y)| serde_json::json!({"a":a,"b":b,"score":x.similarity(y)}))
        })
        .collect();
    std::fs::write(
        destination,
        serde_json::to_vec_pretty(
            &serde_json::json!({"load_ms":load_ms,"inference_ms":times,"pairs":pairs}),
        )?,
    )?;
    println!("faces={} load_ms={load_ms:.2}", embeddings.len());
    Ok(())
}
