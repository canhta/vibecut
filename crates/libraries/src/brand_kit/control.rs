//! Projects the active brand kit onto the engine control view.
use super::Result;
use super::files::FileState;
use super::store::BrandKit;
use engine::control::{BrandColorView, BrandEntryState, BrandKitView, BrandMediaView};

impl BrandKit {
    /// The active kit as `brand.list` reports it, or `None` when no kit is active.
    pub fn active_view(&self) -> Result<Option<BrandKitView>> {
        let Some(active) = self.active()? else {
            return Ok(None);
        };
        let content = active.content;
        let media = content
            .media
            .into_iter()
            .zip(active.media)
            .map(|(media, state)| BrandMediaView {
                id: media.id,
                name: media.name,
                sha256: media.sha256,
                size: media.size,
                state: state.into(),
            })
            .collect();
        Ok(Some(BrandKitView {
            id: active.id,
            name: content.name,
            colors: content
                .colors
                .into_iter()
                .map(|color| BrandColorView {
                    name: color.name,
                    rgba: color.rgba,
                })
                .collect(),
            fonts: content.fonts,
            media,
            text_presets: content.text_presets,
            caption_presets: content.caption_presets,
        }))
    }
}

impl From<FileState> for BrandEntryState {
    fn from(state: FileState) -> Self {
        match state {
            FileState::Available => Self::Available,
            FileState::Missing => Self::Missing,
            FileState::Corrupt => Self::Corrupt,
        }
    }
}
