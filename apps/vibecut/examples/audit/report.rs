//! Copies reference images, writes the audit gallery HTML.
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use vibecut::egui;

pub(super) fn reference_root(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VIBECUT_UI_REFERENCE") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    let local = root.join(".spikes/ui-reference");
    if local.is_dir() {
        return Some(local);
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .map(|worktree| worktree.join(".spikes/ui-reference"))
        .filter(|path| path.is_dir())
}

pub(super) fn copy_reference(
    root: Option<&Path>,
    output: &Path,
    variant: &str,
    relative: &str,
) -> Option<String> {
    let source = root?.join(relative);
    let extension = source.extension()?.to_str()?;
    let name = format!("{variant}.{extension}");
    let directory = output.join("reference");
    std::fs::create_dir_all(&directory).ok()?;
    std::fs::copy(&source, directory.join(&name)).ok()?;
    Some(format!("reference/{name}"))
}

/// egui renders premultiplied alpha over a transparent clear; the audit PNG is
/// opaque so it can be compared and reviewed outside a compositor.
pub(super) fn opaque(image: image::RgbaImage, background: egui::Color32) -> image::RgbaImage {
    let [red, green, blue, _] = background.to_array();
    let mut output = image::RgbaImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let [source_red, source_green, source_blue, source_alpha] = pixel.0;
        let inverse = 255 - u32::from(source_alpha);
        let blend = |source: u8, destination: u8| {
            (u32::from(source) + u32::from(destination) * inverse / 255).min(255) as u8
        };
        output.put_pixel(
            x,
            y,
            image::Rgba([
                blend(source_red, red),
                blend(source_green, green),
                blend(source_blue, blue),
                255,
            ]),
        );
    }
    output
}

pub(super) struct Row {
    pub(super) variant: String,
    pub(super) theme: &'static str,
    pub(super) images: Vec<(String, f32)>,
    pub(super) reference: Option<String>,
}

pub(super) fn index_html(rows: &[Row]) -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>VibeCut UI audit</title>\n<style>\n\
         body{font:14px/1.4 system-ui,sans-serif;margin:24px;background:#0f1113;color:#fff}\n\
         h1{font-size:24px} h2{font-size:18px;margin-top:32px}\n\
         table{border-collapse:collapse;width:100%}\n\
         th,td{text-align:left;vertical-align:top;padding:8px;border-top:1px solid #26272a}\n\
         img{max-width:100%;display:block;border:1px solid #26272a}\n\
         .missing{color:#898a8b;font-style:italic}\n\
         </style>\n</head>\n<body>\n<h1>VibeCut UI audit</h1>\n\
         <p>Offscreen renders at 1x and 2x. Reference images are copied from \
         <code>.spikes/ui-reference/</code> when present.</p>\n",
    );
    let mut current = "";
    for Row {
        variant,
        theme,
        images,
        reference,
    } in rows
    {
        let scene = variant
            .strip_suffix("-dark")
            .or_else(|| variant.strip_suffix("-light"))
            .unwrap_or(variant);
        if scene != current {
            if !current.is_empty() {
                let _ = writeln!(html, "</table>");
            }
            let _ = writeln!(html, "<h2>{scene}</h2>\n<table>");
            let _ = writeln!(
                html,
                "<tr><th>theme</th><th>1x</th><th>2x</th><th>reference</th></tr>"
            );
            current = scene;
        }
        let _ = writeln!(html, "<tr><th>{theme}</th>");
        for (file, _) in images {
            let _ = writeln!(
                html,
                "<td><a href=\"{file}\"><img src=\"{file}\" alt=\"{variant}\"></a></td>"
            );
        }
        match reference {
            Some(reference) => {
                let _ = writeln!(
                    html,
                    "<td><img src=\"{reference}\" alt=\"reference for {variant}\"></td></tr>"
                );
            }
            None => {
                let _ = writeln!(
                    html,
                    "<td class=\"missing\">no matching reference found</td></tr>"
                );
            }
        }
    }
    if !rows.is_empty() {
        html.push_str("</table>\n");
    }
    html.push_str("</body>\n</html>\n");
    html
}
