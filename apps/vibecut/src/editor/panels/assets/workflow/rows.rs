//! Projects asset metadata into display rows.

use crate::editor::assets::{
    AssetRow, AssetState, Color32, Icon, Locale, MediaKind, SourceAsset, StreamKind, Text,
    TrackKind, theme,
};
use crate::i18n;

pub(in crate::editor::panels::assets) fn asset_kind(
    asset: &SourceAsset,
) -> std::result::Result<TrackKind, engine::control::Error> {
    if asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Video)
    {
        Ok(TrackKind::Video)
    } else if asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Audio)
    {
        Ok(TrackKind::Audio)
    } else {
        Err(engine::control::Error::Edit(
            engine::EditError::Unsupported("media has no video or audio stream"),
        ))
    }
}
pub(in crate::editor::panels::assets) fn asset_state_color(
    state: AssetState,
    dark: bool,
) -> Color32 {
    match state {
        AssetState::Probing => theme::secondary(dark),
        AssetState::Ready => theme::success(dark),
        AssetState::Unsupported(_) => theme::warning(dark),
    }
}
pub(in crate::editor::panels::assets) fn asset_row(
    asset: &SourceAsset,
    locale: Locale,
    size: Option<u64>,
    state: AssetState,
) -> AssetRow {
    let mut info = Vec::new();
    let video = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref());
    let audio = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.audio.as_ref());
    let media_kind = MediaKind::of(asset);
    let (kind, icon) = match media_kind {
        MediaKind::Image => (Text::Image, Icon::File),
        MediaKind::Video => (Text::Video, Icon::FilmStrip),
        MediaKind::Audio => (Text::Audio, Icon::MusicNotes),
        MediaKind::Other => (Text::Media, Icon::File),
    };
    let type_label = i18n::text(locale, kind).to_owned();
    info.push(type_label);
    info.push(asset.probe().duration.map_or_else(
        || i18n::text(locale, Text::DurationUnknown).to_owned(),
        time::format_clock,
    ));
    if let Some(size) = size {
        let size = format_media_size(locale, size);
        info.push(size);
    }
    if let Some(video) = video {
        let width = i18n::number(locale, u64::from(video.width));
        let height = i18n::number(locale, u64::from(video.height));
        info.push(i18n::format(
            locale,
            Text::AssetDimensions,
            &[("width", &width), ("height", &height)],
        ));
    }
    if let Some(rate) = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
    {
        let value = if rate.den() == 1 {
            i18n::signed(locale, rate.num())
        } else {
            i18n::decimal(locale, rate.num() as f64 / rate.den() as f64, 3)
        };
        info.push(i18n::format(locale, Text::AssetFps, &[("rate", &value)]));
    }
    if let Some(audio) = audio {
        let rate = i18n::number(locale, u64::from(audio.sample_rate));
        let channels = i18n::number(locale, u64::from(audio.channels));
        info.push(i18n::format(
            locale,
            Text::AssetAudio,
            &[("rate", &rate), ("channels", &channels)],
        ));
    }
    AssetRow {
        id: asset.id(),
        name: file_name(asset.path()),
        info: info.join(" · "),
        icon,
        state,
        duration: asset.probe().duration,
        missing: false,
    }
}
pub(in crate::editor::panels::assets) fn format_media_size(locale: Locale, bytes: u64) -> String {
    let (value, unit) = if bytes >= 1 << 30 {
        (
            i18n::decimal(locale, bytes as f64 / (1u64 << 30) as f64, 1),
            "GB",
        )
    } else if bytes >= 1 << 20 {
        (
            i18n::decimal(locale, bytes as f64 / (1u64 << 20) as f64, 1),
            "MB",
        )
    } else if bytes >= 1 << 10 {
        (
            i18n::decimal(locale, bytes as f64 / (1u64 << 10) as f64, 1),
            "KB",
        )
    } else {
        (i18n::number(locale, bytes), "B")
    };
    i18n::format(locale, Text::MediaSize, &[("size", &value), ("unit", unit)])
}
pub(in crate::editor::panels::assets) fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}
