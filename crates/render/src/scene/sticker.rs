//! Sticker pixels: bundled flag SVGs rasterised at the destination pixel size, and a placeholder
//! for an asset that cannot be drawn. Shapes are graphic items, not stickers.

use super::Placement;
use super::sticker_flags::FLAGS;
use crate::image::{color, over};
use crate::{ExecutionHooks, Image, MemoryKind, Result, check};
use project::StickerParams;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Longest side, in pixels, a single sticker raster may have.
pub const MAX_STICKER_RASTER_SIDE: u32 = 8192;
/// Provider id of the bundled flags; the catalog in `libraries` uses the same value.
pub const FLAGS_PROVIDER: &str = "flags";

/// Why a sticker could not be drawn from its asset.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StickerError {
    /// No bundled asset answers this id.
    #[error("sticker asset is missing")]
    Missing { sticker: String },
    /// The asset exists but does not parse as SVG, or the requested raster size is out of range.
    #[error("sticker asset cannot be rasterised")]
    Invalid { sticker: String },
}
impl StickerError {
    /// The sticker id the failure belongs to.
    pub fn sticker(&self) -> &str {
        match self {
            Self::Missing { sticker } | Self::Invalid { sticker } => sticker,
        }
    }
}

/// A premultiplied RGBA8 raster, row-major with no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerRaster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The bundled flag named by a `flags:<code>` id: its table code and SVG bytes.
fn flag_svg(sticker: &str) -> Option<(&'static str, &'static [u8])> {
    let (provider, value) = sticker.split_once(':')?;
    if provider != FLAGS_PROVIDER {
        return None;
    }
    let value = value.to_ascii_lowercase();
    let index = FLAGS
        .binary_search_by(|(code, _)| (*code).cmp(value.as_str()))
        .ok()?;
    FLAGS.get(index).copied()
}

/// The parsed tree of one bundled flag. Each flag parses once per process; a frame only scales
/// and paints the cached tree.
fn flag_tree(code: &'static str, bytes: &'static [u8]) -> Option<Arc<resvg::usvg::Tree>> {
    static TREES: OnceLock<Mutex<BTreeMap<&'static str, Arc<resvg::usvg::Tree>>>> = OnceLock::new();
    let trees = TREES.get_or_init(Mutex::default);
    let mut trees = trees
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(tree) = trees.get(code) {
        return Some(Arc::clone(tree));
    }
    let tree =
        Arc::new(resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?);
    trees.insert(code, Arc::clone(&tree));
    Some(tree)
}

/// Rasterises a bundled SVG sticker to exactly `width` x `height` pixels. The vector tree is scaled
/// to that size, so the result is crisp at every size up to [`MAX_STICKER_RASTER_SIDE`].
///
/// # Errors
///
/// [`StickerError::Missing`] when no bundled asset has this id and
/// [`StickerError::Invalid`] for an unparsable asset or a size outside
/// `1..=MAX_STICKER_RASTER_SIDE`.
pub fn rasterize_sticker(
    sticker: &str,
    width: u32,
    height: u32,
) -> std::result::Result<StickerRaster, StickerError> {
    let missing = || StickerError::Missing {
        sticker: sticker.to_owned(),
    };
    let invalid = || StickerError::Invalid {
        sticker: sticker.to_owned(),
    };
    let (code, bytes) = flag_svg(sticker).ok_or_else(missing)?;
    if !(1..=MAX_STICKER_RASTER_SIDE).contains(&width)
        || !(1..=MAX_STICKER_RASTER_SIDE).contains(&height)
    {
        return Err(invalid());
    }
    let tree = flag_tree(code, bytes).ok_or_else(invalid)?;
    let size = tree.size();
    if !(size.width() > 0. && size.height() > 0.) {
        return Err(invalid());
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(invalid)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(StickerRaster {
        width,
        height,
        pixels: pixmap.take(),
    })
}

/// Draws one sticker into the item's layer. A sticker whose asset is missing or invalid paints a
/// placeholder in its place and returns the typed failure for the caller to report.
pub(super) fn draw_sticker(
    target: &mut Image,
    params: &StickerParams,
    placement: Placement,
    hooks: &dyn ExecutionHooks,
) -> Result<Option<StickerError>> {
    let r = placement.destination;
    let (width, height) = (f64::from(target.width), f64::from(target.height));
    let (dw, dh) = (r.width * width, r.height * height);
    let intrinsic = (f64::from(params.width), f64::from(params.height));
    let fit = (dw / intrinsic.0).min(dh / intrinsic.1);
    let side = f64::from(MAX_STICKER_RASTER_SIDE);
    let pw = (intrinsic.0 * fit).round().clamp(1., side) as u32;
    let ph = (intrinsic.1 * fit).round().clamp(1., side) as u32;
    let x0 = (r.x * width + (dw - f64::from(pw)) / 2.).round() as i64;
    let y0 = (r.y * height + (dh - f64::from(ph)) / 2.).round() as i64;
    let _lease = hooks.reserve(MemoryKind::Compositor, u64::from(pw) * u64::from(ph) * 8)?;
    let raster = match rasterize_sticker(&params.sticker, pw, ph) {
        Ok(raster) => raster,
        Err(error) => {
            paint_placeholder(target, [x0, y0], [pw, ph], placement.opacity, hooks)?;
            return Ok(Some(error));
        }
    };
    let columns = target.width as usize;
    for (row, line) in raster.pixels.chunks_exact(pw as usize * 4).enumerate() {
        check(hooks)?;
        let Some(y) = usize::try_from(y0 + row as i64)
            .ok()
            .filter(|y| *y < target.height as usize)
        else {
            continue;
        };
        let Some(dst_row) = target.pixels.chunks_mut(columns).nth(y) else {
            continue;
        };
        for (column, pixel) in line.chunks_exact(4).enumerate() {
            let Some(dst) = usize::try_from(x0 + column as i64)
                .ok()
                .and_then(|x| dst_row.get_mut(x))
            else {
                continue;
            };
            let &[red, green, blue, alpha] = pixel else {
                continue;
            };
            if alpha == 0 {
                continue;
            }
            let straight =
                resvg::tiny_skia::PremultipliedColorU8::from_rgba(red, green, blue, alpha)
                    .map(|premultiplied| premultiplied.demultiply())
                    .map_or([0; 4], |c| [c.red(), c.green(), c.blue(), c.alpha()]);
            over(dst, color(straight), placement.opacity);
        }
    }
    Ok(None)
}

/// A dim box with a light border and both diagonals, drawn where a sticker's asset is unavailable.
fn paint_placeholder(
    target: &mut Image,
    origin: [i64; 2],
    size: [u32; 2],
    opacity: f32,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let fill = color([40, 40, 40, 160]);
    let line = color([230, 230, 230, 255]);
    let border = 2_i64;
    let (w, h) = (i64::from(size[0]), i64::from(size[1]));
    let longest = w.max(h).max(1);
    let columns = target.width as usize;
    for (y, row) in target.pixels.chunks_mut(columns).enumerate() {
        check(hooks)?;
        let ry = y as i64 - origin[1];
        if !(0..h).contains(&ry) {
            continue;
        }
        for (x, dst) in row.iter_mut().enumerate() {
            let rx = x as i64 - origin[0];
            if !(0..w).contains(&rx) {
                continue;
            }
            let edge = rx < border || ry < border || rx >= w - border || ry >= h - border;
            let d1 = (rx * h - ry * w).abs() / longest;
            let d2 = ((w - 1 - rx) * h - ry * w).abs() / longest;
            let cross = d1 < border || d2 < border;
            over(dst, if edge || cross { line } else { fill }, opacity);
        }
    }
    Ok(())
}
