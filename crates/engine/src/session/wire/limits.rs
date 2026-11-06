//! States the admission budgets that travel with the catalog.
use serde::Serialize;

pub const MAX_FRAME_BYTES: usize = 8 << 20;
pub const MAX_RESPONSE_BYTES: usize = 8 << 20;
pub const MAX_RECEIPTS: usize = 4096;
pub const MAX_RECEIPT_BYTES: usize = 32 << 20;
pub const RECEIPT_RESERVATION: usize = 128 << 10;
pub const MAX_EVENTS: usize = 128;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub const MAX_DEPTH: usize = 32;
pub const MAX_NODES: usize = 200_000;
pub const MAX_COLLECTION: usize = 100_000;
pub const MAX_TEXT_BYTES: usize = 4 << 20;
pub const MAX_BATCH_COMMANDS: usize = 1024;
pub const MAX_BATCH_DEPTH: usize = 16;
pub const MAX_KEY_BYTES: usize = 128;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_PREVIEW_FRAMES: usize = 4;
pub const MAX_PREVIEW_EDGE: u32 = 640;
pub const PREVIEW_QUALITY: u8 = 80;
pub const MAX_PREVIEW_CLIPS: usize = 16;

/// Static admission budgets travel with the catalog and participate in its hash.
#[derive(Clone, Copy, Debug, Serialize, schemars::JsonSchema)]
pub struct Limits {
    pub frame_bytes: usize,
    pub response_bytes: usize,
    pub depth: usize,
    pub nodes: usize,
    pub collection: usize,
    pub text_bytes: usize,
    pub batch_commands: usize,
    pub batch_depth: usize,
    pub key_bytes: usize,
    pub path_bytes: usize,
    pub receipts: usize,
    pub receipt_bytes: usize,
    pub receipt_reservation: usize,
    pub events: usize,
    pub preview_frames: usize,
    pub preview_edge: u32,
    pub preview_quality: u8,
    pub preview_clips: usize,
    pub safe_integer: i64,
}
pub const LIMITS: Limits = Limits {
    frame_bytes: MAX_FRAME_BYTES,
    response_bytes: MAX_RESPONSE_BYTES,
    depth: MAX_DEPTH,
    nodes: MAX_NODES,
    collection: MAX_COLLECTION,
    text_bytes: MAX_TEXT_BYTES,
    batch_commands: MAX_BATCH_COMMANDS,
    batch_depth: MAX_BATCH_DEPTH,
    key_bytes: MAX_KEY_BYTES,
    path_bytes: MAX_PATH_BYTES,
    receipts: MAX_RECEIPTS,
    receipt_bytes: MAX_RECEIPT_BYTES,
    receipt_reservation: RECEIPT_RESERVATION,
    events: MAX_EVENTS,
    preview_frames: MAX_PREVIEW_FRAMES,
    preview_edge: MAX_PREVIEW_EDGE,
    preview_quality: PREVIEW_QUALITY,
    preview_clips: MAX_PREVIEW_CLIPS,
    safe_integer: MAX_SAFE_INTEGER,
};
