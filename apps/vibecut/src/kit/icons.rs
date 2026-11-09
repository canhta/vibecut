//! One embedded Phosphor regular icon catalog for the desktop frontend.
//!
//! The application owns the finite subset in `assets/icons/phosphor/regular`,
//! the `Icon` names, and draw-time tinting. `egui_extras`' SVG loader rasterizes
//! the prepared bytes; this module never reads files or installs UI state on its
//! own beyond [`install`].

use eframe::egui::{self, Color32, ImageSource, Vec2, load::Bytes};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

/// Every icon the application uses. Unknown names cannot reach a widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    ArrowClockwise,
    ArrowCounterClockwise,
    ArrowLeft,
    ArrowsHorizontal,
    ArrowsInLineHorizontal,
    ArrowsLeftRight,
    BookmarkSimple,
    CaretDown,
    CaretRight,
    CircleHalf,
    Copy,
    CornersOut,
    Crop,
    Diamond,
    DotsThree,
    Export,
    Eye,
    EyeSlash,
    File,
    FilmStrip,
    Funnel,
    FloppyDisk,
    FloppyDiskBack,
    FolderOpen,
    GearSix,
    House,
    LinkSimple,
    ListBullets,
    Lock,
    LockOpen,
    Magnet,
    MagnifyingGlass,
    MagnifyingGlassMinus,
    MagnifyingGlassPlus,
    MusicNotes,
    Pause,
    PencilSimple,
    Play,
    Plus,
    Scissors,
    SkipBack,
    SkipForward,
    SlidersHorizontal,
    Sparkle,
    SquaresFour,
    SpeakerHigh,
    SpeakerSlash,
    Sticker,
    Subtitles,
    TextT,
    Translate,
    Trash,
    UploadSimple,
    Waveform,
    X,
}

impl Icon {
    #[allow(dead_code)] // complete catalog list, used by the icon tests and future pickers.
    pub(crate) const ALL: &'static [Self] = &[
        Self::ArrowClockwise,
        Self::ArrowCounterClockwise,
        Self::ArrowLeft,
        Self::ArrowsHorizontal,
        Self::ArrowsInLineHorizontal,
        Self::ArrowsLeftRight,
        Self::BookmarkSimple,
        Self::CaretDown,
        Self::CaretRight,
        Self::CircleHalf,
        Self::Copy,
        Self::CornersOut,
        Self::Crop,
        Self::Diamond,
        Self::DotsThree,
        Self::Export,
        Self::Eye,
        Self::EyeSlash,
        Self::File,
        Self::FilmStrip,
        Self::Funnel,
        Self::FloppyDisk,
        Self::FloppyDiskBack,
        Self::FolderOpen,
        Self::GearSix,
        Self::House,
        Self::LinkSimple,
        Self::ListBullets,
        Self::Lock,
        Self::LockOpen,
        Self::Magnet,
        Self::MagnifyingGlass,
        Self::MagnifyingGlassMinus,
        Self::MagnifyingGlassPlus,
        Self::MusicNotes,
        Self::Pause,
        Self::PencilSimple,
        Self::Play,
        Self::Plus,
        Self::Scissors,
        Self::SkipBack,
        Self::SkipForward,
        Self::SlidersHorizontal,
        Self::Sparkle,
        Self::SquaresFour,
        Self::SpeakerHigh,
        Self::SpeakerSlash,
        Self::Sticker,
        Self::Subtitles,
        Self::TextT,
        Self::Translate,
        Self::Trash,
        Self::UploadSimple,
        Self::Waveform,
        Self::X,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::ArrowClockwise => "arrow-clockwise",
            Self::ArrowCounterClockwise => "arrow-counter-clockwise",
            Self::ArrowLeft => "arrow-left",
            Self::ArrowsHorizontal => "arrows-horizontal",
            Self::ArrowsInLineHorizontal => "arrows-in-line-horizontal",
            Self::ArrowsLeftRight => "arrows-left-right",
            Self::BookmarkSimple => "bookmark-simple",
            Self::CaretDown => "caret-down",
            Self::CaretRight => "caret-right",
            Self::CircleHalf => "circle-half",
            Self::Copy => "copy",
            Self::CornersOut => "corners-out",
            Self::Crop => "crop",
            Self::Diamond => "diamond",
            Self::DotsThree => "dots-three",
            Self::Export => "export",
            Self::Eye => "eye",
            Self::EyeSlash => "eye-slash",
            Self::File => "file",
            Self::Funnel => "funnel",
            Self::FilmStrip => "film-strip",
            Self::FloppyDisk => "floppy-disk",
            Self::FloppyDiskBack => "floppy-disk-back",
            Self::FolderOpen => "folder-open",
            Self::GearSix => "gear-six",
            Self::House => "house",
            Self::LinkSimple => "link-simple",
            Self::ListBullets => "list-bullets",
            Self::Lock => "lock",
            Self::LockOpen => "lock-open",
            Self::Magnet => "magnet",
            Self::MagnifyingGlass => "magnifying-glass",
            Self::MagnifyingGlassMinus => "magnifying-glass-minus",
            Self::MagnifyingGlassPlus => "magnifying-glass-plus",
            Self::MusicNotes => "music-notes",
            Self::Pause => "pause",
            Self::PencilSimple => "pencil-simple",
            Self::Play => "play",
            Self::Plus => "plus",
            Self::Scissors => "scissors",
            Self::SkipBack => "skip-back",
            Self::SkipForward => "skip-forward",
            Self::SlidersHorizontal => "sliders-horizontal",
            Self::Sparkle => "sparkle",
            Self::SquaresFour => "squares-four",
            Self::SpeakerHigh => "speaker-high",
            Self::SpeakerSlash => "speaker-slash",
            Self::Sticker => "sticker",
            Self::Subtitles => "subtitles",
            Self::TextT => "text-t",
            Self::Translate => "translate",
            Self::Trash => "trash",
            Self::UploadSimple => "upload-simple",
            Self::Waveform => "waveform",
            Self::X => "x",
        }
    }

    #[cfg(test)]
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|icon| icon.name() == name)
    }
}

/// Phosphor `regular` 2.1.1, unmodified. The last entry wins for duplicates.
const ICONS: &[(&str, &[u8])] = &[
    (
        "arrow-clockwise",
        include_bytes!("../../../../assets/icons/phosphor/regular/arrow-clockwise.svg"),
    ),
    (
        "arrow-counter-clockwise",
        include_bytes!("../../../../assets/icons/phosphor/regular/arrow-counter-clockwise.svg"),
    ),
    (
        "arrow-left",
        include_bytes!("../../../../assets/icons/phosphor/regular/arrow-left.svg"),
    ),
    (
        "arrows-horizontal",
        include_bytes!("../../../../assets/icons/phosphor/regular/arrows-horizontal.svg"),
    ),
    (
        "arrows-in-line-horizontal",
        include_bytes!("../../../../assets/icons/phosphor/regular/arrows-in-line-horizontal.svg"),
    ),
    (
        "arrows-left-right",
        include_bytes!("../../../../assets/icons/phosphor/regular/arrows-left-right.svg"),
    ),
    (
        "bookmark-simple",
        include_bytes!("../../../../assets/icons/phosphor/regular/bookmark-simple.svg"),
    ),
    (
        "caret-down",
        include_bytes!("../../../../assets/icons/phosphor/regular/caret-down.svg"),
    ),
    (
        "caret-right",
        include_bytes!("../../../../assets/icons/phosphor/regular/caret-right.svg"),
    ),
    (
        "circle-half",
        include_bytes!("../../../../assets/icons/phosphor/regular/circle-half.svg"),
    ),
    (
        "copy",
        include_bytes!("../../../../assets/icons/phosphor/regular/copy.svg"),
    ),
    (
        "corners-out",
        include_bytes!("../../../../assets/icons/phosphor/regular/corners-out.svg"),
    ),
    (
        "crop",
        include_bytes!("../../../../assets/icons/phosphor/regular/crop.svg"),
    ),
    (
        "diamond",
        include_bytes!("../../../../assets/icons/phosphor/regular/diamond.svg"),
    ),
    (
        "dots-three",
        include_bytes!("../../../../assets/icons/phosphor/regular/dots-three.svg"),
    ),
    (
        "export",
        include_bytes!("../../../../assets/icons/phosphor/regular/export.svg"),
    ),
    (
        "eye",
        include_bytes!("../../../../assets/icons/phosphor/regular/eye.svg"),
    ),
    (
        "eye-slash",
        include_bytes!("../../../../assets/icons/phosphor/regular/eye-slash.svg"),
    ),
    (
        "file",
        include_bytes!("../../../../assets/icons/phosphor/regular/file.svg"),
    ),
    (
        "funnel",
        include_bytes!("../../../../assets/icons/phosphor/regular/funnel.svg"),
    ),
    (
        "film-strip",
        include_bytes!("../../../../assets/icons/phosphor/regular/film-strip.svg"),
    ),
    (
        "floppy-disk",
        include_bytes!("../../../../assets/icons/phosphor/regular/floppy-disk.svg"),
    ),
    (
        "floppy-disk-back",
        include_bytes!("../../../../assets/icons/phosphor/regular/floppy-disk-back.svg"),
    ),
    (
        "folder-open",
        include_bytes!("../../../../assets/icons/phosphor/regular/folder-open.svg"),
    ),
    (
        "gear-six",
        include_bytes!("../../../../assets/icons/phosphor/regular/gear-six.svg"),
    ),
    (
        "house",
        include_bytes!("../../../../assets/icons/phosphor/regular/house.svg"),
    ),
    (
        "link-simple",
        include_bytes!("../../../../assets/icons/phosphor/regular/link-simple.svg"),
    ),
    (
        "list-bullets",
        include_bytes!("../../../../assets/icons/phosphor/regular/list-bullets.svg"),
    ),
    (
        "lock",
        include_bytes!("../../../../assets/icons/phosphor/regular/lock.svg"),
    ),
    (
        "lock-open",
        include_bytes!("../../../../assets/icons/phosphor/regular/lock-open.svg"),
    ),
    (
        "magnet",
        include_bytes!("../../../../assets/icons/phosphor/regular/magnet.svg"),
    ),
    (
        "magnifying-glass",
        include_bytes!("../../../../assets/icons/phosphor/regular/magnifying-glass.svg"),
    ),
    (
        "magnifying-glass-minus",
        include_bytes!("../../../../assets/icons/phosphor/regular/magnifying-glass-minus.svg"),
    ),
    (
        "magnifying-glass-plus",
        include_bytes!("../../../../assets/icons/phosphor/regular/magnifying-glass-plus.svg"),
    ),
    (
        "music-notes",
        include_bytes!("../../../../assets/icons/phosphor/regular/music-notes.svg"),
    ),
    (
        "pause",
        include_bytes!("../../../../assets/icons/phosphor/regular/pause.svg"),
    ),
    (
        "pencil-simple",
        include_bytes!("../../../../assets/icons/phosphor/regular/pencil-simple.svg"),
    ),
    (
        "play",
        include_bytes!("../../../../assets/icons/phosphor/regular/play.svg"),
    ),
    (
        "plus",
        include_bytes!("../../../../assets/icons/phosphor/regular/plus.svg"),
    ),
    (
        "scissors",
        include_bytes!("../../../../assets/icons/phosphor/regular/scissors.svg"),
    ),
    (
        "skip-back",
        include_bytes!("../../../../assets/icons/phosphor/regular/skip-back.svg"),
    ),
    (
        "skip-forward",
        include_bytes!("../../../../assets/icons/phosphor/regular/skip-forward.svg"),
    ),
    (
        "sliders-horizontal",
        include_bytes!("../../../../assets/icons/phosphor/regular/sliders-horizontal.svg"),
    ),
    (
        "sparkle",
        include_bytes!("../../../../assets/icons/phosphor/regular/sparkle.svg"),
    ),
    (
        "squares-four",
        include_bytes!("../../../../assets/icons/phosphor/regular/squares-four.svg"),
    ),
    (
        "speaker-high",
        include_bytes!("../../../../assets/icons/phosphor/regular/speaker-high.svg"),
    ),
    (
        "speaker-slash",
        include_bytes!("../../../../assets/icons/phosphor/regular/speaker-slash.svg"),
    ),
    (
        "sticker",
        include_bytes!("../../../../assets/icons/phosphor/regular/sticker.svg"),
    ),
    (
        "subtitles",
        include_bytes!("../../../../assets/icons/phosphor/regular/subtitles.svg"),
    ),
    (
        "text-t",
        include_bytes!("../../../../assets/icons/phosphor/regular/text-t.svg"),
    ),
    (
        "translate",
        include_bytes!("../../../../assets/icons/phosphor/regular/translate.svg"),
    ),
    (
        "trash",
        include_bytes!("../../../../assets/icons/phosphor/regular/trash.svg"),
    ),
    (
        "upload-simple",
        include_bytes!("../../../../assets/icons/phosphor/regular/upload-simple.svg"),
    ),
    (
        "waveform",
        include_bytes!("../../../../assets/icons/phosphor/regular/waveform.svg"),
    ),
    (
        "x",
        include_bytes!("../../../../assets/icons/phosphor/regular/x.svg"),
    ),
];

/// Explicit, visible fallback for the impossible case of an unregistered name.
const FALLBACK: &str = "file";
const URI_PREFIX: &str = "bytes://phosphor/2.1.1/regular/";

struct Catalog {
    icons: HashMap<&'static str, Arc<[u8]>>,
}

impl Catalog {
    fn new() -> Self {
        let icons = ICONS
            .iter()
            .map(|(name, bytes)| {
                let svg = String::from_utf8_lossy(bytes).replace("currentColor", "white");
                (*name, Arc::from(svg.into_bytes()))
            })
            .collect();
        Self { icons }
    }

    fn bytes(&self, name: &str) -> Arc<[u8]> {
        self.icons
            .get(name)
            .or_else(|| self.icons.get(FALLBACK))
            .cloned()
            .unwrap_or_default()
    }
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(Catalog::new)
}

/// Install the SVG rasterizer once per egui context.
pub(crate) fn install(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
}

/// Shared SVG bytes for one icon. Unknown names use the visible fallback bytes
/// but keep their requested cache key, so a bug never draws an empty square.
pub(crate) fn source(name: &str) -> ImageSource<'static> {
    ImageSource::Bytes {
        uri: format!("{URI_PREFIX}{name}.svg").into(),
        bytes: Bytes::Shared(catalog().bytes(name)),
    }
}

/// An image sized to `size` points (the Phosphor box, not the drawn glyph).
/// The caller tints it, usually with [`crate::kit::accent`] when selected.
pub(crate) fn icon(name: Icon, size: f32) -> egui::Image<'static> {
    egui::Image::new(source(name.name())).fit_to_exact_size(Vec2::splat(size))
}

/// Convenience for callers that always tint: accent when selected, otherwise
/// the caller's foreground token.
pub(crate) fn tinted(name: Icon, size: f32, tint: Color32) -> egui::Image<'static> {
    icon(name, size).tint(tint)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing
    )]

    use super::*;
    use sha2::{Digest, Sha256};

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
    }

    #[test]
    fn catalog_exactly_matches_the_icon_enum() {
        assert_eq!(Icon::ALL.len(), ICONS.len());
        for icon in Icon::ALL {
            assert!(
                ICONS.iter().any(|(name, _)| *name == icon.name()),
                "missing embedded SVG for {}",
                icon.name()
            );
        }
        for (name, _) in ICONS {
            assert!(
                Icon::from_name(name).is_some(),
                "embedded SVG {name} has no Icon variant"
            );
        }
    }

    #[test]
    fn unknown_names_are_not_silent_blank_squares() {
        assert_eq!(Icon::from_name("not-an-icon"), None);
        let ImageSource::Bytes { uri, bytes } = source("not-an-icon") else {
            unreachable!("icon source is always embedded bytes")
        };
        assert_eq!(uri, "bytes://phosphor/2.1.1/regular/not-an-icon.svg");
        let Bytes::Shared(bytes) = bytes else {
            unreachable!("catalog shares one Arc per icon")
        };
        assert!(!bytes.is_empty());
        assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
    }

    #[test]
    fn every_icon_is_pinned_by_sha256_in_the_assets_readme() {
        let readme = include_str!("../../../../assets/icons/phosphor/README.md");
        for (name, bytes) in ICONS {
            let digest = hex(&Sha256::digest(bytes));
            let row = format!("| `{name}.svg` | `{digest}` |");
            assert!(
                readme.lines().any(|line| line == row),
                "README hash row missing or stale for {name}: {row}"
            );
        }
        let rows = readme
            .lines()
            .filter(|line| line.starts_with("| `") && line.contains(".svg` | `"))
            .count();
        assert_eq!(rows, ICONS.len(), "README lists a different icon set");
    }

    #[test]
    fn one_arc_is_shared_per_icon() {
        let ImageSource::Bytes { bytes, .. } = source("play") else {
            unreachable!("icon source is always embedded bytes")
        };
        let ImageSource::Bytes { bytes: again, .. } = source("play") else {
            unreachable!("icon source is always embedded bytes")
        };
        let (Bytes::Shared(first), Bytes::Shared(second)) = (bytes, again) else {
            unreachable!("catalog shares one Arc per icon")
        };
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn svg_loader_rasterizes_every_icon_and_keeps_the_measured_glyph() {
        // The 20 px box is the rendered SVG's nominal size. Phosphor regular's
        // filled paths ink 14-20 px inside it, mostly around the measured 16 px
        // glyph; the visual stroke is 16/256 * 20 = 1.25 px.
        let ctx = egui::Context::default();
        install(&ctx);
        for icon in Icon::ALL {
            let ImageSource::Bytes { uri, bytes } = source(icon.name()) else {
                unreachable!("icon source is always embedded bytes")
            };
            ctx.include_bytes(uri.clone(), bytes);
            for size in [16, 20, 24] {
                let loaded = ctx
                    .try_load_image(
                        &uri,
                        egui::load::SizeHint::Size {
                            width: size,
                            height: size,
                            maintain_aspect_ratio: true,
                        },
                    )
                    .unwrap();
                let egui::load::ImagePoll::Ready { image } = loaded else {
                    panic!("{} did not load synchronously", icon.name())
                };
                assert_eq!(image.size, [size as usize, size as usize]);
                let (mut x0, mut y0, mut x1, mut y1) =
                    (size as usize, size as usize, 0usize, 0usize);
                for (index, pixel) in image.pixels.iter().enumerate() {
                    if pixel.a() != 0 {
                        let x = index % size as usize;
                        let y = index / size as usize;
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                }
                assert!(
                    x1 >= x0 && y1 >= y0,
                    "{} rendered empty at {size}px",
                    icon.name()
                );
                if size == 20 {
                    let width = x1 - x0 + 1;
                    let height = y1 - y0 + 1;
                    let longest = width.max(height);
                    assert!(
                        (14..=20).contains(&longest),
                        "{} inks {width}x{height}, outside the 20 px box or its measured glyph",
                        icon.name()
                    );
                }
            }
        }
    }
}
