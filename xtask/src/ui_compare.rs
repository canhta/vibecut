//! Structural comparison of a UI audit render against a reference capture.
//!
//! Colour is deliberately ignored: both images are reduced to luminance,
//! contrast-normalised per image, and compared through Sobel edge maps. Tiles
//! of `TILE` pixels report mean absolute edge error, so a moved panel fails
//! while the same layout in another palette passes.
use image::{Rgba, RgbaImage};
use std::{
    fs,
    path::{Path, PathBuf},
};

const TILE: u32 = 32;
const BLUR_RADIUS: u32 = 2;
/// Edge strength saturates here (in normalised-luminance units) so a recolour
/// that changes how strong an edge is, but not where, scores near zero.
const EDGE_SATURATION: f64 = 1.0;
const DEFAULT_MAX_ERROR: f64 = 0.02;
const WORST_TILES: usize = 10;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 9_000_000;
const MAX_OUTPUT_PIXELS: u64 = 36_000_000;
const USAGE: &str = "usage: cargo xtask ui-compare OURS.png REFERENCE.png [--region X,Y,W,H] [--out DIR] [--max-error F]";

#[derive(Clone, Copy)]
struct Region {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct Options {
    ours: PathBuf,
    reference: PathBuf,
    region: Option<Region>,
    out: PathBuf,
    max_error: f64,
}

#[derive(Debug)]
struct Tile {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    error: f64,
}

#[derive(Debug)]
struct Outputs {
    side_by_side: PathBuf,
    heatmap: PathBuf,
    tiles: PathBuf,
}

#[derive(Debug)]
struct Report {
    width: u32,
    height: u32,
    whole_error: f64,
    tiles: Vec<Tile>,
    outputs: Outputs,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = Options::parse(root, args)?;
    let report = compare(&options)?;
    println!(
        "ui-compare: {}x{}, region {},{} {}x{}",
        report.width,
        report.height,
        options.region.map_or(0, |region| region.x),
        options.region.map_or(0, |region| region.y),
        report.width,
        report.height
    );
    println!(
        "ui-compare: whole-region edge MAE {:.9}",
        report.whole_error
    );
    println!("ui-compare: worst tiles (edge MAE):");
    for tile in report.worst(WORST_TILES) {
        println!(
            "  x={} y={} {}x{} mae={:.9}",
            tile.x, tile.y, tile.width, tile.height, tile.error
        );
    }
    println!(
        "ui-compare: wrote {}, {}, {}",
        report.outputs.side_by_side.display(),
        report.outputs.heatmap.display(),
        report.outputs.tiles.display()
    );
    if report.whole_error > options.max_error {
        return Err(format!(
            "ui-compare: whole-region edge MAE {:.9} exceeds --max-error {:.9}",
            report.whole_error, options.max_error
        ));
    }
    if let Some(tile) = report
        .tiles
        .iter()
        .find(|tile| tile.error > options.max_error)
    {
        return Err(format!(
            "ui-compare: tile x={} y={} {}x{} edge MAE {:.9} exceeds --max-error {:.9}",
            tile.x, tile.y, tile.width, tile.height, tile.error, options.max_error
        ));
    }
    println!("ui-compare: pass (--max-error {:.9})", options.max_error);
    Ok(())
}

impl Options {
    fn parse(root: &Path, args: &[String]) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut region = None;
        let mut out = None;
        let mut max_error = DEFAULT_MAX_ERROR;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            match arg.as_str() {
                "--region" => {
                    let value = args
                        .get(index + 1)
                        .ok_or_else(|| format!("--region needs X,Y,W,H\n{USAGE}"))?;
                    region = Some(parse_region(value)?);
                    index += 2;
                }
                "--out" => {
                    let value = args
                        .get(index + 1)
                        .ok_or_else(|| format!("--out needs a directory\n{USAGE}"))?;
                    out = Some(resolve(root, value));
                    index += 2;
                }
                "--max-error" => {
                    let value = args
                        .get(index + 1)
                        .ok_or_else(|| format!("--max-error needs a number\n{USAGE}"))?;
                    max_error = value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(|| {
                            format!("--max-error must be a finite non-negative number\n{USAGE}")
                        })?;
                    index += 2;
                }
                option if option.starts_with("--") => {
                    return Err(format!("unknown option `{option}`\n{USAGE}"));
                }
                value => {
                    positional.push(value.to_owned());
                    index += 1;
                }
            }
        }
        let (ours, reference) = match positional.as_slice() {
            [ours, reference] => (resolve(root, ours), resolve(root, reference)),
            _ => return Err(USAGE.to_owned()),
        };
        Ok(Self {
            ours,
            reference,
            region,
            out: out.unwrap_or_else(|| root.join("target/ui-compare")),
            max_error,
        })
    }
}

fn resolve(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn parse_region(value: &str) -> Result<Region, String> {
    let parts: Vec<_> = value.split(',').map(str::trim).collect();
    let (x, y, width, height) = match parts.as_slice() {
        [x, y, width, height] => (
            parse_coordinate(x, "X")?,
            parse_coordinate(y, "Y")?,
            parse_coordinate(width, "width")?,
            parse_coordinate(height, "height")?,
        ),
        _ => return Err(format!("--region must be X,Y,W,H, got `{value}`")),
    };
    if width == 0 || height == 0 {
        return Err(format!(
            "--region width and height must be positive, got `{value}`"
        ));
    }
    Ok(Region {
        x,
        y,
        width,
        height,
    })
}

fn parse_coordinate(value: &str, label: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("--region {label} must be a non-negative integer, got `{value}`"))
}

fn compare(options: &Options) -> Result<Report, String> {
    let ours = load(&options.ours)?;
    let reference = load(&options.reference)?;
    let (width, height) = ours.dimensions();
    let (reference_width, reference_height) = reference.dimensions();
    if (width, height) != (reference_width, reference_height) {
        return Err(format!(
            "size mismatch: ours {width}x{height}, reference {reference_width}x{reference_height}; downscale a 2x capture to 1x before comparing (never resized)"
        ));
    }
    let region = options.region.unwrap_or(Region {
        x: 0,
        y: 0,
        width,
        height,
    });
    let end_x = region
        .x
        .checked_add(region.width)
        .ok_or("region overflows image width")?;
    let end_y = region
        .y
        .checked_add(region.height)
        .ok_or("region overflows image height")?;
    if end_x > width || end_y > height {
        return Err(format!(
            "region {},{} {}x{} exceeds image {width}x{height}",
            region.x, region.y, region.width, region.height
        ));
    }
    let ours = image::imageops::crop_imm(&ours, region.x, region.y, region.width, region.height)
        .to_image();
    let reference =
        image::imageops::crop_imm(&reference, region.x, region.y, region.width, region.height)
            .to_image();

    let mut ours_luma = luminance(&ours);
    let mut reference_luma = luminance(&reference);
    contrast_normalise(&mut ours_luma);
    contrast_normalise(&mut reference_luma);
    let ours_edge = box_blur(
        &sobel(&ours_luma, region.width, region.height)?,
        region.width,
        region.height,
        BLUR_RADIUS,
    )?;
    let reference_edge = box_blur(
        &sobel(&reference_luma, region.width, region.height)?,
        region.width,
        region.height,
        BLUR_RADIUS,
    )?;
    let difference: Vec<f64> = ours_edge
        .iter()
        .zip(&reference_edge)
        .map(|(ours, reference)| (ours - reference).abs())
        .collect();
    let whole_error = mean(&difference)?;
    let tiles = tile_errors(&ours_edge, &reference_edge, region.width, region.height)?;

    fs::create_dir_all(&options.out)
        .map_err(|error| format!("{}: {error}", options.out.display()))?;
    let side_by_side = options.out.join("side-by-side.png");
    let heatmap = options.out.join("heatmap.png");
    let tiles_path = options.out.join("tiles.csv");
    let ours_edge_image = edge_image(&ours_edge, region.width, region.height);
    let reference_edge_image = edge_image(&reference_edge, region.width, region.height);
    save_png(
        &compose(&[&ours, &ours_edge_image, &reference, &reference_edge_image])?,
        &side_by_side,
    )?;
    save_png(
        &compose(&[
            &ours_edge_image,
            &reference_edge_image,
            &heat_image(&difference, region.width, region.height)?,
        ])?,
        &heatmap,
    )?;
    write_tiles(&tiles, &tiles_path)?;

    Ok(Report {
        width: region.width,
        height: region.height,
        whole_error,
        tiles,
        outputs: Outputs {
            side_by_side,
            heatmap,
            tiles: tiles_path,
        },
    })
}

fn load(path: &Path) -> Result<RgbaImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("{}: {error}", path.display()))?
        .to_rgba8();
    let pixels = u64::from(image.width()) * u64::from(image.height());
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err(format!(
            "{}: {}x{} image must contain between 1 and {MAX_PIXELS} pixels",
            path.display(),
            image.width(),
            image.height()
        ));
    }
    Ok(image)
}

fn luminance(image: &RgbaImage) -> Vec<f64> {
    image
        .pixels()
        .map(|pixel| {
            let [red, green, blue, _] = pixel.0;
            (0.2126 * f64::from(red) + 0.7152 * f64::from(green) + 0.0722 * f64::from(blue)) / 255.0
        })
        .collect()
}

/// Remove global brightness and contrast so a light/dark or tinted capture
/// compares by structure. A flat image becomes all zero.
fn contrast_normalise(values: &mut [f64]) {
    let len = values.len() as f64;
    if values.is_empty() {
        return;
    }
    let mean = values.iter().sum::<f64>() / len;
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / len;
    let deviation = variance.sqrt();
    if deviation <= f64::EPSILON {
        values.fill(0.0);
    } else {
        for value in values.iter_mut() {
            *value = (*value - mean) / deviation;
        }
    }
}

fn sobel(values: &[f64], width: u32, height: u32) -> Result<Vec<f64>, String> {
    let image_width = i64::from(width);
    let image_height = i64::from(height);
    let mut output = Vec::with_capacity(values.len());
    for y in 0..image_height {
        for x in 0..image_width {
            let top_left = sample(values, image_width, image_height, x - 1, y - 1)?;
            let top = sample(values, image_width, image_height, x, y - 1)?;
            let top_right = sample(values, image_width, image_height, x + 1, y - 1)?;
            let left = sample(values, image_width, image_height, x - 1, y)?;
            let right = sample(values, image_width, image_height, x + 1, y)?;
            let bottom_left = sample(values, image_width, image_height, x - 1, y + 1)?;
            let bottom = sample(values, image_width, image_height, x, y + 1)?;
            let bottom_right = sample(values, image_width, image_height, x + 1, y + 1)?;
            let gradient_x =
                -top_left + top_right - 2.0 * left + 2.0 * right - bottom_left + bottom_right;
            let gradient_y =
                -top_left - 2.0 * top - top_right + bottom_left + 2.0 * bottom + bottom_right;
            let magnitude = (gradient_x * gradient_x + gradient_y * gradient_y).sqrt();
            output.push((magnitude / EDGE_SATURATION).tanh());
        }
    }
    Ok(output)
}

fn sample(values: &[f64], width: i64, height: i64, x: i64, y: i64) -> Result<f64, String> {
    let x = x.clamp(0, width - 1);
    let y = y.clamp(0, height - 1);
    let index = usize::try_from(y * width + x).map_err(|_| "image dimensions too large")?;
    values
        .get(index)
        .copied()
        .ok_or_else(|| "edge map index out of range".to_owned())
}

fn box_blur(values: &[f64], width: u32, height: u32, radius: u32) -> Result<Vec<f64>, String> {
    if radius == 0 {
        return Ok(values.to_vec());
    }
    let image_width = i64::from(width);
    let image_height = i64::from(height);
    let radius = i64::from(radius);
    let window = (radius * 2 + 1) as f64;
    let mut horizontal = Vec::with_capacity(values.len());
    for y in 0..image_height {
        for x in 0..image_width {
            let mut sum = 0.0;
            for offset in -radius..=radius {
                sum += sample(values, image_width, image_height, x + offset, y)?;
            }
            horizontal.push(sum / window);
        }
    }
    let mut output = Vec::with_capacity(values.len());
    for y in 0..image_height {
        for x in 0..image_width {
            let mut sum = 0.0;
            for offset in -radius..=radius {
                sum += sample(&horizontal, image_width, image_height, x, y + offset)?;
            }
            output.push(sum / window);
        }
    }
    Ok(output)
}

fn mean(values: &[f64]) -> Result<f64, String> {
    if values.is_empty() {
        return Err("cannot average an empty image".into());
    }
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}

fn tile_errors(
    ours: &[f64],
    reference: &[f64],
    width: u32,
    height: u32,
) -> Result<Vec<Tile>, String> {
    if ours.len() != reference.len() {
        return Err("edge maps have different lengths".into());
    }
    let mut tiles = Vec::new();
    let mut y = 0;
    while y < height {
        let tile_height = TILE.min(height - y);
        let mut x = 0;
        while x < width {
            let tile_width = TILE.min(width - x);
            let mut sum = 0.0;
            for row in y..y + tile_height {
                for column in x..x + tile_width {
                    let index =
                        usize::try_from(u64::from(row) * u64::from(width) + u64::from(column))
                            .map_err(|_| "image dimensions too large")?;
                    let ours = ours
                        .get(index)
                        .ok_or_else(|| "edge map index out of range".to_owned())?;
                    let reference = reference
                        .get(index)
                        .ok_or_else(|| "edge map index out of range".to_owned())?;
                    sum += (ours - reference).abs();
                }
            }
            tiles.push(Tile {
                x,
                y,
                width: tile_width,
                height: tile_height,
                error: sum / f64::from(tile_width * tile_height),
            });
            x += TILE;
        }
        y += TILE;
    }
    Ok(tiles)
}

impl Report {
    fn worst(&self, count: usize) -> Vec<&Tile> {
        let mut tiles: Vec<_> = self.tiles.iter().collect();
        tiles.sort_by(|left, right| {
            right
                .error
                .total_cmp(&left.error)
                .then(left.y.cmp(&right.y))
                .then(left.x.cmp(&right.x))
        });
        tiles.truncate(count);
        tiles
    }
}

fn edge_image(values: &[f64], width: u32, height: u32) -> RgbaImage {
    let maximum = values.iter().copied().fold(0.0, f64::max);
    let scale = if maximum > 0.0 { 255.0 / maximum } else { 0.0 };
    RgbaImage::from_fn(width, height, |x, y| {
        let index = usize::try_from(u64::from(y) * u64::from(width) + u64::from(x)).ok();
        let value = index
            .and_then(|index| values.get(index))
            .map_or(0.0, |value| (value * scale).clamp(0.0, 255.0));
        let value = value.round() as u8;
        Rgba([value, value, value, 255])
    })
}

fn heat_image(difference: &[f64], width: u32, height: u32) -> Result<RgbaImage, String> {
    let maximum = difference.iter().copied().fold(0.0, f64::max);
    let mut image = RgbaImage::new(width, height);
    for (index, value) in difference.iter().enumerate() {
        let index = u64::try_from(index).map_err(|_| "image dimensions too large")?;
        let x = u32::try_from(index % u64::from(width)).map_err(|_| "image too wide")?;
        let y = u32::try_from(index / u64::from(width)).map_err(|_| "image too tall")?;
        let fraction = if maximum > 0.0 { value / maximum } else { 0.0 };
        image.put_pixel(x, y, heat_colour(fraction));
    }
    Ok(image)
}

fn heat_colour(fraction: f64) -> Rgba<u8> {
    let fraction = fraction.clamp(0.0, 1.0);
    let channel = |offset: f64| ((fraction * 3.0 - offset).clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba([channel(0.0), channel(1.0), channel(2.0), 255])
}

fn compose(panels: &[&RgbaImage]) -> Result<RgbaImage, String> {
    let height = panels
        .iter()
        .map(|panel| panel.height())
        .max()
        .ok_or("no image panels")?;
    if panels.iter().any(|panel| panel.height() != height) {
        return Err("image panels have different heights".into());
    }
    let width = panels.iter().try_fold(0u32, |width, panel| {
        width
            .checked_add(panel.width())
            .ok_or("combined image is too wide")
    })?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_OUTPUT_PIXELS {
        return Err(format!(
            "combined image is {width}x{height} ({pixels} pixels), above the {MAX_OUTPUT_PIXELS} pixel cap"
        ));
    }
    let mut output = RgbaImage::new(width, height);
    let mut x = 0i64;
    for panel in panels {
        image::imageops::overlay(&mut output, *panel, x, 0);
        x += i64::from(panel.width());
    }
    Ok(output)
}

fn save_png(image: &RgbaImage, path: &Path) -> Result<(), String> {
    image
        .save(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn write_tiles(tiles: &[Tile], path: &Path) -> Result<(), String> {
    let mut text = String::from("x,y,width,height,mae\n");
    for tile in tiles {
        text.push_str(&format!(
            "{},{},{},{},{:.9}\n",
            tile.x, tile.y, tile.width, tile.height, tile.error
        ));
    }
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("target/harness-tests")
                .join(format!(
                    "ui-compare-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn blank(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([232, 232, 232, 255]))
    }

    fn paint(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32) {
        for row in y..y + height {
            for column in x..x + width {
                image.put_pixel(column, row, Rgba([36, 44, 52, 255]));
            }
        }
    }

    fn scene() -> RgbaImage {
        let mut image = blank(192, 128);
        paint(&mut image, 16, 16, 80, 96);
        paint(&mut image, 128, 16, 48, 24);
        paint(&mut image, 128, 56, 20, 20);
        image
    }

    /// Same layout, different fill colours whose luminance ratios differ from
    /// the scene, so edge strengths change but edge positions do not.
    fn tinted(image: &RgbaImage) -> RgbaImage {
        RgbaImage::from_fn(image.width(), image.height(), |x, y| {
            let [red, ..] = image.get_pixel(x, y).0;
            match (red > 128, x < 120) {
                (true, _) => Rgba([40, 90, 70, 255]),
                (false, true) => Rgba([150, 40, 120, 255]),
                (false, false) => Rgba([30, 120, 90, 255]),
            }
        })
    }

    fn inverted(image: &RgbaImage) -> RgbaImage {
        RgbaImage::from_fn(image.width(), image.height(), |x, y| {
            let [red, green, blue, alpha] = image.get_pixel(x, y).0;
            Rgba([255 - red, 255 - green, 255 - blue, alpha])
        })
    }

    fn save(scratch: &Scratch, name: &str, image: &RgbaImage) -> PathBuf {
        let path = scratch.path(name);
        image.save(&path).unwrap();
        path
    }

    fn options(
        scratch: &Scratch,
        ours: &Path,
        reference: &Path,
        region: Option<Region>,
        max_error: f64,
    ) -> Options {
        Options {
            ours: ours.to_path_buf(),
            reference: reference.to_path_buf(),
            region,
            out: scratch.path("out"),
            max_error,
        }
    }

    #[test]
    fn identical_images_score_zero_and_pass() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &scene());
        let reference = save(&scratch, "reference.png", &scene());
        let report = compare(&options(&scratch, &ours, &reference, None, 0.0)).unwrap();
        assert_eq!(report.whole_error, 0.0);
        assert!(report.tiles.iter().all(|tile| tile.error == 0.0));
        assert!(report.outputs.side_by_side.exists());
        assert!(report.outputs.heatmap.exists());
        assert!(report.outputs.tiles.exists());
        run(
            &scratch.0,
            &[
                ours.display().to_string(),
                reference.display().to_string(),
                "--out".to_owned(),
                scratch.path("cli").display().to_string(),
                "--max-error".to_owned(),
                "0".to_owned(),
            ],
        )
        .unwrap();
    }

    #[test]
    fn same_layout_recoloured_scores_near_zero() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &scene());
        let reference = save(&scratch, "recoloured.png", &inverted(&scene()));
        let report = compare(&options(
            &scratch,
            &ours,
            &reference,
            None,
            DEFAULT_MAX_ERROR,
        ))
        .unwrap();
        assert!(
            report.whole_error < 1e-9,
            "recoloured layout should not carry structure error: {}",
            report.whole_error
        );
    }

    #[test]
    fn recolour_with_other_contrast_stays_far_below_a_moved_panel() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &scene());
        let tinted = save(&scratch, "tinted.png", &tinted(&scene()));
        let moved = save(&scratch, "moved.png", &move_panel(scene()));
        let recoloured = compare(&options(&scratch, &ours, &tinted, None, 1.0)).unwrap();
        let shifted = compare(&options(&scratch, &ours, &moved, None, 1.0)).unwrap();
        let worst = |report: &Report| report.worst(1).first().map_or(0.0, |tile| tile.error);
        assert!(
            worst(&recoloured) * 4.0 < worst(&shifted),
            "recolour tile {} vs moved tile {}",
            worst(&recoloured),
            worst(&shifted)
        );
    }

    #[test]
    fn shifted_block_names_its_tile() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "blank.png", &blank(192, 128));
        let mut shifted = blank(192, 128);
        paint(&mut shifted, 72, 40, 16, 16);
        let reference = save(&scratch, "shifted.png", &shifted);
        let report = compare(&options(
            &scratch,
            &ours,
            &reference,
            None,
            DEFAULT_MAX_ERROR,
        ))
        .unwrap();
        let worst = report.worst(1);
        let tile = worst.first().unwrap();
        assert_eq!((tile.x, tile.y), (64, 32), "worst tile: {tile:?}");
        assert!(tile.error > DEFAULT_MAX_ERROR, "worst tile: {tile:?}");
        let limit = (report.whole_error + tile.error) / 2.0;
        assert!(report.whole_error < limit && tile.error > limit);
        let error = run(
            &scratch.0,
            &[
                ours.display().to_string(),
                reference.display().to_string(),
                "--out".to_owned(),
                scratch.path("cli").display().to_string(),
                "--max-error".to_owned(),
                limit.to_string(),
            ],
        )
        .unwrap_err();
        assert!(error.contains("tile x=64 y=32"), "{error}");
    }

    #[test]
    fn panel_moved_six_pixels_scores_high() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &scene());
        let reference = save(&scratch, "moved.png", &move_panel(scene()));
        let report = compare(&options(
            &scratch,
            &ours,
            &reference,
            None,
            DEFAULT_MAX_ERROR,
        ))
        .unwrap();
        assert!(
            report.whole_error > DEFAULT_MAX_ERROR,
            "a 6 px panel move must fail: {}",
            report.whole_error
        );
        let error = run(
            &scratch.0,
            &[
                ours.display().to_string(),
                reference.display().to_string(),
                "--out".to_owned(),
                scratch.path("cli").display().to_string(),
            ],
        )
        .unwrap_err();
        assert!(error.contains("whole-region edge MAE"), "{error}");
        assert!(error.contains("exceeds --max-error"), "{error}");
    }

    fn move_panel(mut image: RgbaImage) -> RgbaImage {
        paint(&mut image, 22, 16, 80, 96);
        for row in 16..112 {
            for column in 16..22 {
                image.put_pixel(column, row, Rgba([232, 232, 232, 255]));
            }
        }
        image
    }

    #[test]
    fn size_mismatch_names_both_sizes_and_never_resizes() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &blank(64, 48));
        let reference = save(&scratch, "reference.png", &blank(64, 64));
        let error = compare(&options(
            &scratch,
            &ours,
            &reference,
            None,
            DEFAULT_MAX_ERROR,
        ))
        .unwrap_err();
        assert!(error.contains("64x48"), "{error}");
        assert!(error.contains("64x64"), "{error}");
        assert!(error.contains("never resized"), "{error}");
    }

    #[test]
    fn region_crop_ignores_changes_outside_the_region() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &scene());
        let mut changed = scene();
        paint(&mut changed, 160, 0, 32, 32);
        let reference = save(&scratch, "changed.png", &changed);
        let region = Region {
            x: 0,
            y: 0,
            width: 128,
            height: 128,
        };
        let report = compare(&options(
            &scratch,
            &ours,
            &reference,
            Some(region),
            DEFAULT_MAX_ERROR,
        ))
        .unwrap();
        assert_eq!(report.whole_error, 0.0);
        assert_eq!((report.width, report.height), (128, 128));
        let side = image::open(&report.outputs.side_by_side).unwrap();
        assert_eq!(side.dimensions(), (128 * 4, 128));
        let heat = image::open(&report.outputs.heatmap).unwrap();
        assert_eq!(heat.dimensions(), (128 * 3, 128));
    }

    #[test]
    fn region_outside_the_image_is_an_error() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &blank(128, 96));
        let reference = save(&scratch, "reference.png", &blank(128, 96));
        let region = Region {
            x: 100,
            y: 80,
            width: 40,
            height: 20,
        };
        let error = compare(&options(
            &scratch,
            &ours,
            &reference,
            Some(region),
            DEFAULT_MAX_ERROR,
        ))
        .unwrap_err();
        assert!(error.contains("region 100,80 40x20"), "{error}");
        assert!(error.contains("128x96"), "{error}");
    }

    #[test]
    fn output_bytes_are_deterministic_across_runs() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &scene());
        let reference = save(&scratch, "moved.png", &move_panel(scene()));
        let first = compare(&options(
            &scratch,
            &ours,
            &reference,
            None,
            DEFAULT_MAX_ERROR,
        ))
        .unwrap();
        let second_options = Options {
            ours: ours.clone(),
            reference: reference.clone(),
            region: None,
            out: scratch.path("out-again"),
            max_error: DEFAULT_MAX_ERROR,
        };
        let second = compare(&second_options).unwrap();
        for (left, right) in [
            (&first.outputs.side_by_side, &second.outputs.side_by_side),
            (&first.outputs.heatmap, &second.outputs.heatmap),
            (&first.outputs.tiles, &second.outputs.tiles),
        ] {
            assert_eq!(fs::read(left).unwrap(), fs::read(right).unwrap());
        }
    }

    #[test]
    fn invalid_arguments_are_rejected_with_usage() {
        let scratch = Scratch::new();
        let ours = save(&scratch, "ours.png", &blank(32, 32));
        let ours = ours.display().to_string();
        for (args, expected) in [
            (vec![ours.clone()], "usage:"),
            (
                vec![ours.clone(), ours.clone(), "--bogus".to_owned()],
                "unknown option",
            ),
            (
                vec![
                    ours.clone(),
                    ours.clone(),
                    "--region".to_owned(),
                    "1,2,3".to_owned(),
                ],
                "X,Y,W,H",
            ),
            (
                vec![
                    ours.clone(),
                    ours.clone(),
                    "--region".to_owned(),
                    "0,0,0,4".to_owned(),
                ],
                "must be positive",
            ),
            (
                vec![
                    ours.clone(),
                    ours.clone(),
                    "--max-error".to_owned(),
                    "NaN".to_owned(),
                ],
                "finite non-negative",
            ),
            (
                vec![
                    ours.clone(),
                    ours.clone(),
                    "--max-error".to_owned(),
                    "-1".to_owned(),
                ],
                "finite non-negative",
            ),
            (
                vec![ours.clone(), ours.clone(), "--region".to_owned()],
                "--region needs",
            ),
        ] {
            let error = run(&scratch.0, &args).unwrap_err();
            assert!(error.contains(expected), "{args:?}: {error}");
        }
    }
}
