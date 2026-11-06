# Model catalog and language-neutral weights

`models::catalog` is metadata only: identity, licence, licence URL, source, exact
files with pinned HTTPS URLs, sizes and SHA-256. Downloads remain engine-owned,
user-initiated, consented and integrity-verified; weights are never bundled.

`Model::languages` describes supported language scope. An empty list is
**language-neutral** metadata for weights that do not read or produce language,
such as vision detectors. Validation accepts that empty list instead of rejecting
it, and `Model::language_neutral()` exposes the distinction. It never means
"supports every language": `Model::matches_language` returns false for neutral
weights, and `suggested_transcriber` requires a real language match, so a vision
entry can never be selected as an ASR model. Existing entries keep their concrete
lists (`whisper-large-v3-turbo`: vi, en, zh; `gipformer-68m-rnnt-int8`: vi;
`whisper-small-dtw`: vi, en, zh, zh-Hans, zh-Hant).

Vision entries, both MIT and both language-neutral:

| Model id | Upstream source | Files |
|---|---|---|
| `yunet-2026may` | `opencv/opencv_zoo` `48b10fa505aeb51fbd127fc60a5c660040e823b7`, `models/face_detection_yunet` | `face_detection_yunet_2026may.onnx`, 229,738 B, `ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0` |
| `clip-vit-base-patch32` | `Xenova/clip-vit-base-patch32` `6b7c523703ae26e39be4b03774966f8571c7f523`; weights derived from MIT OpenAI CLIP | `vision_model.onnx` 351,685,709 B `fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40`; `text_model.onnx` 254,058,553 B `3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc`; `tokenizer.json` 2,224,119 B `f7f3b7af117d467b58374797691a6438d3e6b9e9cef800dfd5dced7f697a90cd`; `config.json` 4,524 B `493ef57ff783e42d1530c91b53469b7fdf8db8a9c1408e86998fcb7899a4f495`; `preprocessor_config.json` 520 B `6f638fb9401a6d6296feff533ee7efe657b787c49f954f82f5906b36ef2a1b1f` |

The CLIP text tower is registered so fixed prompt embeddings are reproducible,
not to run text inference per frame. No derived embedding file is downloaded,
shipped or bundled: compute the two fixed prompts once on first use, keyed and
cached by text-model hash + tokenizer hash + exact prompt text. Tokenize with the
pinned `tokenizer.json`, pad or truncate with id 49,407 to 77 ids, run
`text_model.onnx` with `input_ids` `[N,77]` int64, and L2-normalize each projected
embedding. The prompts are `a screenshot of a computer screen` and
`a photograph of a person`; image embeddings come from `vision_model.onnx` with
the pinned 224x224 RGB mean/std; screen probability uses `14.3 * (image · text)`
and a 0.5 threshold. The consuming adapter owns that wiring; this crate only
publishes metadata.

## Light-ASD export recipe (entry intentionally absent)

`light-asd-ava` has no catalog entry until a project-controlled immutable HTTPS
host for its converted graphs is approved; `ModelFile` admits no local or derived
path. The upstream source is `Junhua-Liao/Light-ASD`
`ed38c232de5efe0261dbd68627c0ade7cdfe14eb`, MIT repository notice (the repository
has no separate weight-licence file; record the maintainer decision before any
default), `weight/pretrain_AVA_CVPR.model`, 4,175,289 B, SHA-256
`d44bc3ea7baa8e0946fa3921311714a630ed8b90a1928fab0dbe30d918909317`.

The canonical runtime bundle is the **original dynamic export** triplet. Static exports and per-length
files are not admitted. Supply these six local files explicitly (no automatic
transfer, catalog entry or default-model exposure until hosting is approved):

| File | Bytes | SHA-256 |
|---|---:|---|
| `lightasd_audio.onnx` | 99,776 | `335dd4f221fbe90330761209c1acd8637fd67399003d69b79031a7d75c820318` |
| `lightasd_audio.onnx.data` | 1,105,408 | `b440a45f84af5e34890f48573b1e164587c690191f7a2c22d902ad5edb61f152` |
| `lightasd_visual.onnx` | 19,002 | `4bcec1cc985eea8416540ab481e198e611b4125b26c124408cd89442bf4def4e` |
| `lightasd_visual.onnx.data` | 2,173,696 | `4400bdaa0925a73bc0486fd569be5223fc8b9cb5e2c6c2d8c487b288d4b1e267` |
| `lightasd_backend.onnx` | 9,393 | `f3d2b8b2ac81888e1a023e0bb1b3d9b764a151bf309d2510ae1357fbecd58084` |
| `lightasd_backend.onnx.data` | 794,112 | `960c75126082846b34623606a9aff56baa6f092f1dbb7d37d59a2096105af9a0` |
