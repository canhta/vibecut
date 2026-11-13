//! Local safe Candle teacher-forcing, cross-attention normalization and DTW.
mod mel;
mod model;
mod path;
mod tokenize;
use crate::{
    Aligner, AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location,
    Operation, ProviderConfig, RecognitionSegment, Request, SampleRange, Scope, Stage, Support,
    TimedText, TimingUnit, contracts::checkpoint,
};
use candle_core::Tensor;
use captions::{LanguageRegistry, LanguageTag, NfcText};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek},
    path::Path,
    sync::Mutex,
};
use tokenizers::Tokenizer;
pub const MODEL_ID: &str = "whisper-small-dtw";
pub const MODE: &str = "teacher-forced-dtw";
pub const MAX_SAMPLES: u64 = 480000;
impl Scope {
    /// Consent scope of one teacher-forced alignment pass over `samples`.
    pub fn alignment(
        provider: ProviderConfig,
        language: LanguageTag,
        samples: SampleRange,
        sample_rate: u32,
        payload_fingerprint: NfcText,
    ) -> Self {
        Self {
            provider,
            operation: Operation::Align,
            payload_fingerprint,
            mode: MODE.into(),
            texts: Vec::new(),
            source_language: language,
            target_language: None,
            source_samples: samples,
            sample_rate,
            max_bytes: samples.len().saturating_mul(4),
        }
    }
}
struct Runtime {
    model: model::Model,
    tokenizer: Tokenizer,
    heads: Vec<(usize, usize)>,
    eot: u32,
    sot: u32,
    transcribe: u32,
    no_timestamps: u32,
}
/// CPU-only, one invocation at a time; engine schedules/owns workers and fallback.
pub struct CandleAligner {
    runtime: Mutex<Runtime>,
    config: ProviderConfig,
    languages: Vec<LanguageTag>,
    profiles: LanguageRegistry,
}
fn inference<T>(value: candle_core::Result<T>, hooks: &dyn ExecutionHooks) -> Result<T, Error> {
    value.map_err(|_| {
        if hooks.cancelled() {
            Error::Cancelled
        } else {
            Error::Inference
        }
    })
}
fn read_verified(
    dir: &Path,
    file: &crate::models::ModelFile,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<u8>, Error> {
    let mut reader = File::open(dir.join(&file.name)).map_err(crate::models::storage_error)?;
    crate::models::verify_file(&mut reader, file, hooks)?;
    reader.rewind().map_err(crate::models::storage_error)?;
    let mut bytes = Vec::new();
    reader
        .take(file.size + 1)
        .read_to_end(&mut bytes)
        .map_err(crate::models::storage_error)?;
    if bytes.len() as u64 != file.size {
        return Err(Error::ModelIntegrity);
    }
    Ok(bytes)
}
impl CandleAligner {
    /// Loads only the pinned catalog model from immutable verified files. No mmap,
    /// unsafe, downloads, private keys, remote IO or whisper.cpp runtime is used.
    pub fn load(
        metadata: &crate::models::Model,
        dir: &Path,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        if metadata != &crate::models::alignment_model()? {
            return Err(Error::Unsupported);
        }
        checkpoint(hooks, Stage::Align)?;
        let file = |name: &str| {
            metadata
                .files
                .iter()
                .find(|f| f.name == name)
                .ok_or(Error::ModelIntegrity)
        };
        let config = read_verified(dir, file("config.json")?, hooks)?;
        let cfg: model::Config =
            serde_json::from_slice(&config).map_err(|_| Error::ModelIntegrity)?;
        validate_config(&cfg)?;
        #[derive(Deserialize)]
        struct Generation {
            alignment_heads: Vec<(usize, usize)>,
        }
        let generation: Generation =
            serde_json::from_slice(&read_verified(dir, file("generation_config.json")?, hooks)?)
                .map_err(|_| Error::AttentionUnavailable)?;
        if generation.alignment_heads.is_empty()
            || generation.alignment_heads.len() > 64
            || generation
                .alignment_heads
                .iter()
                .any(|(l, h)| *l >= cfg.decoder_layers || *h >= cfg.decoder_attention_heads)
        {
            return Err(Error::AttentionUnavailable);
        }
        let tokenizer = Tokenizer::from_bytes(read_verified(dir, file("tokenizer.json")?, hooks)?)
            .map_err(|_| Error::ModelIntegrity)?;
        let token = |name: &str| {
            tokenizer
                .token_to_id(name)
                .filter(|id| (*id as usize) < cfg.vocab_size)
                .ok_or(Error::ModelIntegrity)
        };
        let (eot, sot, transcribe, no_timestamps) = (
            token("<|endoftext|>")?,
            token("<|startoftranscript|>")?,
            token("<|transcribe|>")?,
            token("<|notimestamps|>")?,
        );
        let model = inference(
            model::Model::load(cfg, read_verified(dir, file("model.safetensors")?, hooks)?),
            hooks,
        )?;
        checkpoint(hooks, Stage::Align)?;
        let profiles = LanguageRegistry::standard()?;
        let policy = serde_json::to_vec(&("candle-0.11.1:tokenizers-0.23.2:rustfft-6.4.1:teacherforcing-v1:median7:dtw:frame320:cpu", &metadata.files, &profiles)).map_err(|_| Error::Invalid("alignment settings"))?;
        let fingerprint: String = Sha256::digest(&policy)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let config = ProviderConfig::new(
            "local-candle-dtw".into(),
            MODEL_ID.into(),
            format!("candle-dtw-v1:{fingerprint}").into(),
            Location::Local {
                backend: "candle".into(),
                model_ref: file("model.safetensors")?.sha256.as_str().into(),
            },
        )?;
        Ok(Self {
            runtime: Mutex::new(Runtime {
                model,
                tokenizer,
                heads: generation.alignment_heads,
                eot,
                sot,
                transcribe,
                no_timestamps,
            }),
            config,
            languages: metadata.languages.clone(),
            profiles,
        })
    }
}
fn validate_config(c: &model::Config) -> Result<(), Error> {
    if c.d_model == 0
        || c.d_model > 1280
        || c.encoder_layers == 0
        || c.encoder_layers > 32
        || c.decoder_layers == 0
        || c.decoder_layers > 32
        || ![80, 128].contains(&c.num_mel_bins)
        || c.max_source_positions != 1500
        || c.max_target_positions < 5
        || c.max_target_positions > 448
        || c.vocab_size < 50000
        || c.vocab_size > 60000
        || c.encoder_attention_heads == 0
        || c.encoder_attention_heads > 64
        || c.decoder_attention_heads == 0
        || c.decoder_attention_heads > 64
        || !c.d_model.is_multiple_of(c.encoder_attention_heads)
        || !c.d_model.is_multiple_of(c.decoder_attention_heads)
    {
        return Err(Error::ModelIntegrity);
    }
    Ok(())
}
impl Aligner for CandleAligner {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        let unit = self.profiles.profile(&scope.source_language).timing_unit;
        Capabilities {
            model: self.config.model().clone(),
            mode: MODE.into(),
            operation: Operation::Align,
            languages: self.languages.clone(),
            target_languages: vec![],
            timing: [TimingUnit::Segment, unit].into(),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: Some(MAX_SAMPLES * 4),
            max_duration_micros: Some(30_000_000),
            evidence: Evidence::Advertised,
            revision: "whisper-small-dtw-v1".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn align(
        &self,
        audio: AudioWindow<'_>,
        missing: &[RecognitionSegment],
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        checkpoint(hooks, Stage::Align)?;
        if request.scope.provider != self.config {
            return Err(Error::Unsupported);
        }
        request.prepare(
            &self.capabilities(request.scope),
            &CostEstimate::Free,
            Operation::Align,
        )?;
        audio.validate(request.scope)?;
        if audio.sample_rate != 16000
            || audio.samples.len() < 201
            || audio.range.len() > MAX_SAMPLES
            || missing.is_empty()
            || missing.len() > 64
        {
            return Err(Error::Invalid("alignment window"));
        }
        if missing.len() != request.scope.texts.len()
            || missing
                .iter()
                .zip(&request.scope.texts)
                .any(|(segment, text)| &segment.text != text)
        {
            return Err(Error::ConsentRequired);
        }
        let mut ids = BTreeSet::new();
        let mut total = 0usize;
        for s in missing {
            total = total.saturating_add(s.text.as_str().len());
            if s.text.as_str().trim().is_empty()
                || s.id.as_str().trim().is_empty()
                || s.id.as_str().len() > 256
                || !ids.insert(&s.id)
                || s.range.is_some_and(|r| !audio.range.contains(r))
            {
                return Err(Error::Invalid("alignment segments"));
            }
        }
        if total > 16384 {
            return Err(Error::Payload);
        }
        if audio.samples.iter().all(|v| v.abs() < 1e-8) {
            return Err(Error::TimingUnavailable);
        }
        let mut runtime = self.runtime.try_lock().map_err(|_| Error::Inference)?;
        let Runtime {
            model,
            tokenizer,
            heads,
            eot,
            sot,
            transcribe,
            no_timestamps,
        } = &mut *runtime;
        let profile = self.profiles.profile(&request.scope.source_language);
        let (text_tokens, units) = tokenize::encode(tokenizer, missing, profile, *eot)?;
        let language = request.scope.source_language.primary();
        let lang = tokenizer
            .token_to_id(&format!("<|{language}|>"))
            .ok_or(Error::UnsupportedLanguage)?;
        let mut tokens = vec![*sot, lang, *transcribe, *no_timestamps];
        tokens.extend_from_slice(&text_tokens);
        tokens.push(*eot);
        if tokens.len() > model.cfg.max_target_positions {
            return Err(Error::Payload);
        }
        let mel = mel::log_mel(audio.samples, model.cfg.num_mel_bins, hooks)?;
        let tensor = inference(
            Tensor::from_vec(mel, (1, model.cfg.num_mel_bins, 3000), &model.device),
            hooks,
        )?;
        let encoded = inference(model.encoder.forward(&tensor, hooks), hooks)?;
        let attention = inference(
            model.decoder.forward(&tokens, &encoded, Some(heads), hooks),
            hooks,
        )?;
        let attention = attention.ok_or(Error::AttentionUnavailable)?;
        let frames = audio.samples.len().div_ceil(320).min(1500);
        let attention = inference(attention.narrow(2, 0, frames), hooks)?;
        let attention = inference(attention.contiguous(), hooks)?;
        let weights = inference(candle_nn::ops::softmax_last_dim(&attention), hooks)?;
        let weights = inference(weights.to_vec3::<f32>(), hooks)?;
        let rows = text_tokens.len() + 1;
        let costs = path::costs(&weights, 3, rows, frames)?;
        let bounds = path::boundaries(&costs, rows, frames, hooks)?;
        let mut result: Vec<_> = missing
            .iter()
            .map(|s| RecognitionSegment {
                id: s.id.clone(),
                text: s.text.clone(),
                range: s.range,
                timing: vec![],
                raw_tokens: vec![],
            })
            .collect();
        for unit in units {
            let start = *bounds.get(unit.tokens.start).ok_or(Error::Inference)?;
            let end = *bounds.get(unit.tokens.end).ok_or(Error::Inference)?;
            let source = missing.get(unit.segment).ok_or(Error::Inference)?;
            let allowed = source.range.unwrap_or(audio.range);
            let start = audio
                .range
                .start()
                .checked_add((start * 320) as u64)
                .ok_or(Error::Payload)?
                .max(allowed.start());
            let end = audio
                .range
                .start()
                .checked_add((end * 320) as u64)
                .ok_or(Error::Payload)?
                .min(allowed.end());
            let range = SampleRange::new(start, end).map_err(|_| Error::TimingUnavailable)?;
            let segment = result.get_mut(unit.segment).ok_or(Error::Inference)?;
            segment.timing.push(TimedText {
                text: source
                    .text
                    .as_str()
                    .get(unit.text)
                    .ok_or(Error::Inference)?
                    .into(),
                unit: unit.kind,
                range,
                provenance: crate::Provenance::Forced,
                speaker: None,
                confidence: None,
            });
            segment.range = Some(match segment.range {
                Some(r) => SampleRange::new(r.start().min(start), r.end().max(end))?,
                None => range,
            });
        }
        if result.iter().any(|s| s.timing.is_empty()) {
            return Err(Error::TimingUnavailable);
        }
        checkpoint(hooks, Stage::Align)?;
        Ok(result)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "bounded offline alignment fixtures"
)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Hooks(AtomicBool);
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }
        fn stage(&self, _: Stage) {}
    }
    fn hooks() -> Hooks {
        Hooks(AtomicBool::new(false))
    }
    fn tokenizer() -> Tokenizer {
        Tokenizer::from_bytes(r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"WhitespaceSplit"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"Xin":1,"chào":2,"hello":3,"world":4,"中":5,"文":6},"unk_token":"[UNK]"}}"#.as_bytes()).unwrap()
    }
    fn segments(text: &str) -> Vec<RecognitionSegment> {
        vec![RecognitionSegment {
            id: "1".into(),
            text: text.into(),
            range: None,
            timing: vec![],
            raw_tokens: vec![],
        }]
    }
    #[test]
    fn dtw_path_is_monotonic_follows_attention_and_refuses_hostile_matrices() {
        let mut cost = vec![10.; 4 * 12];
        for row in 0..4 {
            for f in row * 3..row * 3 + 3 {
                cost[row * 12 + f] = -10.;
            }
        }
        assert_eq!(
            path::boundaries(&cost, 4, 12, &hooks()).unwrap(),
            vec![0, 3, 6, 9]
        );
        for bad in [vec![], vec![f32::NAN; 48], vec![f32::INFINITY; 48]] {
            assert!(path::boundaries(&bad, 4, 12, &hooks()).is_err());
        }
        assert!(path::boundaries(&cost, 0, 12, &hooks()).is_err());
        assert!(path::boundaries(&cost, 4, 0, &hooks()).is_err());
        assert!(matches!(
            path::boundaries(&cost, 4, 12, &Hooks(AtomicBool::new(true))),
            Err(Error::Cancelled)
        ));
        for seed in 0..20 {
            let cost: Vec<_> = (0..48).map(|n| ((n * 17 + seed * 7) % 31) as f32).collect();
            let b = path::boundaries(&cost, 4, 12, &hooks()).unwrap();
            assert_eq!(b.len(), 4);
            assert!(b.windows(2).all(|w| w[0] <= w[1]));
            assert!(b.iter().all(|n| *n < 12));
        }
    }
    #[test]
    fn unavailable_degenerate_and_nonfinite_attention_never_produces_fake_zero_times() {
        assert!(matches!(
            path::costs(&[], 0, 2, 3),
            Err(Error::AttentionUnavailable)
        ));
        assert!(matches!(
            path::costs(&[vec![vec![0.; 3]; 2]], 0, 2, 3),
            Err(Error::AttentionUnavailable)
        ));
        assert!(matches!(
            path::costs(&[vec![vec![f32::NAN; 3]; 2]], 0, 2, 3),
            Err(Error::AttentionUnavailable)
        ));
        assert!(matches!(
            path::costs(&[vec![vec![1.; 2]; 2]], 0, 2, 3),
            Err(Error::AttentionUnavailable)
        ));
        assert!(matches!(
            path::costs(&[vec![vec![1.; 3]; 2]], 2, 2, 3),
            Err(Error::AttentionUnavailable)
        ));
        let heads = vec![
            (0..3)
                .map(|row| {
                    (0..21)
                        .map(|frame| if frame / 7 == row { 0.9 } else { 0.1 })
                        .collect()
                })
                .collect(),
        ];
        let cost = path::costs(&heads, 0, 3, 21).unwrap();
        assert!(cost.iter().all(|v| v.is_finite()));
        assert_eq!(
            path::boundaries(&cost, 3, 21, &hooks()).unwrap(),
            vec![0, 7, 14]
        );
    }
    #[test]
    fn language_grouping_reuses_captions_for_vi_syllables_en_words_and_han_characters() {
        let profiles = LanguageRegistry::standard().unwrap();
        for (tag, text, expected) in [
            ("vi", "Xin cha\u{300}o", vec!["Xin", "chào"]),
            ("en", "hello world", vec!["hello", "world"]),
            ("zh", "中文", vec!["中", "文"]),
            ("zh-Hant", "中文", vec!["中", "文"]),
        ] {
            let input = segments(text);
            let profile = profiles.profile(&LanguageTag::parse(tag).unwrap());
            let (ids, units) = tokenize::encode(&tokenizer(), &input, profile, 100).unwrap();
            assert!(!ids.is_empty());
            let words: Vec<_> = units
                .iter()
                .map(|u| input[0].text.as_str().get(u.text.clone()).unwrap())
                .collect();
            assert_eq!(words, expected);
            assert!(units.iter().all(|u| u.kind == profile.timing_unit
                && u.tokens.start < u.tokens.end
                && u.tokens.end <= ids.len()));
        }
        assert!(
            tokenize::encode(
                &tokenizer(),
                &segments(" "),
                profiles.profile(&LanguageTag::parse("en").unwrap()),
                100
            )
            .is_err()
        );
        assert!(
            tokenize::encode(
                &tokenizer(),
                &segments(&"hello ".repeat(444)),
                profiles.profile(&LanguageTag::parse("en").unwrap()),
                100
            )
            .is_err()
        );
    }
    #[test]
    fn mel_front_end_is_bounded_finite_and_cancellable() {
        for input in [
            vec![],
            vec![0.; 200],
            vec![f32::NAN; 16000],
            vec![0.; 480001],
        ] {
            assert!(mel::log_mel(&input, 80, &hooks()).is_err());
        }
        let tone: Vec<_> = (0..16000)
            .map(|i| 0.2 * (2. * std::f32::consts::PI * 1000. * i as f32 / 16000.).sin())
            .collect();
        let m = mel::log_mel(&tone, 80, &hooks()).unwrap();
        assert_eq!(m.len(), 80 * 3000);
        assert!(m.iter().all(|v| v.is_finite()));
        assert!(matches!(
            mel::log_mel(&tone, 80, &Hooks(AtomicBool::new(true))),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn mel_matches_independent_centered_impulse_recipe_without_a_delay_shift() {
        let mut impulse = vec![0.; 16000];
        impulse[0] = 0.5;
        impulse[4000] = -0.25;
        let mel = mel::log_mel(&impulse, 80, &hooks()).unwrap();
        assert!((mel[0] - (0.448372_f32)).abs() < 2e-5);
        assert!((mel[30001] - (-0.061646_f32)).abs() < 2e-5);
        assert!((mel[75002] - (-1.5_f32)).abs() < 2e-5);
        assert!((mel[120024] - (-0.204563_f32)).abs() < 2e-5);
        assert!((mel[180025] - (0.298895_f32)).abs() < 2e-5);
        assert!((mel[237026] - (-0.211887_f32)).abs() < 2e-5);
        assert!((mel[16500] - (-1.5_f32)).abs() < 2e-5);
        assert!(mel::log_mel(&vec![f32::MAX; 16000], 80, &hooks()).is_err());
    }
    #[test]
    fn metadata_mutation_is_refused_before_io() {
        let mut metadata = crate::models::alignment_model().unwrap();
        metadata.files[0].sha256 = "0".repeat(64);
        assert!(matches!(
            CandleAligner::load(&metadata, Path::new("absent"), &hooks()),
            Err(Error::Unsupported)
        ));
        let metadata = crate::models::alignment_model().unwrap();
        assert!(matches!(
            CandleAligner::load(
                &metadata,
                Path::new("absent"),
                &Hooks(AtomicBool::new(true))
            ),
            Err(Error::Cancelled)
        ));
    }
}
