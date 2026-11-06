//! Bounded media preparation for local vision analysis.
//!
//! Decodes source frames at exact presentation times and prepares the face crops,
//! detector tensors and 16 kHz mono MFCC frames used by the local vision
//! preprocessing recipe, with caller-bounded work slices and bounded memory.
//! Detector inference, shot tracking and job orchestration belong to their
//! owners; this module only prepares media.

mod face;
mod frame;
mod limits;
mod mfcc;
mod scene;

pub use face::{FACE_CROP_SIZE, FaceBox, FaceCrop, FaceTensor};
pub use frame::{VisionFrame, decode_frame_at, decode_shot_frames};
pub use limits::VisionPrepLimits;
pub use mfcc::{
    MFCC_COEFFICIENTS, MfccFrame, MfccProgress, VISION_SAMPLE_RATE, mfcc_range,
    mfcc_range_with_progress,
};
