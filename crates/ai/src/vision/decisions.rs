//! Speaker aggregation, scene classification and face decoding from raw detector outputs.
use super::{
    ActiveSpeaker, Error, ExecutionHooks, MAX_CHUNKS, MAX_SHOT_FRAMES, MAX_TRACKS, SceneKind,
    SceneResult, TrackScores, finite,
};
#[cfg(feature = "vision")]
use super::{FACE_HEIGHT, FACE_WIDTH, FaceBox, MAX_FACES, sized};
use crate::{Stage, contracts::checkpoint};
use std::collections::{BTreeMap, BTreeSet};

/// Reproduce /13's overlap means, +/-2 observed-frame smoothing, and highest
/// mean visible track at >=0.4. Empty evidence explicitly means no visible speaker.
/// At most 64 tracks, 512 chunks and 9000 frames per shot; gaps are not filled.
pub fn aggregate_speakers(
    chunks: &[TrackScores<'_>],
    hooks: &dyn ExecutionHooks,
) -> Result<ActiveSpeaker, Error> {
    checkpoint(hooks, Stage::Vision)?;
    if chunks.len() > MAX_CHUNKS {
        return Err(Error::VisionShape);
    }
    let mut tracks: BTreeMap<u32, BTreeMap<usize, (f64, u32)>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for chunk in chunks {
        checkpoint(hooks, Stage::Vision)?;
        if !(25..=75).contains(&chunk.probabilities.len())
            || chunk
                .start_frame
                .checked_add(chunk.probabilities.len())
                .is_none_or(|end| end > MAX_SHOT_FRAMES)
            || !seen.insert((chunk.track, chunk.start_frame, chunk.probabilities.len()))
        {
            return Err(Error::VisionShape);
        }
        finite(chunk.probabilities)?;
        if chunk.probabilities.iter().any(|p| !(0.0..=1.0).contains(p)) {
            return Err(Error::VisionShape);
        }
        if !tracks.contains_key(&chunk.track) && tracks.len() == MAX_TRACKS {
            return Err(Error::VisionShape);
        }
        let frames = tracks.entry(chunk.track).or_default();
        for (i, p) in chunk.probabilities.iter().enumerate() {
            let value = frames.entry(chunk.start_frame + i).or_default();
            value.0 += f64::from(*p);
            value.1 += 1;
        }
    }
    let mut best: Option<(u32, f64)> = None;
    for (track, frames) in tracks {
        checkpoint(hooks, Stage::Vision)?;
        let raw: Vec<_> = frames
            .values()
            .map(|(sum, n)| sum / f64::from(*n))
            .collect();
        let mut sum = 0.;
        for i in 0..raw.len() {
            let window = raw
                .get(i.saturating_sub(2)..(i + 3).min(raw.len()))
                .ok_or(Error::VisionShape)?;
            sum += window.iter().sum::<f64>() / window.len() as f64;
        }
        let mean = sum / raw.len() as f64;
        // Stable ties choose the lowest track identity, independent of chunk order.
        if best.is_none_or(|(_, value)| mean > value) {
            best = Some((track, mean));
        }
    }
    checkpoint(hooks, Stage::Vision)?;
    Ok(match best {
        Some((track, probability)) if probability >= 0.4 => ActiveSpeaker::Visible {
            track,
            probability: probability as f32,
        },
        _ => ActiveSpeaker::NoVisibleSpeaker,
    })
}
fn normalized(values: &[f32]) -> Result<Vec<f32>, Error> {
    finite(values)?;
    let norm = values
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= 1e-12 {
        return Err(Error::VisionNonFinite);
    }
    Ok(values
        .iter()
        .map(|v| (f64::from(*v) / norm) as f32)
        .collect())
}
/// 512-element projected embeddings, prompts ordered screen then person.
/// Normalization and stable two-class softmax match /13; no implicit fallback.
pub fn classify_embedding(image: &[f32], prompts: [&[f32]; 2]) -> Result<SceneResult, Error> {
    if image.len() != 512 || prompts.iter().any(|p| p.len() != 512) {
        return Err(Error::VisionShape);
    }
    let image = normalized(image)?;
    let logits: Vec<f32> = prompts
        .iter()
        .map(|p| {
            Ok(14.3
                * image
                    .iter()
                    .zip(normalized(p)?)
                    .map(|(a, b)| a * b)
                    .sum::<f32>())
        })
        .collect::<Result<_, Error>>()?;
    let a = *logits.first().ok_or(Error::VisionShape)?;
    let b = *logits.get(1).ok_or(Error::VisionShape)?;
    let probability = 1. / (1. + (b - a).exp());
    Ok(SceneResult {
        kind: if probability >= 0.5 {
            SceneKind::Screen
        } else {
            SceneKind::Camera
        },
        screen_probability: Some(probability),
    })
}
#[cfg(feature = "vision")]
pub(super) fn decode_faces(outputs: &[Vec<f32>]) -> Result<Vec<FaceBox>, Error> {
    if outputs.len() != 12 {
        return Err(Error::VisionShape);
    }
    let mut boxes = Vec::new();
    for (i, stride) in [8usize, 16, 32].into_iter().enumerate() {
        let n = (FACE_WIDTH / stride) * (FACE_HEIGHT / stride);
        for offset in [0, 3, 6, 9] {
            let width = match offset {
                0 | 3 => 1,
                6 => 4,
                _ => 10,
            };
            sized(
                outputs.get(i + offset).ok_or(Error::VisionShape)?,
                n * width,
            )?;
        }
        for k in 0..n {
            let cls = *outputs
                .get(i)
                .and_then(|o| o.get(k))
                .ok_or(Error::VisionShape)?;
            let obj = *outputs
                .get(i + 3)
                .and_then(|o| o.get(k))
                .ok_or(Error::VisionShape)?;
            let score = (cls.clamp(0., 1.) * obj.clamp(0., 1.)).sqrt();
            if score <= 0.6 {
                continue;
            }
            let b = outputs
                .get(i + 6)
                .and_then(|o| o.get(k * 4..k * 4 + 4))
                .ok_or(Error::VisionShape)?;
            let [dx, dy, dw, dh]: [f32; 4] = b.try_into().map_err(|_| Error::VisionShape)?;
            let width = dw.clamp(-20., 20.).exp() * stride as f32;
            let height = dh.clamp(-20., 20.).exp() * stride as f32;
            let x = (k % (FACE_WIDTH / stride)) as f32 * stride as f32 + dx * stride as f32
                - width / 2.;
            let y = (k / (FACE_WIDTH / stride)) as f32 * stride as f32 + dy * stride as f32
                - height / 2.;
            finite(&[x, y, width, height])?;
            let mut landmarks = [[0.; 2]; 5];
            let raw = outputs
                .get(i + 9)
                .and_then(|o| o.get(k * 10..k * 10 + 10))
                .ok_or(Error::VisionShape)?;
            for (point, pair) in landmarks.iter_mut().zip(raw.as_chunks::<2>().0) {
                point[0] = (k % (FACE_WIDTH / stride)) as f32 * stride as f32
                    + pair.first().copied().ok_or(Error::VisionShape)? * stride as f32;
                point[1] = (k / (FACE_WIDTH / stride)) as f32 * stride as f32
                    + pair.get(1).copied().ok_or(Error::VisionShape)? * stride as f32;
                finite(point)?;
            }
            boxes.push(FaceBox {
                landmarks: Some(landmarks),
                x,
                y,
                width,
                height,
                confidence: score,
            });
        }
    }
    boxes.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    boxes.truncate(5000);
    let mut kept: Vec<FaceBox> = Vec::new();
    for b in boxes {
        if kept.iter().all(|a| iou(a, &b) <= 0.3) {
            if kept.len() == MAX_FACES {
                return Err(Error::Payload);
            }
            kept.push(b);
        }
    }
    Ok(kept)
}
#[cfg(feature = "vision")]
fn iou(a: &FaceBox, b: &FaceBox) -> f32 {
    let w = ((a.x + a.width).min(b.x + b.width) - a.x.max(b.x)).max(0.);
    let h = ((a.y + a.height).min(b.y + b.height) - a.y.max(b.y)).max(0.);
    w * h / (a.width * a.height + b.width * b.height - w * h).max(f32::MIN_POSITIVE)
}

#[cfg(all(test, feature = "vision"))]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::unwrap_used, clippy::indexing_slicing)]
    fn face_decode_nms_threshold_and_nonfinite_outputs() {
        let mut outputs: Vec<Vec<f32>> = (0..12)
            .map(|i| {
                let stride = [8, 16, 32][i % 3];
                let n = (640 / stride) * (384 / stride);
                vec![
                    0.;
                    n * match i / 3 {
                        0 | 1 => 1,
                        2 => 4,
                        _ => 10,
                    }
                ]
            })
            .collect();
        assert!(decode_faces(&outputs).unwrap().is_empty());
        outputs[0][0] = 1.;
        outputs[3][0] = 1.;
        outputs[9][0] = 1.;
        outputs[9][1] = 2.;
        assert_eq!(decode_faces(&outputs).unwrap().len(), 1);
        assert_eq!(
            decode_faces(&outputs).unwrap()[0].landmarks.unwrap()[0],
            [8., 16.]
        );
        // A second identical box is suppressed, even across feature scales.
        outputs[1][0] = 1.;
        outputs[4][0] = 1.;
        outputs[7][2] = (0.5f32).ln();
        outputs[7][3] = (0.5f32).ln();
        assert_eq!(decode_faces(&outputs).unwrap().len(), 1);
        // Tiny but positive decoded boxes must retain their true IoU.
        outputs[1][0] = 0.;
        outputs[4][0] = 0.;
        outputs[0][1] = 1.;
        outputs[3][1] = 1.;
        outputs[6][4] = -1.;
        for offset in [2, 3, 6, 7] {
            outputs[6][offset] = -20.;
        }
        assert_eq!(decode_faces(&outputs).unwrap().len(), 1);
        outputs[9][0] = f32::NAN;
        assert!(matches!(
            decode_faces(&outputs),
            Err(Error::VisionNonFinite)
        ));
        assert!(matches!(decode_faces(&[]), Err(Error::VisionShape)));
    }
}
