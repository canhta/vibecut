use super::{
    AUDIO_STEP, EditError, FaceTrack, JobContext, Scene, SceneKind, SecondAnalysis, ShotAnalysis,
    SourceRange, SpeakerRef, SpeakerState, TICKS_PER_SECOND, Tick, VIDEO_STEP, VisionDetector,
    VisionFrame, VisionPrepLimits,
};
use ai::vision::{
    FaceBox, FaceEmbedding, FaceFrame, IdentityFrame, SceneFrame, ShotResult, SpeakerChunk,
};

struct Track {
    embeddings: Vec<FaceEmbedding>,
    embedding_attempts: usize,
    bounds: FaceBox,
    seen: Tick,
    visible: bool,
    observed: usize,
    segments: Vec<Vec<(Tick, Vec<f32>)>>,
    score: f64,
    scored: usize,
}
struct Bin {
    presence: [u8; 64],
    scores: [f64; 64],
    scored: [u8; 64],
    samples: u8,
    scene: Scene,
    classified: bool,
    audio: bool,
}
impl Default for Bin {
    fn default() -> Self {
        Self {
            presence: [0; 64],
            scores: [0.; 64],
            scored: [0; 64],
            samples: 0,
            scene: Scene::Unknown,
            classified: false,
            audio: false,
        }
    }
}
pub(super) struct Tracks {
    shot: u32,
    range: SourceRange,
    tracks: Vec<Track>,
    bins: Vec<Bin>,
    frame: usize,
}
impl Tracks {
    pub(super) fn new(shot: u32, range: SourceRange) -> Self {
        let bins = ((range.duration().0 - 1) / TICKS_PER_SECOND + 1) as usize;
        Self {
            shot,
            range,
            tracks: Vec::new(),
            bins: (0..bins).map(|_| Bin::default()).collect(),
            frame: 0,
        }
    }
    pub(super) fn frame(
        &mut self,
        frame: &VisionFrame,
        at: Tick,
        detector: &mut dyn VisionDetector,
        ctx: &JobContext,
    ) -> Result<(), EditError> {
        ctx.check_cancelled()?;
        let bin_index = ((at.0 - self.range.start().0) / TICKS_PER_SECOND) as usize;
        if self.frame.is_multiple_of(3) {
            let tensor = frame.face_tensor()?;
            let faces = detector.faces(FaceFrame::new(tensor.values())?, &VisionHooks(ctx))?;
            // Validate injected adapters as strictly as the production adapter.
            ShotResult::new(
                faces.clone(),
                ai::vision::ActiveSpeaker::NoVisibleSpeaker,
                ai::vision::SceneResult::unknown(),
            )?;
            let mut faces: Vec<_> = faces
                .into_iter()
                .filter(|f| {
                    f.x < tensor.resized_width() as f32
                        && f.y < tensor.resized_height() as f32
                        && f.x + f.width > 0.
                        && f.y + f.height > 0.
                })
                .collect();
            faces.sort_by(|a, b| {
                a.x.total_cmp(&b.x)
                    .then(a.y.total_cmp(&b.y))
                    .then(b.confidence.total_cmp(&a.confidence))
            });
            for t in &mut self.tracks {
                t.visible = false;
            }
            for face in faces {
                let sx = frame.width() as f32 / tensor.resized_width() as f32;
                let sy = frame.height() as f32 / tensor.resized_height() as f32;
                let bounds = FaceBox {
                    landmarks: face.landmarks.map(|p| p.map(|[x, y]| [x * sx, y * sy])),
                    x: face.x * sx,
                    y: face.y * sy,
                    width: face.width * sx,
                    height: face.height * sy,
                    confidence: face.confidence,
                };
                let best = self
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| !t.visible && at.0 - t.seen.0 <= TICKS_PER_SECOND * 2 / 5)
                    .map(|(i, t)| (i, iou(&t.bounds, &bounds)))
                    .filter(|(_, score)| *score >= 0.3)
                    .max_by(|(a, x), (b, y)| x.total_cmp(y).then(b.cmp(a)))
                    .map(|(i, _)| i);
                let track_index = best.unwrap_or(self.tracks.len());
                if let Some(i) = best {
                    let track = self.tracks.get_mut(i).ok_or(EditError::Limit)?;
                    track.bounds = bounds;
                    track.seen = at;
                    track.visible = true;
                } else {
                    if self.tracks.len() == 64 {
                        return Err(EditError::Limit);
                    }
                    self.tracks.push(Track {
                        embeddings: Vec::new(),
                        embedding_attempts: 0,
                        bounds,
                        seen: at,
                        visible: true,
                        observed: 0,
                        segments: Vec::new(),
                        score: 0.,
                        scored: 0,
                    });
                }
                let track = self.tracks.get_mut(track_index).ok_or(EditError::Limit)?;
                if track.embedding_attempts < 3 && track.observed >= track.embedding_attempts * 25 {
                    track.embedding_attempts += 1;
                    if let Some(points) = track.bounds.landmarks
                        && let Some(tensor) = frame.identity_tensor(points)?
                        && let Some(embedding) =
                            detector.embedding(IdentityFrame::new(&tensor)?, &VisionHooks(ctx))?
                    {
                        track.embeddings.push(embedding);
                    }
                }
            }
            let bin = self.bins.get_mut(bin_index).ok_or(EditError::Limit)?;
            if self.tracks.iter().any(|t| t.visible) {
                bin.scene = Scene::Camera;
            } else if bin.scene != Scene::Camera && !bin.classified {
                let values = frame.scene_tensor()?;
                let scene = detector.scene(SceneFrame::new(&values)?, &VisionHooks(ctx))?;
                ShotResult::new(
                    Vec::new(),
                    ai::vision::ActiveSpeaker::NoVisibleSpeaker,
                    scene,
                )?;
                bin.scene = match scene.kind {
                    SceneKind::Screen => Scene::Screen,
                    SceneKind::Camera => Scene::Camera,
                    SceneKind::Unknown => Scene::Unknown,
                };
                bin.classified = true;
            }
        }
        let bin = self.bins.get_mut(bin_index).ok_or(EditError::Limit)?;
        bin.samples = bin.samples.checked_add(1).ok_or(EditError::Limit)?;
        for (i, track) in self
            .tracks
            .iter_mut()
            .enumerate()
            .filter(|(_, t)| t.visible)
        {
            ctx.check_cancelled()?;
            track.observed += 1;
            let presence = bin.presence.get_mut(i).ok_or(EditError::Limit)?;
            *presence = presence.checked_add(1).ok_or(EditError::Limit)?;
            let crop = frame.face_crop(
                media::FaceBox {
                    x: f64::from(track.bounds.x),
                    y: f64::from(track.bounds.y),
                    width: f64::from(track.bounds.width),
                    height: f64::from(track.bounds.height),
                },
                &VisionPrepLimits::default(),
            )?;
            let contiguous = track
                .segments
                .last()
                .and_then(|s| s.last())
                .is_some_and(|(last, _)| at.0 - last.0 == VIDEO_STEP);
            if !contiguous {
                track.segments.push(Vec::new());
            }
            let segment = track.segments.last_mut().ok_or(EditError::Limit)?;
            if segment.len() >= 75 {
                return Err(EditError::Limit);
            }
            segment.push((at, crop.pixels().iter().map(|v| f32::from(*v)).collect()));
        }
        self.frame += 1;
        Ok(())
    }
    pub(super) fn flush(
        &mut self,
        mfcc: &[media::MfccFrame],
        detector: &mut dyn VisionDetector,
        ctx: &JobContext,
    ) -> Result<(), EditError> {
        for feature in mfcc {
            let index =
                ((feature.source_start.0 - self.range.start().0) / TICKS_PER_SECOND) as usize;
            self.bins.get_mut(index).ok_or(EditError::Limit)?.audio = true;
        }
        for (id, track) in self.tracks.iter_mut().enumerate() {
            for segment in std::mem::take(&mut track.segments) {
                ctx.check_cancelled()?;
                let mut video = Vec::new();
                let mut audio = Vec::new();
                let mut ticks = Vec::new();
                // Only contiguous exact evidence is submitted. No tail padding,
                // no audio from a different grid/shot, and no gap interpolation.
                for (at, crop) in segment {
                    let Ok(offset) = mfcc.binary_search_by_key(&at, |f| f.source_start) else {
                        if ticks.is_empty() {
                            continue;
                        } else {
                            break;
                        }
                    };
                    let Some(features) = mfcc.get(offset..offset + 4) else {
                        break;
                    };
                    if features
                        .iter()
                        .enumerate()
                        .any(|(i, f)| f.source_start.0 != at.0 + i as i64 * AUDIO_STEP)
                    {
                        break;
                    }
                    video.extend_from_slice(&crop);
                    for feature in features {
                        audio.extend_from_slice(&feature.coefficients);
                    }
                    ticks.push(at);
                }
                if ticks.len() < 25 {
                    continue;
                }
                let probabilities = detector.speaker(
                    SpeakerChunk::new(ticks.len(), &audio, &video)?,
                    &VisionHooks(ctx),
                )?;
                if probabilities.len() != ticks.len()
                    || probabilities
                        .iter()
                        .any(|p| !p.is_finite() || !(0.0..=1.).contains(p))
                {
                    return Err(ai::Error::VisionShape.into());
                }
                ctx.check_cancelled()?;
                for (i, at) in ticks.iter().enumerate() {
                    let smooth = probabilities
                        .get(i.saturating_sub(2)..(i + 3).min(probabilities.len()))
                        .ok_or(EditError::Limit)?;
                    let score =
                        smooth.iter().map(|v| f64::from(*v)).sum::<f64>() / smooth.len() as f64;
                    track.score += score;
                    track.scored += 1;
                    let bin = self
                        .bins
                        .get_mut(((at.0 - self.range.start().0) / TICKS_PER_SECOND) as usize)
                        .ok_or(EditError::Limit)?;
                    *bin.scores.get_mut(id).ok_or(EditError::Limit)? += score;
                    let count = bin.scored.get_mut(id).ok_or(EditError::Limit)?;
                    *count = count.checked_add(1).ok_or(EditError::Limit)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn finish(
        self,
        speech: &[SourceRange],
        audio: bool,
    ) -> Result<(ShotAnalysis, super::reidentification::Evidence), EditError> {
        let tracks: Vec<_> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.observed >= 13)
            .map(|(i, t)| FaceTrack {
                identity: None,
                id: i as u32,
                bounds: [t.bounds.x, t.bounds.y, t.bounds.width, t.bounds.height],
                observed_frames: t.observed,
            })
            .collect();
        let mut seconds = Vec::with_capacity(self.bins.len());
        let mut previous = None;
        for (i, bin) in self.bins.iter().enumerate() {
            let start = Tick(self.range.start().0 + i as i64 * TICKS_PER_SECOND);
            let end = Tick((start.0.saturating_add(TICKS_PER_SECOND)).min(self.range.end().0));
            let range = SourceRange::new(start, end)?;
            let count = tracks
                .iter()
                .filter(|t| {
                    bin.presence
                        .get(t.id as usize)
                        .is_some_and(|n| *n >= bin.samples.clamp(1, 13))
                })
                .count();
            let scores: Vec<_> = tracks
                .iter()
                .filter_map(|t| {
                    let n = *bin.scored.get(t.id as usize)?;
                    (n > 0).then(|| {
                        (
                            t.id,
                            bin.scores.get(t.id as usize).copied().unwrap_or(0.) / f64::from(n),
                        )
                    })
                })
                .collect();
            let speaker = state(
                self.shot,
                &scores,
                previous,
                audio && bin.audio && overlaps(speech, range),
                bin.samples > 0
                    && bin
                        .presence
                        .iter()
                        .zip(&bin.scored)
                        .all(|(visible, scored)| scored >= visible),
            );
            previous = match speaker {
                SpeakerState::Visible { local, .. } => Some(local.track),
                _ => None,
            };
            seconds.push(SecondAnalysis {
                range,
                people_count: count,
                speaker,
            });
        }
        let mut counts: Vec<_> = seconds.iter().map(|s| s.people_count).collect();
        counts.sort_unstable();
        let people_count = counts.get(counts.len() / 2).copied().unwrap_or(0);
        let scores: Vec<_> = tracks
            .iter()
            .filter_map(|t| {
                self.tracks
                    .get(t.id as usize)
                    .filter(|t| t.scored > 0)
                    .map(|v| (t.id, v.score / v.scored as f64))
            })
            .collect();
        let speaker = state(
            self.shot,
            &scores,
            None,
            seconds
                .iter()
                .any(|second| second.speaker == SpeakerState::OffScreenSpeech),
            self.frame > 0
                && self
                    .tracks
                    .iter()
                    .all(|track| track.scored == track.observed),
        );
        let evidence = tracks
            .iter()
            .filter_map(|t| {
                self.tracks
                    .get(t.id as usize)
                    .filter(|v| !v.embeddings.is_empty())
                    .map(|v| (t.id, v.embeddings.clone()))
            })
            .collect();
        Ok((
            ShotAnalysis {
                range: self.range,
                people_count,
                tracks,
                speaker,
                continuity_hint: None,
                scene: {
                    let camera = self
                        .bins
                        .iter()
                        .filter(|b| b.scene == Scene::Camera)
                        .count();
                    let screen = self
                        .bins
                        .iter()
                        .filter(|b| b.scene == Scene::Screen)
                        .count();
                    if camera == 0 && screen == 0 {
                        Scene::Unknown
                    } else if screen > camera {
                        Scene::Screen
                    } else {
                        Scene::Camera
                    }
                },
                seconds,
            },
            evidence,
        ))
    }
}
fn overlaps(speech: &[SourceRange], range: SourceRange) -> bool {
    let first = speech.partition_point(|r| r.end() <= range.start());
    speech.get(first).is_some_and(|r| r.start() < range.end())
}
fn state(
    shot: u32,
    scores: &[(u32, f64)],
    previous: Option<u32>,
    speech: bool,
    negative_evidence: bool,
) -> SpeakerState {
    let best = scores
        .iter()
        .filter(|(id, p)| *p >= if previous == Some(*id) { 0.4 } else { 0.5 })
        .max_by(|(a, x), (b, y)| x.total_cmp(y).then(b.cmp(a)));
    if let Some((track, p)) = best {
        SpeakerState::Visible {
            local: SpeakerRef {
                shot,
                track: *track,
            },
            probability: *p as f32,
        }
    } else if speech && negative_evidence {
        SpeakerState::OffScreenSpeech
    } else {
        SpeakerState::Unknown
    }
}
fn iou(a: &FaceBox, b: &FaceBox) -> f32 {
    let area = ((a.x + a.width).min(b.x + b.width) - a.x.max(b.x)).max(0.)
        * ((a.y + a.height).min(b.y + b.height) - a.y.max(b.y)).max(0.);
    area / (a.width * a.height + b.width * b.height - area).max(f32::MIN_POSITIVE)
}

// Keep the engine's Preparing/Analyzing stages instead of replacing them with
// the adapter's generic Vision stage at every inference checkpoint.
struct VisionHooks<'a>(&'a JobContext);
impl ai::ExecutionHooks for VisionHooks<'_> {
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
    fn stage(&self, _: ai::Stage) {}
    fn cancelling(&self) {
        ai::ExecutionHooks::cancelling(self.0);
    }
}
