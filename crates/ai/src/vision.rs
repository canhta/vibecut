//! Bounded prepared-tensor detector seam. Media owns pixels/crops/MFCC preparation;
//! engine owns shot detection, tracking, scheduling and persistent artifact caches.
use crate::{Error, ExecutionHooks};
#[cfg(feature = "vision")]
mod tract;
#[cfg(feature = "vision")]
pub use tract::{TractVision, VisionPaths};
mod decisions;
pub use decisions::{aggregate_speakers, classify_embedding};

/// Content key for engine's analysis artifacts, excluding paths and media data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VisionFingerprint([u8; 32]);
impl VisionFingerprint {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl std::fmt::Display for VisionFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub const FACE_WIDTH: usize = 640;
pub const FACE_HEIGHT: usize = 384;
pub const CLIP_SIDE: usize = 224;
pub const MAX_FACES: usize = 256;
pub const MAX_TRACKS: usize = 64;
pub const MAX_SHOT_FRAMES: usize = 9_000;
pub const MAX_CHUNKS: usize = 512;
pub const RECIPE: &str = "vision-v1:yunet-bgr640x384-score.6-iou.3:asd25fps-mfcc4to1-smooth2-threshold.4:clip-rgb224-logit14.3";
pub const PROMPTS: [&str; 2] = [
    "a screenshot of a computer screen",
    "a photograph of a person",
];

fn finite(values: &[f32]) -> Result<(), Error> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(Error::VisionNonFinite)
    }
}
fn sized(values: &[f32], count: usize) -> Result<(), Error> {
    if values.len() != count {
        return Err(Error::VisionShape);
    }
    finite(values)
}
/// CHW BGR, 640x384, unscaled pixel values 0..=255, letterbox at bottom/right.
/// Boxes returned by the detector use this prepared coordinate system.
#[derive(Clone, Copy)]
pub struct FaceFrame<'a>(&'a [f32]);
impl<'a> FaceFrame<'a> {
    pub fn values(&self) -> &[f32] {
        self.0
    }
    pub fn new(values: &'a [f32]) -> Result<Self, Error> {
        sized(values, 3 * FACE_WIDTH * FACE_HEIGHT)?;
        if values.iter().any(|v| !(0.0..=255.0).contains(v)) {
            return Err(Error::VisionShape);
        }
        Ok(Self(values))
    }
}
/// CHW RGB 224x224 after CLIP resize/center crop and pinned mean/std normalization.
#[derive(Clone, Copy)]
pub struct SceneFrame<'a>(&'a [f32]);
impl<'a> SceneFrame<'a> {
    pub fn values(&self) -> &[f32] {
        self.0
    }
    pub fn new(values: &'a [f32]) -> Result<Self, Error> {
        sized(values, 3 * CLIP_SIDE * CLIP_SIDE)?;
        if values.iter().any(|v| v.abs() > 4.0) {
            return Err(Error::VisionShape);
        }
        Ok(Self(values))
    }
}
/// Exact 25..=75 grayscale 112x112 frames (25 fps, pixels 0..=255) and
/// 4 MFCC frames per video frame, 13 coefficients (100 Hz). Never padded here.
#[derive(Clone, Copy)]
pub struct SpeakerChunk<'a> {
    frames: usize,
    audio: &'a [f32],
    video: &'a [f32],
}
impl<'a> SpeakerChunk<'a> {
    pub fn new(frames: usize, audio: &'a [f32], video: &'a [f32]) -> Result<Self, Error> {
        if !(25..=75).contains(&frames) {
            return Err(Error::VisionShape);
        }
        sized(audio, frames * 4 * 13)?;
        sized(video, frames * 112 * 112)?;
        if video.iter().any(|v| !(0.0..=255.0).contains(v)) || audio.iter().any(|v| v.abs() > 1e6) {
            return Err(Error::VisionShape);
        }
        Ok(Self {
            frames,
            audio,
            video,
        })
    }
    pub fn audio(&self) -> &[f32] {
        self.audio
    }
    pub fn video(&self) -> &[f32] {
        self.video
    }
    pub fn frames(&self) -> usize {
        self.frames
    }
}
/// Finite box in the prepared 640x384 plane; edges can extend outside the plane.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneKind {
    Screen,
    Camera,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneResult {
    pub kind: SceneKind,
    pub screen_probability: Option<f32>,
}
impl SceneResult {
    pub fn unknown() -> Self {
        Self {
            kind: SceneKind::Unknown,
            screen_probability: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActiveSpeaker {
    Visible { track: u32, probability: f32 },
    NoVisibleSpeaker,
}
/// One chunk's speaker probabilities in shot-local video frame coordinates.
/// Overlap within a track is averaged; duplicate chunks are refused.
pub struct TrackScores<'a> {
    pub track: u32,
    pub start_frame: usize,
    pub probabilities: &'a [f32],
}
/// Bounded shot artifact; frame indices are shot-local, track identity is shot-local.
/// Cross-shot speaker identity is owned by engine, never inferred by this adapter.
#[derive(Debug, PartialEq)]
pub struct ShotResult {
    faces: Vec<FaceBox>,
    active_speaker: ActiveSpeaker,
    scene: SceneResult,
}
/// Blocking, local inference only. Each call checks cancellation before/after
/// plan construction and every inference; a running tract call drains before
/// cancellation discards its output. The caller serializes access per instance.
pub trait VisionDetector: Send {
    fn faces(
        &mut self,
        frame: FaceFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<FaceBox>, Error>;
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<f32>, Error>;
    fn scene(
        &mut self,
        frame: SceneFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<SceneResult, Error>;
}

impl ShotResult {
    pub fn new(
        faces: Vec<FaceBox>,
        active_speaker: ActiveSpeaker,
        scene: SceneResult,
    ) -> Result<Self, Error> {
        if faces.len() > MAX_FACES {
            return Err(Error::VisionShape);
        }
        for face in &faces {
            finite(&[face.x, face.y, face.width, face.height, face.confidence])?;
            if face.width <= 0. || face.height <= 0. || !(0.0..=1.0).contains(&face.confidence) {
                return Err(Error::VisionShape);
            }
        }
        if let ActiveSpeaker::Visible { probability, .. } = active_speaker {
            finite(&[probability])?;
            if !(0.4..=1.0).contains(&probability) {
                return Err(Error::VisionShape);
            }
        }
        if let Some(probability) = scene.screen_probability {
            finite(&[probability])?;
        }
        match (scene.kind, scene.screen_probability) {
            (SceneKind::Unknown, None) => {}
            (SceneKind::Screen | SceneKind::Camera, Some(p))
                if p.is_finite()
                    && (0.0..=1.0).contains(&p)
                    && ((p >= 0.5) == (scene.kind == SceneKind::Screen)) => {}
            _ => return Err(Error::VisionShape),
        }
        Ok(Self {
            faces,
            active_speaker,
            scene,
        })
    }
    pub fn faces(&self) -> &[FaceBox] {
        &self.faces
    }
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
    pub fn active_speaker(&self) -> ActiveSpeaker {
        self.active_speaker
    }
    pub fn scene(&self) -> SceneResult {
        self.scene
    }
}
