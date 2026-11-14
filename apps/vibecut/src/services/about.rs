//! The facts the About dialog shows: version, terms links and third-party licence notices.

/// One third-party licence row: who wrote it, under which licence, and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) author: &'static str,
    pub(crate) licence: &'static str,
    pub(crate) source: &'static str,
}

/// The release version, from the workspace package.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Terms of use: the project licence text.
pub(crate) const TERMS_URL: &str = "https://github.com/canhta/vibecut/blob/main/LICENSE";
/// Privacy: the open-source policy's privacy section.
pub(crate) const PRIVACY_URL: &str =
    "https://github.com/canhta/vibecut/blob/main/docs/engineering/open-source.md#privacy";

/// FFmpeg is linked dynamically; the open-source policy names it with its licence.
const FFMPEG: Notice = Notice {
    author: "FFmpeg developers",
    licence: "LGPL-2.1-or-later (GPL-3.0 with libx264 on Linux)",
    source: "https://ffmpeg.org",
};

/// Notices generated at build time from every bundled asset's `.attribution` sidecar.
const ASSETS: &[Notice] = include!(concat!(env!("OUT_DIR"), "/licences.rs"));

/// Every third-party notice: FFmpeg first, then the bundled assets by author.
pub(crate) fn notices() -> impl Iterator<Item = Notice> {
    std::iter::once(FFMPEG).chain(ASSETS.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_name_ffmpeg_and_every_bundled_asset_licence() {
        let notices: Vec<_> = notices().collect();
        assert_eq!(notices.first(), Some(&FFMPEG));
        for licence in ["OFL-1.1", "MIT"] {
            assert!(
                notices.iter().any(|notice| notice.licence == licence),
                "{licence}"
            );
        }
        assert!(
            notices
                .iter()
                .all(|notice| !notice.author.is_empty() && !notice.source.is_empty())
        );
    }
}
