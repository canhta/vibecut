//! Media library presentation state: view mode, stable sorting and type
//! classification. Project assets stay owned by the engine; this module never
//! mutates a `SourceAsset` and reads file size only for display/sort.
use media::{AssetId, SourceAsset, StreamKind};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use time::Tick;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaView {
    #[default]
    Grid,
    List,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaSortKey {
    #[default]
    Name,
    Type,
    Duration,
    Size,
}
impl MediaSortKey {
    pub const ALL: [Self; 4] = [Self::Name, Self::Type, Self::Duration, Self::Size];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Type => "type",
            Self::Duration => "duration",
            Self::Size => "size",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.id() == id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaSortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaSort {
    pub key: MediaSortKey,
    pub order: MediaSortOrder,
}

/// Source type order matches the reference's lexicographic `audio, image, video`
/// order; `Other` sorts last so malformed/unknown probes never lead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaKind {
    Audio,
    Image,
    Video,
    Other,
}

impl MediaKind {
    pub fn of(asset: &SourceAsset) -> Self {
        if asset.is_image() {
            return Self::Image;
        }
        let mut video = false;
        let mut audio = false;
        for stream in &asset.probe().streams {
            match stream.kind {
                StreamKind::Video => video = true,
                StreamKind::Audio => audio = true,
                StreamKind::Other => {}
            }
        }
        if video {
            Self::Video
        } else if audio {
            Self::Audio
        } else {
            Self::Other
        }
    }
}

/// The next dropdown click: the active key flips order, a new key starts ascending.
pub fn next_sort(current: MediaSort, key: MediaSortKey) -> MediaSort {
    if current.key == key {
        MediaSort {
            key,
            order: match current.order {
                MediaSortOrder::Asc => MediaSortOrder::Desc,
                MediaSortOrder::Desc => MediaSortOrder::Asc,
            },
        }
    } else {
        MediaSort {
            key,
            order: MediaSortOrder::Asc,
        }
    }
}

/// Case-insensitive file name; absolute paths are validated by the media owner.
pub fn asset_name(asset: &SourceAsset) -> String {
    asset.path().file_name().map_or_else(
        || asset.path().to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Stable project-order-preserving sort. Equal keys keep their asset-vector
/// order, which makes repeated sorts deterministic and independent of locale.
pub fn ordered_assets(
    assets: &[SourceAsset],
    sort: MediaSort,
    sizes: &BTreeMap<AssetId, Option<u64>>,
) -> Vec<AssetId> {
    let mut ordered: Vec<&SourceAsset> = assets.iter().filter(|asset| !asset.ephemeral()).collect();
    ordered.sort_by(|a, b| {
        let ordering = match sort.key {
            MediaSortKey::Name => asset_name(a)
                .to_lowercase()
                .cmp(&asset_name(b).to_lowercase()),
            MediaSortKey::Type => MediaKind::of(a).cmp(&MediaKind::of(b)),
            MediaSortKey::Duration => a
                .probe()
                .duration
                .unwrap_or(Tick::ZERO)
                .cmp(&b.probe().duration.unwrap_or(Tick::ZERO)),
            MediaSortKey::Size => {
                let size = |asset: &SourceAsset| {
                    sizes
                        .get(&asset.id())
                        .copied()
                        .flatten()
                        .unwrap_or_default()
                };
                size(a).cmp(&size(b))
            }
        };
        match sort.order {
            MediaSortOrder::Asc => ordering,
            MediaSortOrder::Desc => ordering.reverse(),
        }
    });
    ordered.into_iter().map(SourceAsset::id).collect()
}

/// File size is display metadata, not part of the project record.
pub fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|metadata| metadata.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use media::{
        AssetData, AudioInfo, ColorInfo, ContentHash, SourceProbe, SourceStream, TimeBase,
        VideoInfo,
    };
    use time::FrameRate;

    fn stream(kind: StreamKind, frame_count: Option<u64>) -> SourceStream {
        SourceStream {
            index: 0,
            kind,
            codec: "test".into(),
            time_base: TimeBase::new(1, 1_000).unwrap(),
            duration: Some(Tick(10_000)),
            start: None,
            frame_rate: (kind == StreamKind::Video).then_some(FrameRate::FPS_30),
            frame_count,
            variable_frame_rate: None,
            video: (kind == StreamKind::Video).then_some(VideoInfo {
                width: 1920,
                height: 1080,
                color: ColorInfo::default(),
                bit_depth: Some(8),
            }),
            audio: (kind == StreamKind::Audio).then_some(AudioInfo {
                sample_rate: 48_000,
                channels: 2,
                initial_padding: 0,
                trailing_padding: 0,
            }),
        }
    }

    fn asset(id: u64, name: &str, duration: Tick, streams: Vec<SourceStream>) -> SourceAsset {
        asset_format(id, name, "mp4", duration, streams)
    }

    fn asset_format(
        id: u64,
        name: &str,
        format: &str,
        duration: Tick,
        streams: Vec<SourceStream>,
    ) -> SourceAsset {
        SourceAsset::try_from(AssetData {
            ephemeral: false,
            id: AssetId(id),
            path: std::env::temp_dir().join(name),
            content_hash: ContentHash([id as u8; 32]),
            probe: SourceProbe {
                format: format.into(),
                duration: Some(duration),
                streams,
            },
        })
        .unwrap()
    }

    fn ids(assets: &[SourceAsset], sort: MediaSort) -> Vec<AssetId> {
        ordered_assets(assets, sort, &BTreeMap::new())
    }

    #[test]
    fn equal_keys_keep_project_order_in_both_directions() {
        let name = format!("{} Tiếng Việt 🎬.mp4", "a".repeat(4096));
        let assets = [
            asset(1, &name, Tick(10), vec![stream(StreamKind::Video, None)]),
            asset(
                2,
                &name.to_lowercase(),
                Tick(10),
                vec![stream(StreamKind::Video, None)],
            ),
            asset(
                3,
                &name.to_uppercase(),
                Tick(10),
                vec![stream(StreamKind::Video, None)],
            ),
        ];
        for order in [MediaSortOrder::Asc, MediaSortOrder::Desc] {
            assert_eq!(
                ids(
                    &assets,
                    MediaSort {
                        key: MediaSortKey::Name,
                        order
                    }
                ),
                vec![AssetId(1), AssetId(2), AssetId(3)]
            );
            assert_eq!(
                ids(
                    &assets,
                    MediaSort {
                        key: MediaSortKey::Duration,
                        order
                    }
                ),
                vec![AssetId(1), AssetId(2), AssetId(3)]
            );
        }
    }

    #[test]
    fn name_and_type_sort_are_deterministic_and_descending_reverses() {
        let assets = [
            asset(
                1,
                "Bravo.mp4",
                Tick(30),
                vec![stream(StreamKind::Video, None)],
            ),
            asset(
                2,
                "alpha.wav",
                Tick(10),
                vec![stream(StreamKind::Audio, None)],
            ),
            asset_format(
                3,
                "charlie.png",
                "png_pipe",
                Tick(20),
                vec![stream(StreamKind::Video, None)],
            ),
        ];
        assert_eq!(
            ids(
                &assets,
                MediaSort {
                    key: MediaSortKey::Name,
                    order: MediaSortOrder::Asc
                }
            ),
            vec![AssetId(2), AssetId(1), AssetId(3)]
        );
        assert_eq!(
            ids(
                &assets,
                MediaSort {
                    key: MediaSortKey::Type,
                    order: MediaSortOrder::Asc
                }
            ),
            vec![AssetId(2), AssetId(3), AssetId(1)]
        );
        assert_eq!(
            ids(
                &assets,
                MediaSort {
                    key: MediaSortKey::Duration,
                    order: MediaSortOrder::Desc
                }
            ),
            vec![AssetId(1), AssetId(3), AssetId(2)]
        );
    }

    #[test]
    fn size_sort_treats_missing_files_as_zero_and_flips() {
        let assets = [
            asset(
                1,
                "one.mp4",
                Tick(10),
                vec![stream(StreamKind::Video, None)],
            ),
            asset(
                2,
                "two.mp4",
                Tick(10),
                vec![stream(StreamKind::Video, None)],
            ),
            asset(
                3,
                "three.mp4",
                Tick(10),
                vec![stream(StreamKind::Video, None)],
            ),
            asset(
                4,
                "empty.mp4",
                Tick(10),
                vec![stream(StreamKind::Video, None)],
            ),
        ];
        let sizes = BTreeMap::from([
            (AssetId(1), Some(u64::MAX)),
            (AssetId(2), None),
            (AssetId(3), Some(1 << 63)),
            (AssetId(4), Some(0)),
        ]);
        assert_eq!(
            ordered_assets(
                &assets,
                MediaSort {
                    key: MediaSortKey::Size,
                    order: MediaSortOrder::Asc
                },
                &sizes
            ),
            vec![AssetId(2), AssetId(4), AssetId(3), AssetId(1)]
        );
        assert_eq!(
            ordered_assets(
                &assets,
                MediaSort {
                    key: MediaSortKey::Size,
                    order: MediaSortOrder::Desc
                },
                &sizes
            ),
            vec![AssetId(1), AssetId(3), AssetId(2), AssetId(4)]
        );
    }

    #[test]
    fn dropdown_click_flips_the_active_key_and_resets_another_key() {
        let name_asc = MediaSort::default();
        let name_desc = next_sort(name_asc, MediaSortKey::Name);
        assert_eq!(name_desc.order, MediaSortOrder::Desc);
        assert_eq!(next_sort(name_desc, MediaSortKey::Name), name_asc);
        assert_eq!(
            next_sort(name_desc, MediaSortKey::Size),
            MediaSort {
                key: MediaSortKey::Size,
                order: MediaSortOrder::Asc
            }
        );
    }

    #[test]
    fn media_kind_recognises_video_image_audio_and_empty_probes() {
        assert_eq!(
            MediaKind::of(&asset(
                1,
                "v.mp4",
                Tick(1),
                vec![stream(StreamKind::Video, None)]
            )),
            MediaKind::Video
        );
        assert_eq!(
            MediaKind::of(&asset_format(
                1,
                "i.png",
                "png_pipe",
                Tick(1),
                vec![stream(StreamKind::Video, None)]
            )),
            MediaKind::Image
        );
        assert_eq!(
            MediaKind::of(&asset(
                1,
                "a.wav",
                Tick(1),
                vec![stream(StreamKind::Audio, None)]
            )),
            MediaKind::Audio
        );
        assert_eq!(
            MediaKind::of(&asset(1, "x.bin", Tick(1), vec![])),
            MediaKind::Other
        );
    }
}
