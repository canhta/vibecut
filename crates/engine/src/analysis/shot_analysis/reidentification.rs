//! Associates confident face tracks across shots.
use super::{EditError, FaceTrack, SpeakerRef};
use ai::vision::FaceEmbedding;

// Calibrated on agent-labelled interview boundaries; evidence and limitations
// live in the ticket report. The generic benchmark threshold missed our profile pairs.
const THRESHOLD: f32 = 0.30;
const MARGIN: f32 = 0.08;
const MAX_ANCHORS: usize = 1024;
pub(super) type Evidence = Vec<(u32, Vec<FaceEmbedding>)>;
#[derive(Default)]
pub(super) struct Identities {
    anchors: Vec<(SpeakerRef, Vec<FaceEmbedding>)>,
}

impl Identities {
    pub(super) fn assign(
        &mut self,
        shot: u32,
        tracks: &mut [FaceTrack],
        evidence: Evidence,
        ctx: &dyn ai::ExecutionHooks,
    ) -> Result<(), EditError> {
        if ctx.cancelled() {
            return Err(EditError::Runtime(crate::RuntimeError::Cancelled));
        }
        let mut decisions = Vec::new();
        for (id, samples) in &evidence {
            if ctx.cancelled() {
                return Err(EditError::Runtime(crate::RuntimeError::Cancelled));
            }
            let mut scores: Vec<_> = self
                .anchors
                .iter()
                .map(|(anchor, prior)| {
                    // Every sampled crop must agree: no lucky maximum over a gallery,
                    // no updating an anchor with its own predicted matches (no drift).
                    let score = samples
                        .iter()
                        .flat_map(|a| prior.iter().map(move |b| a.similarity(b)))
                        .fold(1.0f32, f32::min);
                    (*anchor, score)
                })
                .collect();
            scores.sort_by(|a, b| b.1.total_cmp(&a.1));
            let candidate = scores
                .first()
                .filter(|(_, score)| {
                    *score >= THRESHOLD
                        && scores
                            .get(1)
                            .is_none_or(|(_, runner)| score - runner >= MARGIN)
                })
                .map(|(anchor, _)| *anchor);
            decisions.push((*id, candidate));
        }
        for track in tracks.iter_mut() {
            if ctx.cancelled() {
                return Err(EditError::Runtime(crate::RuntimeError::Cancelled));
            }
            track.identity = decisions
                .iter()
                .find(|(id, _)| *id == track.id)
                .and_then(|(_, value)| *value)
                .filter(|candidate| {
                    decisions
                        .iter()
                        .filter(|(_, v)| *v == Some(*candidate))
                        .count()
                        == 1
                });
        }
        // Commit anchors only after all decisions for this shot. Simultaneous
        // faces cannot identify each other; ambiguous faces remain Unknown.
        for (id, samples) in evidence {
            if ctx.cancelled() {
                return Err(EditError::Runtime(crate::RuntimeError::Cancelled));
            }
            if tracks.iter().any(|t| t.id == id && t.identity.is_none()) {
                if self.anchors.len() == MAX_ANCHORS {
                    return Err(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    ));
                }
                self.anchors.push((SpeakerRef { shot, track: id }, samples));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    struct Hooks(bool);
    impl ai::ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            self.0
        }
        fn stage(&self, _: ai::Stage) {}
    }
    fn sample(x: f32, y: f32) -> FaceEmbedding {
        let mut v = vec![0.; 128];
        v[0] = x;
        v[1] = y;
        FaceEmbedding::new(&v).unwrap()
    }
    fn track(id: u32) -> FaceTrack {
        FaceTrack {
            id,
            identity: None,
            bounds: [0., 0., 20., 20.],
            observed_frames: 25,
        }
    }
    fn gallery() -> Identities {
        Identities {
            anchors: vec![(SpeakerRef { shot: 0, track: 0 }, vec![sample(1., 0.)])],
        }
    }
    #[test]
    fn uncertain_ambiguous_colliding_and_disagreeing_faces_stay_unknown() {
        for (cosine, expected) in [(0.2999, false), (0.3001, true), (1., true), (-1., false)] {
            let mut tracks = vec![track(0)];
            gallery()
                .assign(
                    1,
                    &mut tracks,
                    vec![(
                        0,
                        vec![sample(cosine, (1. - cosine * cosine).max(0.).sqrt())],
                    )],
                    &Hooks(false),
                )
                .unwrap();
            assert_eq!(tracks[0].identity.is_some(), expected);
        }
        let mut ambiguous = gallery();
        ambiguous
            .anchors
            .push((SpeakerRef { shot: 0, track: 1 }, vec![sample(1., 0.)]));
        let mut tracks = vec![track(0)];
        ambiguous
            .assign(
                1,
                &mut tracks,
                vec![(0, vec![sample(1., 0.)])],
                &Hooks(false),
            )
            .unwrap();
        assert_eq!(tracks[0].identity, None);
        for (runner, expected) in [(0.93, false), (0.91, true)] {
            let mut identities = gallery();
            identities.anchors.push((
                SpeakerRef { shot: 0, track: 1 },
                vec![sample(runner, (1. - runner * runner).sqrt())],
            ));
            identities
                .assign(
                    1,
                    &mut tracks,
                    vec![(0, vec![sample(1., 0.)])],
                    &Hooks(false),
                )
                .unwrap();
            assert_eq!(tracks[0].identity.is_some(), expected);
        }
        let mut tracks = vec![track(0), track(1)];
        gallery()
            .assign(
                1,
                &mut tracks,
                vec![(0, vec![sample(1., 0.)]), (1, vec![sample(1., 0.)])],
                &Hooks(false),
            )
            .unwrap();
        assert!(tracks.iter().all(|t| t.identity.is_none()));
        let mut tracks = vec![track(0)];
        gallery()
            .assign(
                1,
                &mut tracks,
                vec![(0, vec![sample(1., 0.), sample(0., 1.)])],
                &Hooks(false),
            )
            .unwrap();
        assert_eq!(tracks[0].identity, None);
    }
    #[test]
    fn anchors_do_not_drift_and_cancellation_and_gallery_limits_are_errors() {
        let mut identities = gallery();
        let mut tracks = vec![track(0)];
        identities
            .assign(
                1,
                &mut tracks,
                vec![(0, vec![sample(0.8, 0.6)])],
                &Hooks(false),
            )
            .unwrap();
        assert_eq!(tracks[0].identity, Some(SpeakerRef { shot: 0, track: 0 }));
        assert_eq!(identities.anchors.len(), 1);
        identities
            .assign(
                2,
                &mut tracks,
                vec![(0, vec![sample(0.2, 0.98)])],
                &Hooks(false),
            )
            .unwrap();
        assert_eq!(tracks[0].identity, None);
        assert!(matches!(
            identities.assign(
                3,
                &mut tracks,
                vec![(0, vec![sample(1., 0.)])],
                &Hooks(true)
            ),
            Err(EditError::Runtime(crate::RuntimeError::Cancelled))
        ));
        identities.anchors = (0..MAX_ANCHORS)
            .map(|i| {
                (
                    SpeakerRef {
                        shot: i as u32,
                        track: 0,
                    },
                    vec![sample(1., 0.)],
                )
            })
            .collect();
        assert!(matches!(
            identities.assign(
                1025,
                &mut tracks,
                vec![(0, vec![sample(0., 1.)])],
                &Hooks(false)
            ),
            Err(EditError::Control(
                crate::session::wire::ControlError::Limit
            ))
        ));
    }

    /// Local-only labelled boundary evaluation, through the production matcher
    /// and verified tract adapter. No paths, crops or descriptors are printed.
    #[cfg(feature = "vision")]
    #[test]
    #[ignore = "requires explicitly supplied private agent-labelled boundary tensors"]
    fn real_face_reidentification_eval() {
        use ai::vision::{IdentityFrame, TractFaceEmbedding};
        use std::{collections::BTreeMap, path::PathBuf};
        let root = PathBuf::from(std::env::var_os("VIBECUT_REID_EVAL").unwrap());
        let weights = PathBuf::from(std::env::var_os("VIBECUT_REID_MODELS").unwrap());
        let labels: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("19-labels.json")).unwrap()).unwrap();
        let mut model = TractFaceEmbedding::load(&weights, 1, &Hooks(false)).unwrap();
        let mut embeddings = BTreeMap::new();
        for entry in std::fs::read_dir(root.join("19-tensors")).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_some_and(|v| v == "bin")
                && p.file_stem()
                    .is_some_and(|v| v.to_string_lossy().starts_with("c0-"))
            {
                let bytes = std::fs::read(&p).unwrap();
                let values: Vec<_> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|v| f32::from_le_bytes(*v))
                    .collect();
                embeddings.insert(
                    p.file_stem().unwrap().to_string_lossy().into_owned(),
                    model
                        .embed(IdentityFrame::new(&values).unwrap(), &Hooks(false))
                        .unwrap(),
                );
            }
        }
        let mut reports = Vec::new();
        for boundary in labels["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["clip"] == 0)
        {
            let pairs = boundary["pairs"].as_array().unwrap();
            let keys = |side: &str| {
                pairs
                    .iter()
                    .map(|p| p[side].as_str().unwrap().to_owned())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            };
            let (left, right) = (keys("a"), keys("b"));
            let evidence = |keys: &[String]| {
                keys.iter()
                    .enumerate()
                    .filter_map(|(i, k)| embeddings.get(k).map(|e| (i as u32, vec![e.clone()])))
                    .collect()
            };
            let mut identities = Identities::default();
            identities
                .assign(
                    0,
                    &mut left
                        .iter()
                        .enumerate()
                        .map(|(i, _)| track(i as u32))
                        .collect::<Vec<_>>(),
                    evidence(&left),
                    &Hooks(false),
                )
                .unwrap();
            let mut tracks: Vec<_> = right
                .iter()
                .enumerate()
                .map(|(i, _)| track(i as u32))
                .collect();
            identities
                .assign(1, &mut tracks, evidence(&right), &Hooks(false))
                .unwrap();
            let mut counts = [0u32; 4]; // TP, FP, FN, TN; Unknown on a same pair is FN.
            for pair in pairs {
                let a = left
                    .iter()
                    .position(|k| k == pair["a"].as_str().unwrap())
                    .unwrap();
                let b = right
                    .iter()
                    .position(|k| k == pair["b"].as_str().unwrap())
                    .unwrap();
                let matched = tracks[b].identity
                    == Some(SpeakerRef {
                        shot: 0,
                        track: a as u32,
                    });
                let same = pair["label"] == "same";
                counts[match (same, matched) {
                    (true, true) => 0,
                    (false, true) => 1,
                    (true, false) => 2,
                    (false, false) => 3,
                }] += 1;
            }
            reports.push(serde_json::json!({"boundary":boundary["boundary"],"split":boundary["split"],"counts":counts}));
        }
        std::fs::write(
            root.join("19-matcher-eval.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
        println!(
            "evaluated_boundaries={} embedding_crops={}",
            reports.len(),
            embeddings.len()
        );
    }
}
