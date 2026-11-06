//! Admits export only when requested loudness normalization was measured for exactly this audio.
use crate::EditError;
use project::{ItemParams, Project, ProjectData};

/// Version of the measurement recipe; part of every content key and cache key.
pub(crate) const VERSION: &str =
    "loudness-v1:bs1770-4-kweight:gate-70-rel10:400ms-hop100ms:truepeak-polyphase";

/// Hash of everything that changes audible output or the requested targets:
/// the active scene's audible items with their volume/mute automation, track
/// mutes, sample rate and the loudness targets. Master fader and visual edits are
/// excluded because measurement ignores them.
pub fn content_key(data: &ProjectData) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(VERSION.as_bytes());
    hash.update(&data.settings.audio_sample_rate.to_le_bytes());
    for target in [data.settings.loudness.clip, data.settings.loudness.master] {
        match target {
            Some(t) => {
                hash.update(&[1]);
                hash.update(&t.target_lufs.to_bits().to_le_bytes());
                hash.update(&t.true_peak_db.to_bits().to_le_bytes());
            }
            None => {
                hash.update(&[0]);
            }
        }
    }
    hash.update(&data.scene().id.0.to_le_bytes());
    for track in data.scene().timeline.iter() {
        hash.update(&track.id.0.to_le_bytes());
        hash.update(&[u8::from(track.muted)]);
        for item in &track.items {
            let (volume, muted, source_audio) = match &item.content {
                ItemParams::Audio(p) => (p.volume, p.muted, true),
                ItemParams::Video(p) => (p.volume, p.muted, p.source_audio_enabled),
                _ => continue,
            };
            hash.update(&item.id.0.to_le_bytes());
            hash.update(&item.start.0.to_le_bytes());
            hash.update(&item.source_range.start().0.to_le_bytes());
            hash.update(&item.source_range.end().0.to_le_bytes());
            hash.update(&item.source_asset.map_or(0, |a| a.0).to_le_bytes());
            if let Some(asset) = item
                .source_asset
                .and_then(|id| data.assets.iter().find(|a| a.id() == id))
            {
                hash.update(&asset.content_hash().0);
            }
            hash.update(&volume.to_bits().to_le_bytes());
            hash.update(&[u8::from(muted), u8::from(source_audio)]);
            for (path, param) in item.animations.iter() {
                if matches!(path.as_str(), "volume" | "muted") {
                    hash.update(path.as_str().as_bytes());
                    // Param serialization is infallible for validated projects.
                    hash.update(&serde_json::to_vec(param).unwrap_or_default());
                }
            }
        }
    }
    *hash.finalize().as_bytes()
}

/// Export admission: requested normalization needs gains measured for exactly
/// this audio content, so preview and export cannot disagree.
pub fn admit(project: &Project) -> Result<(), EditError> {
    let l = &project.data().settings.loudness;
    if l.clip.is_none() && l.master.is_none() {
        return Ok(());
    }
    match &l.measured {
        Some(m) if m.key == content_key(project.data()) => Ok(()),
        _ => Err(EditError::LoudnessStale),
    }
}
