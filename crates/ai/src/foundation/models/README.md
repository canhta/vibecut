# Model catalog and language-neutral weights

`models::catalog` is metadata only: identity, licence, licence URL, source, exact files with pinned HTTPS URLs, sizes and SHA-256. Downloads are engine-owned, user-initiated, consented and integrity-verified; weights are never bundled.

Default cache discovery belongs to [engine runtime](../../../../engine/src/runtime/README.md#default-cache-location).

`Model::languages` is the supported language scope. An empty list means language-neutral weights that neither read nor produce language, such as vision detectors; `Model::language_neutral()` exposes it. It never means every language: `Model::matches_language` is false for neutral weights and `suggested_transcriber` requires a real match, so a vision entry is never an ASR model. Concrete lists: `whisper-large-v3-turbo` vi, en, zh; `gipformer-68m-rnnt-int8` vi; `whisper-small-dtw` vi, en, zh, zh-Hans, zh-Hant.

Vision entries in this table are MIT and language-neutral; the Apache-2.0 SFace and PP-HumanSeg entries are documented in [vision](../../vision/README.md) and below:

| Model id | Upstream source | Files |
|---|---|---|
| `yunet-2026may` | `opencv/opencv_zoo` `48b10fa505aeb51fbd127fc60a5c660040e823b7`, `models/face_detection_yunet` | `face_detection_yunet_2026may.onnx`, 229,738 B, `ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0` |
| `clip-vit-base-patch32` | `Xenova/clip-vit-base-patch32` `6b7c523703ae26e39be4b03774966f8571c7f523`; weights derived from MIT OpenAI CLIP | `vision_model.onnx` 351,685,709 B `fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40`; `text_model.onnx` 254,058,553 B `3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc`; `tokenizer.json` 2,224,119 B `f7f3b7af117d467b58374797691a6438d3e6b9e9cef800dfd5dced7f697a90cd`; `config.json` 4,524 B `493ef57ff783e42d1530c91b53469b7fdf8db8a9c1408e86998fcb7899a4f495`; `preprocessor_config.json` 520 B `6f638fb9401a6d6296feff533ee7efe657b787c49f954f82f5906b36ef2a1b1f` |

`pphumanseg-2023mar` (Apache-2.0, so default-eligible; the model-directory notice is recorded in [PPHUMANSEG.attribution](../../../PPHUMANSEG.attribution)) is the person-segmentation weights behind the person matte, chosen from primary sources: the OpenCV Zoo model README names Apache-2.0 for every file in the directory, the weights are a PaddleHub PP-HumanSeg port, and the graph uses only Conv, Relu, Add, Resize (linear, half_pixel), Concat, Transpose and Softmax, which the admitted tract runtime implements. Alternatives were refused on licence or runtime: robust video matting weights are GPL-3.0 (not admitted for weights without an ADR) and MediaPipe selfie segmentation ships as TFLite, which tract does not run.

| Field | Value |
|---|---|
| Model card | [OpenCV Zoo `human_segmentation_pphumanseg` README](https://github.com/opencv/opencv_zoo/blob/48b10fa505aeb51fbd127fc60a5c660040e823b7/models/human_segmentation_pphumanseg/README.md): accuracy 0.9656, mIoU 0.9164 on the zoo's own evaluation |
| Version | `opencv/opencv_zoo` `48b10fa505aeb51fbd127fc60a5c660040e823b7`, file `human_segmentation_pphumanseg_2023mar.onnx` (ONNX IR 8, opset 11) |
| File | 6,163,938 B, SHA-256 `552d8a984054e59b5d773d24b9b12022b22046ceb2bbc4c9aaeacec36a9ddf24` (the int8 and block-quantized siblings are not admitted: the zoo records int8 mIoU 0.3642) |
| Input | `x` f32 `[1,3,192,192]`, RGB, `(v/255 - 0.5)/0.5` |
| Output | `save_infer_model/scale_0.tmp_1` f32 `[1,2,192,192]`, softmax over the channel axis; channel 1 is the person probability |

The CLIP text tower is registered so the two fixed prompt embeddings can be derived once per loaded bundle; it is not used for per-frame text inference. No derived embedding file is shipped. The [vision contract](../../vision/README.md) and its `tract`/`decisions` modules own the prompts and the screen-score recipe.

## Light-ASD export recipe (entry intentionally absent)

`light-asd-ava` has no catalog entry until a project-controlled immutable HTTPS host for its converted graphs is approved; `ModelFile` admits no local or derived path. Upstream: `Junhua-Liao/Light-ASD` `ed38c232de5efe0261dbd68627c0ade7cdfe14eb`, MIT repository notice (no separate weight licence; record the maintainer decision before any default), `weight/pretrain_AVA_CVPR.model`, 4,175,289 B, SHA-256 `d44bc3ea7baa8e0946fa3921311714a630ed8b90a1928fab0dbe30d918909317`.

The runtime bundle is the original dynamic export triplet; static exports and per-length files are not admitted. Supply these six local files explicitly:

| File | Bytes | SHA-256 |
|---|---:|---|
| `lightasd_audio.onnx` | 99,776 | `335dd4f221fbe90330761209c1acd8637fd67399003d69b79031a7d75c820318` |
| `lightasd_audio.onnx.data` | 1,105,408 | `b440a45f84af5e34890f48573b1e164587c690191f7a2c22d902ad5edb61f152` |
| `lightasd_visual.onnx` | 19,002 | `4bcec1cc985eea8416540ab481e198e611b4125b26c124408cd89442bf4def4e` |
| `lightasd_visual.onnx.data` | 2,173,696 | `4400bdaa0925a73bc0486fd569be5223fc8b9cb5e2c6c2d8c487b288d4b1e267` |
| `lightasd_backend.onnx` | 9,393 | `f3d2b8b2ac81888e1a023e0bb1b3d9b764a151bf309d2510ae1357fbecd58084` |
| `lightasd_backend.onnx.data` | 794,112 | `960c75126082846b34623606a9aff56baa6f092f1dbb7d37d59a2096105af9a0` |
