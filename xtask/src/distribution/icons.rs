//! Reproduce the established icon exports; normal builds consume committed assets.
use super::{Host, Result, Temp, Tools, arg};
use image::{
    ExtendedColorType, RgbaImage,
    codecs::ico::{IcoEncoder, IcoFrame},
    imageops::{FilterType, overlay},
};
use std::{fs, path::Path};

fn ico(source: &Path, target: &Path, sizes: &[u32]) -> Result<()> {
    let image = image::open(source).map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    for &size in sizes {
        let rgba = image
            .resize_exact(size, size, FilterType::Lanczos3)
            .into_rgba8();
        frames.push(
            IcoFrame::as_png(rgba.as_raw(), size, size, ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?,
        );
    }
    IcoEncoder::new(fs::File::create(target).map_err(|e| e.to_string())?)
        .encode_images(&frames)
        .map_err(|e| e.to_string())
}
pub(super) fn run(tools: &mut Tools) -> Result<()> {
    if tools.host != Host::Mac {
        return Err("icon regeneration requires macOS/Xcode Icon Composer".into());
    }
    let developer = run!(tools, "xcode-select", "-p")?;
    let composer = Path::new(developer.trim())
        .join("../Applications/Icon Composer.app/Contents/Executables/ictool");
    if !composer.is_file() {
        return Err("install Xcode with Icon Composer first".into());
    }
    let directory = tools.root.join("assets/app-icon");
    let brand = tools.root.join("docs/brand");
    let temporary = Temp::new(&tools.runner_temp)?;
    run!(
        tools,
        "sips",
        "-s",
        "format",
        "png",
        directory.join("vibecut.svg"),
        "--out",
        directory.join("vibecut-1024.png")
    )?;
    tools.run(
        composer.to_str().ok_or("Icon Composer path UTF-8")?,
        vec![
            arg(directory.join("AppIcon.icon")),
            arg("--export-image"),
            arg("--output-file"),
            arg(temporary.0.join("macos.png")),
            arg("--platform"),
            arg("macOS"),
            arg("--rendition"),
            arg("Default"),
            arg("--width"),
            arg("1024"),
            arg("--height"),
            arg("1024"),
            arg("--scale"),
            arg("1"),
        ],
    )?;
    let image = image::open(temporary.0.join("macos.png"))
        .map_err(|e| e.to_string())?
        .resize_exact(824, 824, FilterType::Lanczos3)
        .into_rgba8();
    let mut canvas = RgbaImage::new(1024, 1024);
    overlay(&mut canvas, &image, 100, 100);
    canvas
        .save(directory.join("vibecut-macos-1024.png"))
        .map_err(|e| e.to_string())?;
    run!(
        tools,
        "sips",
        "-z",
        "512",
        "512",
        directory.join("vibecut-macos-1024.png"),
        "--out",
        directory.join("vibecut-macos-512.png")
    )?;
    let set = temporary.0.join("vibecut.iconset");
    fs::create_dir(&set).map_err(|e| e.to_string())?;
    for size in [16, 32, 128, 256, 512] {
        for (size, suffix) in [(size, ""), (size * 2, "@2x")] {
            let name = if suffix.is_empty() {
                format!("icon_{size}x{size}.png")
            } else {
                let nominal = size / 2;
                format!("icon_{nominal}x{nominal}@2x.png")
            };
            run!(
                tools,
                "sips",
                "-z",
                size.to_string(),
                size.to_string(),
                directory.join("vibecut-macos-1024.png"),
                "--out",
                set.join(name)
            )?;
        }
    }
    run!(
        tools,
        "iconutil",
        "-c",
        "icns",
        "-o",
        directory.join("vibecut.icns"),
        &set
    )?;
    ico(
        &directory.join("vibecut-1024.png"),
        &directory.join("vibecut.ico"),
        &[16, 20, 24, 32, 40, 48, 64, 128, 256],
    )?;
    run!(
        tools,
        "sips",
        "-s",
        "format",
        "png",
        brand.join("logo.svg"),
        "--out",
        brand.join("logo.png")
    )?;
    let favicon = temporary.0.join("favicon.png");
    run!(
        tools,
        "sips",
        "-s",
        "format",
        "png",
        brand.join("favicon.svg"),
        "--out",
        &favicon
    )?;
    run!(
        tools,
        "sips",
        "-z",
        "180",
        "180",
        &favicon,
        "--out",
        brand.join("apple-touch-icon.png")
    )?;
    ico(&favicon, &brand.join("favicon.ico"), &[16, 32, 48])?;
    let out = tools.root.join("target/icons/native");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    run!(
        tools,
        "xcrun",
        "actool",
        directory.join("AppIcon.icon"),
        "--compile",
        &out,
        "--platform",
        "macosx",
        "--minimum-deployment-target",
        "26.0",
        "--app-icon",
        "AppIcon",
        "--standalone-icon-behavior",
        "all",
        "--output-partial-info-plist",
        out.join("icon-info.plist")
    )?;
    println!(
        "Icons regenerated; inspect exports before committing. Native catalog: {}",
        out.display()
    );
    Ok(())
}
