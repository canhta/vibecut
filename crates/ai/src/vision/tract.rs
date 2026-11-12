use super::{
    FaceBox, FaceEmbedding, FaceFrame, IdentityFrame, PROMPTS, RECIPE, SceneFrame, SceneResult,
    SpeakerChunk, VisionDetector, VisionFingerprint, classify_embedding, decisions::decode_faces,
    finite,
};
use crate::{Error, ExecutionHooks, Stage, contracts::checkpoint, models};
use prost::Message;
use std::{collections::VecDeque, fs::File, path::Path, sync::Arc};
use tract_linalg::multithread::Executor;
use tract_onnx::{
    pb,
    prelude::{
        Datum, Framework, InferenceFact, InferenceModelExt, TValue, Tensor, TypedRunnableModel,
    },
    tract_core::runtime::RunOptions,
};

pub const RUNTIME: &str = "tract-0.23.9";
/// Existing verified model directories; never downloaded implicitly. The caller
/// may release model file leases once load returns: all verified bytes are owned.
/// Light-ASD requires the six original dynamic files named in the ai README.
pub struct VisionPaths<'a> {
    pub yunet: &'a Path,
    pub clip: &'a Path,
    pub light_asd: &'a Path,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Face,
    Identity,
    Image,
    Text,
    Audio,
    Visual,
    Backend,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlanKey {
    hash: String,
    frames: usize,
    dims: Vec<Vec<usize>>,
    runtime: &'static str,
    threads: usize,
}
struct Graph {
    proto: pb::ModelProto,
    hash: String,
    kind: Kind,
}
/// One canonical, pinned local bundle with an LRU of 3..=12 concrete plans.
/// Threads are 1..=4 in a private executor; no global runtime setting is changed.
/// Loading verifies all file bytes before parsing; malformed/replaced files fail
/// integrity checks before tract can inspect tensor dimensions or allocate weights.
pub struct TractVision {
    graphs: Vec<Graph>,
    plans: VecDeque<(PlanKey, Arc<TypedRunnableModel>)>,
    capacity: usize,
    threads: usize,
    executor: Executor,
    prompts: [Vec<f32>; 2],
    reidentification: Option<TractFaceEmbedding>,
}
impl TractVision {
    pub fn load(
        paths: VisionPaths<'_>,
        threads: usize,
        capacity: usize,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        checkpoint(hooks, Stage::Vision)?;
        if !(1..=4).contains(&threads) || !(3..=12).contains(&capacity) {
            return Err(Error::VisionShape);
        }
        let executor = if threads == 1 {
            Executor::SingleThread
        } else {
            Executor::MultiThread(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|i| format!("vision-{i}"))
                    .build()
                    .map_err(|_| Error::VisionLoad)?,
            ))
        };
        let catalog = models::catalog()?;
        let face = catalog
            .iter()
            .find(|m| m.id == "yunet-2026may")
            .ok_or(Error::VisionLoad)?;
        let clip = catalog
            .iter()
            .find(|m| m.id == "clip-vit-base-patch32")
            .ok_or(Error::VisionLoad)?;
        let mut graphs = Vec::new();
        for (model, dir, name, kind) in [
            (
                face,
                paths.yunet,
                "face_detection_yunet_2026may.onnx",
                Kind::Face,
            ),
            (clip, paths.clip, "vision_model.onnx", Kind::Image),
        ] {
            let file = model
                .files
                .iter()
                .find(|f| f.name == name)
                .ok_or(Error::VisionLoad)?;
            graphs.push(Graph::load(
                dir,
                name,
                file.size,
                &file.sha256,
                kind,
                None,
                hooks,
            )?);
        }
        for (name, size, hash, data_size, data_hash, kind) in ASD_FILES {
            let data = read(
                paths.light_asd,
                &format!("{name}.data"),
                data_size,
                data_hash,
                hooks,
            )?;
            graphs.push(Graph::load(
                paths.light_asd,
                name,
                size,
                hash,
                kind,
                Some(&data),
                hooks,
            )?);
        }
        let mut this = Self {
            graphs,
            plans: VecDeque::new(),
            capacity,
            threads,
            executor,
            prompts: [Vec::new(), Vec::new()],
            reidentification: None,
        };
        // Fixed prompts are derived once per loaded bundle, then the text graph
        // and its plan are dropped. They never execute for a frame.
        let tokenizer_file = clip
            .files
            .iter()
            .find(|f| f.name == "tokenizer.json")
            .ok_or(Error::VisionLoad)?;
        let bytes = read(
            paths.clip,
            &tokenizer_file.name,
            tokenizer_file.size,
            &tokenizer_file.sha256,
            hooks,
        )?;
        let tokenizer = tokenizers::Tokenizer::from_bytes(bytes).map_err(|_| Error::VisionLoad)?;
        let mut ids = Vec::with_capacity(154);
        for prompt in PROMPTS {
            let encoding = tokenizer
                .encode(prompt, true)
                .map_err(|_| Error::VisionLoad)?;
            let mut row: Vec<i64> = encoding
                .get_ids()
                .iter()
                .take(77)
                .map(|v| i64::from(*v))
                .collect();
            row.resize(77, 49407);
            ids.extend(row);
        }
        let file = clip
            .files
            .iter()
            .find(|f| f.name == "text_model.onnx")
            .ok_or(Error::VisionLoad)?;
        let text = Graph::load(
            paths.clip,
            &file.name,
            file.size,
            &file.sha256,
            Kind::Text,
            None,
            hooks,
        )?;
        let plan = this.compile(&text, &[vec![2, 77]], hooks)?;
        let input = Tensor::from_shape(&[2, 77], &ids).map_err(|_| Error::VisionShape)?;
        let values = run(&plan, vec![input.into()], hooks)?;
        let output = output(&values, 0, &[2, 512])?;
        for (dest, row) in this
            .prompts
            .iter_mut()
            .zip(output.as_chunks::<512>().0.iter())
        {
            *dest = row.to_vec();
        }
        // Reject a degenerate prompt tower now instead of on the first frame.
        classify_embedding(
            this.prompts.first().ok_or(Error::VisionLoad)?,
            [
                this.prompts.first().ok_or(Error::VisionLoad)?,
                this.prompts.get(1).ok_or(Error::VisionLoad)?,
            ],
        )?;
        checkpoint(hooks, Stage::Vision)?;
        Ok(this)
    }
    /// Loads only the pinned YuNet face graph. Shot analysis needs the full
    /// bundle; auto reframe calls only `faces`, so this keeps a face-only
    /// composition small. `speaker` and `scene` refuse without their graphs.
    pub fn load_faces(
        yunet: &Path,
        threads: usize,
        capacity: usize,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        checkpoint(hooks, Stage::Vision)?;
        if !(1..=4).contains(&threads) || !(3..=12).contains(&capacity) {
            return Err(Error::VisionShape);
        }
        let executor = if threads == 1 {
            Executor::SingleThread
        } else {
            Executor::MultiThread(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|i| format!("vision-{i}"))
                    .build()
                    .map_err(|_| Error::VisionLoad)?,
            ))
        };
        let catalog = models::catalog()?;
        let face = catalog
            .iter()
            .find(|m| m.id == "yunet-2026may")
            .ok_or(Error::VisionLoad)?;
        let file = face
            .files
            .iter()
            .find(|f| f.name == "face_detection_yunet_2026may.onnx")
            .ok_or(Error::VisionLoad)?;
        let face = Graph::load(
            yunet,
            &file.name,
            file.size,
            &file.sha256,
            Kind::Face,
            None,
            hooks,
        )?;
        checkpoint(hooks, Stage::Vision)?;
        Ok(Self {
            graphs: vec![face],
            plans: VecDeque::new(),
            capacity,
            threads,
            executor,
            prompts: [Vec::new(), Vec::new()],
            reidentification: None,
        })
    }
    /// Content identity includes every graph/external-data hash, tokenizer and
    /// exact prompts, recipe, runtime and thread setting. No paths or pixels.
    pub fn fingerprint(&self) -> Result<VisionFingerprint, Error> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for s in self
            .graphs
            .iter()
            .map(|g| g.hash.as_str())
            .chain([RECIPE, RUNTIME])
            .chain(PROMPTS)
        {
            hash.update((s.len() as u64).to_le_bytes());
            hash.update(s.as_bytes());
        }
        // The pinned catalog owns text/tokenizer identities even after their
        // in-memory prompt derivation graph is released.
        for m in models::catalog()?
            .iter()
            .filter(|m| m.id == "clip-vit-base-patch32")
        {
            for f in &m.files {
                hash.update(f.sha256.as_bytes());
            }
        }
        if let Some(embedding) = &self.reidentification {
            hash.update(embedding.fingerprint().as_bytes());
        }
        hash.update((self.threads as u64).to_le_bytes());
        Ok(VisionFingerprint(hash.finalize().into()))
    }
    /// Explicitly attach an already downloaded, verified embedding model.
    pub fn with_reidentification(mut self, embedding: TractFaceEmbedding) -> Self {
        self.reidentification = Some(embedding);
        self
    }
    pub fn cached_plans(&self) -> usize {
        self.plans.len()
    }
    fn compile(
        &self,
        graph: &Graph,
        dims: &[Vec<usize>],
        hooks: &dyn ExecutionHooks,
    ) -> Result<Arc<TypedRunnableModel>, Error> {
        compile_graph(graph, dims, &self.executor, hooks)
    }
    fn plan(
        &mut self,
        kind: Kind,
        frames: usize,
        dims: Vec<Vec<usize>>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Arc<TypedRunnableModel>, Error> {
        checkpoint(hooks, Stage::Vision)?;
        let graph = self
            .graphs
            .iter()
            .find(|g| g.kind == kind)
            .ok_or(Error::VisionLoad)?;
        let key = PlanKey {
            hash: graph.hash.clone(),
            frames,
            dims,
            runtime: RUNTIME,
            threads: self.threads,
        };
        if let Some(i) = self.plans.iter().position(|(k, _)| k == &key) {
            let item = self.plans.remove(i).ok_or(Error::VisionLoad)?;
            let plan = item.1.clone();
            self.plans.push_back(item);
            return Ok(plan);
        }
        if self.plans.len() == self.capacity {
            self.plans.pop_front();
        }
        let plan = self.compile(graph, &key.dims, hooks)?;
        self.plans.push_back((key, plan.clone()));
        Ok(plan)
    }
}
impl VisionDetector for TractVision {
    fn embedding(
        &mut self,
        frame: IdentityFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<FaceEmbedding>, Error> {
        self.reidentification
            .as_mut()
            .map(|e| e.embed(frame, hooks))
            .transpose()
    }
    fn faces(
        &mut self,
        frame: FaceFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<FaceBox>, Error> {
        let plan = self.plan(Kind::Face, 1, vec![vec![1, 3, 384, 640]], hooks)?;
        let input = tensor(&[1, 3, 384, 640], frame.0)?;
        let values = run(&plan, vec![input], hooks)?;
        if values.len() != 12 {
            return Err(Error::VisionShape);
        }
        let mut outputs = Vec::new();
        for i in 0..12 {
            let stride = [8usize, 16, 32]
                .get(i % 3)
                .copied()
                .ok_or(Error::VisionShape)?;
            let width = match i / 3 {
                0 | 1 => 1,
                2 => 4,
                _ => 10,
            };
            outputs.push(output(
                &values,
                i,
                &[1, (640 / stride) * (384 / stride), width],
            )?);
        }
        let result = decode_faces(&outputs)?;
        checkpoint(hooks, Stage::Vision)?;
        Ok(result)
    }
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<f32>, Error> {
        let n = chunk.frames;
        let a = self.plan(Kind::Audio, n, vec![vec![1, n * 4, 13]], hooks)?;
        let audio = run(&a, vec![tensor(&[1, n * 4, 13], chunk.audio)?], hooks)?;
        let ea = output(&audio, 0, &[1, n, 128])?;
        drop(a);
        let v = self.plan(Kind::Visual, n, vec![vec![1, n, 112, 112]], hooks)?;
        let visual = run(&v, vec![tensor(&[1, n, 112, 112], chunk.video)?], hooks)?;
        let ev = output(&visual, 0, &[1, n, 128])?;
        drop(v);
        let b = self.plan(
            Kind::Backend,
            n,
            vec![vec![1, n, 128], vec![1, n, 128]],
            hooks,
        )?;
        let logits = run(
            &b,
            vec![tensor(&[1, n, 128], &ea)?, tensor(&[1, n, 128], &ev)?],
            hooks,
        )?;
        let logits = output(&logits, 0, &[n, 2])?;
        let result = logits
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let [a, b] = *pair;
                1. / (1. + (a - b).exp())
            })
            .collect();
        checkpoint(hooks, Stage::Vision)?;
        Ok(result)
    }
    fn scene(
        &mut self,
        frame: SceneFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<SceneResult, Error> {
        let plan = self.plan(Kind::Image, 1, vec![vec![1, 3, 224, 224]], hooks)?;
        let values = run(&plan, vec![tensor(&[1, 3, 224, 224], frame.0)?], hooks)?;
        let image = output(&values, 0, &[1, 512])?;
        let result = classify_embedding(
            &image,
            [
                self.prompts.first().ok_or(Error::VisionLoad)?,
                self.prompts.get(1).ok_or(Error::VisionLoad)?,
            ],
        )?;
        checkpoint(hooks, Stage::Vision)?;
        Ok(result)
    }
}
/// Integrity-verified, fixed-shape CPU SFace adapter. One plan, no runtime download.
pub struct TractFaceEmbedding {
    plan: Arc<TypedRunnableModel>,
    fingerprint: VisionFingerprint,
}
impl TractFaceEmbedding {
    pub fn load(dir: &Path, threads: usize, hooks: &dyn ExecutionHooks) -> Result<Self, Error> {
        use sha2::{Digest, Sha256};
        checkpoint(hooks, Stage::Vision)?;
        if !(1..=4).contains(&threads) {
            return Err(Error::VisionShape);
        }
        let executor = if threads == 1 {
            Executor::SingleThread
        } else {
            Executor::MultiThread(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|i| format!("face-embedding-{i}"))
                    .build()
                    .map_err(|_| Error::VisionLoad)?,
            ))
        };
        let catalog = models::catalog()?;
        let file = catalog
            .iter()
            .find(|m| m.id == "sface-2021dec")
            .and_then(|m| m.files.first())
            .ok_or(Error::VisionLoad)?;
        let graph = Graph::load(
            dir,
            &file.name,
            file.size,
            &file.sha256,
            Kind::Identity,
            None,
            hooks,
        )?;
        let plan = compile_graph(&graph, &[vec![1, 3, 112, 112]], &executor, hooks)?;
        let mut hash = Sha256::new();
        hash.update(file.sha256.as_bytes());
        hash.update(
            b"sface-v1:five-point-similarity-eye8:bilinear-round-black:rgb112-raw:cosine128",
        );
        hash.update(RUNTIME.as_bytes());
        hash.update((threads as u64).to_le_bytes());
        Ok(Self {
            plan,
            fingerprint: VisionFingerprint(hash.finalize().into()),
        })
    }
    pub fn fingerprint(&self) -> VisionFingerprint {
        self.fingerprint
    }
    pub fn embed(
        &mut self,
        frame: IdentityFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<FaceEmbedding, Error> {
        let values = run(
            &self.plan,
            vec![tensor(&[1, 3, 112, 112], frame.values())?],
            hooks,
        )?;
        let embedding = FaceEmbedding::new(&output(&values, 0, &[1, 128])?)?;
        checkpoint(hooks, Stage::Vision)?;
        Ok(embedding)
    }
}
fn compile_graph(
    graph: &Graph,
    dims: &[Vec<usize>],
    executor: &Executor,
    hooks: &dyn ExecutionHooks,
) -> Result<Arc<TypedRunnableModel>, Error> {
    checkpoint(hooks, Stage::Vision)?;
    let result = guarded(
        || {
            // Only ASD graphs contain stale dynamic exporter annotations. The
            // cloned metadata is specialized without rewriting any operator.
            let mut specialized;
            let proto = if matches!(graph.kind, Kind::Audio | Kind::Visual | Kind::Backend) {
                specialized = graph.proto.clone();
                specialize(&mut specialized, graph.kind, dims)?;
                &specialized
            } else {
                &graph.proto
            };
            let mut model = tract_onnx::onnx()
                .model_for_proto_model(proto)
                .map_err(|_| Error::VisionLoad)?;
            for (i, shape) in dims.iter().enumerate() {
                let dt = if graph.kind == Kind::Text {
                    i64::datum_type()
                } else {
                    f32::datum_type()
                };
                model
                    .set_input_fact(i, InferenceFact::dt_shape(dt, shape.clone()))
                    .map_err(|_| Error::VisionShape)?;
            }
            let model = model.into_optimized().map_err(|_| Error::VisionLoad)?;
            TypedRunnableModel::new_with_options(
                model,
                &RunOptions {
                    executor: Some(executor.clone()),
                    ..Default::default()
                },
            )
            .map_err(|_| Error::VisionLoad)
        },
        Error::VisionLoad,
    );
    checkpoint(hooks, Stage::Vision)?;
    result
}
fn read(
    dir: &Path,
    name: &str,
    size: u64,
    hash: &str,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<u8>, Error> {
    checkpoint(hooks, Stage::Vision)?;
    let mut file = File::open(dir.join(name)).map_err(models::storage_error)?;
    models::read_verified(&mut file, size, hash, hooks)
}
impl Graph {
    #[allow(clippy::too_many_arguments)]
    fn load(
        dir: &Path,
        name: &str,
        size: u64,
        hash: &str,
        kind: Kind,
        data: Option<&[u8]>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        let bytes = read(dir, name, size, hash, hooks)?;
        let mut proto = guarded(
            || {
                // The framework's Read adapter first copies the whole model.
                pb::ModelProto::decode(bytes.as_slice()).map_err(|_| Error::VisionLoad)
            },
            Error::VisionLoad,
        )?;
        let graph = proto.graph.as_mut().ok_or(Error::VisionLoad)?;
        // Inline only pinned external bytes, never ask ONNX to resolve a path.
        for tensor in &mut graph.initializer {
            if tensor.data_location == Some(1) {
                let data = data.ok_or(Error::VisionLoad)?;
                let value = |key: &str| {
                    tensor
                        .external_data
                        .iter()
                        .find(|v| v.key == key)
                        .map(|v| v.value.as_str())
                };
                if value("location") != Some(format!("{name}.data").as_str()) {
                    return Err(Error::VisionLoad);
                }
                let offset: usize = value("offset")
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| Error::VisionLoad)?;
                let length: usize = value("length")
                    .ok_or(Error::VisionLoad)?
                    .parse()
                    .map_err(|_| Error::VisionLoad)?;
                let end = offset.checked_add(length).ok_or(Error::VisionLoad)?;
                tensor.raw_data = data.get(offset..end).ok_or(Error::VisionLoad)?.to_vec();
                tensor.external_data.clear();
                tensor.data_location = Some(0);
            }
        }
        let hash =
            if let Some((_, _, _, _, data_hash, _)) = ASD_FILES.iter().find(|(n, ..)| *n == name) {
                format!("{hash}:{data_hash}")
            } else {
                hash.to_owned()
            };
        checkpoint(hooks, Stage::Vision)?;
        Ok(Self { proto, hash, kind })
    }
}
fn specialize(proto: &mut pb::ModelProto, kind: Kind, dims: &[Vec<usize>]) -> Result<(), Error> {
    let graph = proto.graph.as_mut().ok_or(Error::VisionLoad)?;
    let names: &[&str] = match kind {
        Kind::Audio => &["audio"],
        Kind::Visual => &["video"],
        Kind::Backend => &["embedA", "embedV"],
        _ => return Err(Error::VisionShape),
    };
    if graph.input.len() != dims.len() || graph.input.len() != names.len() {
        return Err(Error::VisionShape);
    }
    for ((input, shape), name) in graph.input.iter_mut().zip(dims).zip(names) {
        if input.name != *name {
            return Err(Error::VisionShape);
        }
        let Some(pb::type_proto::Value::TensorType(t)) =
            input.r#type.as_mut().and_then(|t| t.value.as_mut())
        else {
            return Err(Error::VisionShape);
        };
        if t.elem_type != 1 || t.shape.as_ref().is_none_or(|s| s.dim.len() != shape.len()) {
            return Err(Error::VisionShape);
        }
        if let Some(s) = t.shape.as_mut() {
            for d in &mut s.dim {
                d.value = None;
            }
        }
    }
    graph.value_info.clear();
    for output in &mut graph.output {
        if let Some(pb::type_proto::Value::TensorType(t)) =
            output.r#type.as_mut().and_then(|t| t.value.as_mut())
        {
            t.shape = None;
        }
    }
    Ok(())
}
fn tensor(shape: &[usize], values: &[f32]) -> Result<TValue, Error> {
    Tensor::from_shape(shape, values)
        .map(Into::into)
        .map_err(|_| Error::VisionShape)
}
fn output(values: &[TValue], i: usize, shape: &[usize]) -> Result<Vec<f32>, Error> {
    let value = values.get(i).ok_or(Error::VisionShape)?;
    if value.shape() != shape {
        return Err(Error::VisionShape);
    }
    let values: Vec<f32> = value
        .to_plain_array_view::<f32>()
        .map_err(|_| Error::VisionShape)?
        .iter()
        .copied()
        .collect();
    finite(&values)?;
    Ok(values)
}
fn run(
    plan: &Arc<TypedRunnableModel>,
    input: Vec<TValue>,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<TValue>, Error> {
    checkpoint(hooks, Stage::Vision)?;
    // Tract's inner executor scope restores TLS only on normal return. Catch
    // inside an outer scope so a caught inference panic restores the caller TLS.
    let result = tract_linalg::multithread::multithread_tract_scope(Executor::SingleThread, || {
        guarded(
            || {
                plan.run(input.into())
                    .map(|v| v.into_vec())
                    .map_err(|_| Error::Inference)
            },
            Error::Inference,
        )
    });
    checkpoint(hooks, Stage::Vision)?;
    result
}
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>, error: Error) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| error)?
}
const ASD_FILES: [(&str, u64, &str, u64, &str, Kind); 3] = [
    (
        "lightasd_audio.onnx",
        99776,
        "335dd4f221fbe90330761209c1acd8637fd67399003d69b79031a7d75c820318",
        1105408,
        "b440a45f84af5e34890f48573b1e164587c690191f7a2c22d902ad5edb61f152",
        Kind::Audio,
    ),
    (
        "lightasd_visual.onnx",
        19002,
        "4bcec1cc985eea8416540ab481e198e611b4125b26c124408cd89442bf4def4e",
        2173696,
        "4400bdaa0925a73bc0486fd569be5223fc8b9cb5e2c6c2d8c487b288d4b1e267",
        Kind::Visual,
    ),
    (
        "lightasd_backend.onnx",
        9393,
        "f3d2b8b2ac81888e1a023e0bb1b3d9b764a151bf309d2510ae1357fbecd58084",
        794112,
        "960c75126082846b34623606a9aff56baa6f092f1dbb7d37d59a2096105af9a0",
        Kind::Backend,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tract_onnx::prelude::{DatumExt, IntoRunnable, TypedModel};
    struct CancelAfterInference(AtomicUsize);
    impl ExecutionHooks for CancelAfterInference {
        fn cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst) >= 2
        }
        fn stage(&self, _: Stage) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    #[allow(clippy::unwrap_used)]
    fn cancellation_at_return_boundary_discards_completed_inference() {
        let mut model = TypedModel::default();
        let input = model.add_source("input", f32::fact([1])).unwrap();
        model.select_output_outlets(&[input]).unwrap();
        let plan = model.into_runnable().unwrap();
        let hooks = CancelAfterInference(AtomicUsize::new(0));
        assert!(matches!(
            run(&plan, vec![tensor(&[1], &[1.]).unwrap()], &hooks),
            Err(Error::Cancelled)
        ));
        assert_eq!(hooks.0.load(Ordering::SeqCst), 2);
    }
}
