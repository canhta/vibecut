//! Projects selected-item metadata for the inspector.
use super::{Editor, ElementKind, ItemId, ItemParams, StreamKind, TrackKind};

impl Editor {
    pub(in crate::editor) fn selected_item_id(&self) -> Option<ItemId> {
        if self.selection.len() != 1 {
            return None;
        }
        self.selection.iter().next().copied()
    }
    pub(in crate::editor) fn element_kind(&self, id: ItemId) -> Option<(ElementKind, bool)> {
        let data = self.session.project().data();
        let (track, item) = data.scene().timeline.iter().find_map(|track| {
            track
                .items
                .iter()
                .find(|item| item.id == id)
                .map(|item| (track, item))
        })?;
        let kind = match (&item.content, track.kind) {
            (ItemParams::Text(_) | ItemParams::Caption(_), _) => ElementKind::Text,
            (ItemParams::Audio(_), _) => ElementKind::Audio,
            (_, TrackKind::Sticker) => ElementKind::Sticker,
            (_, TrackKind::Effect) => ElementKind::Effect,
            (_, TrackKind::Filter | TrackKind::Adjustment) => ElementKind::Graphic,
            (ItemParams::Overlay(_), _) => ElementKind::Graphic,
            (ItemParams::Video(_), _) => {
                let Some(asset) = item
                    .source_asset
                    .and_then(|asset| data.assets.iter().find(|candidate| candidate.id() == asset))
                else {
                    return Some((ElementKind::Graphic, false));
                };
                let video = asset
                    .probe()
                    .streams
                    .iter()
                    .find(|stream| stream.kind == StreamKind::Video);
                match video {
                    Some(stream) if stream.frame_count == Some(1) => ElementKind::Image,
                    Some(_) => ElementKind::Video,
                    None => ElementKind::Image,
                }
            }
        };
        let has_audio = kind == ElementKind::Video
            && item.source_asset.is_some_and(|asset| {
                data.assets.iter().any(|candidate| {
                    candidate.id() == asset
                        && candidate
                            .probe()
                            .streams
                            .iter()
                            .any(|stream| stream.kind == StreamKind::Audio)
                })
            });
        Some((kind, has_audio))
    }
}
