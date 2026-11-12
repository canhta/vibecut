//! Private implementation of the offscreen UI audit example.
//!
//! The composition root in `ui_audit.rs` keeps the CLI entry, the two existing
//! test-fixture module paths and the example tests. This module owns the
//! dispatch into scene builders, README capture/publication and the gallery.
#![allow(
    clippy::indexing_slicing,
    reason = "an offline audit capture indexes its own manifest and fixed-size frame buffers; a panic aborts the tool, never a person's project"
)]
mod application;
mod fixture;
mod readme;
mod report;
mod scenes;

use std::path::{Path, PathBuf};

pub(crate) use application::Failure;
#[cfg(test)]
pub(crate) use application::{App, Theme};
pub(crate) use readme::readme_media;
#[cfg(test)]
pub(crate) use readme::{finish_publication, publish_outputs, run_tool};
#[cfg(test)]
pub(super) use scenes::inspector_mixed_vi;

use fixture::settle;
use report::{Row, copy_reference, index_html, opaque, reference_root};
use scenes::select;

pub(crate) fn run(args: Vec<String>) -> Result<(), Failure> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize()?;
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("target"),
    };
    if args.first().is_some_and(|arg| arg == "--readme") {
        if args.len() != 4 || args.get(2).is_none_or(|arg| arg != "--source") {
            return Err("usage: cargo xtask ui-audit --readme hero|timeline --source PATH".into());
        }
        return readme_media(&root, &target, &args[1], Path::new(&args[3]));
    }
    let output = target.join("ui-audit");
    std::fs::create_dir_all(&output)?;
    let references = reference_root(&root);
    let selected = select(args.into_iter())?;
    let mut rows = Vec::new();
    for (scene, theme) in &selected {
        let variant = format!("{}-{}", scene.name, theme.key());
        let mut images = Vec::new();
        for scale in application::SCALES {
            let mut app = (scene.build)(*theme, scale)?;
            app.run_steps(2);
            settle(&mut app)?;
            let image = app.render().map_err(std::io::Error::other)?;
            let image = opaque(image, vibecut::window_color(theme.dark()));
            let file = format!("{variant}@{}x.png", scale as u32);
            image.save(output.join(&file))?;
            if (scene.name.starts_with("assets-")
                || scene.name.starts_with("export-")
                || scene.name.starts_with("inspector-effects"))
                && scale == 1.0
            {
                let geometry:Vec<_>=app.state().inspect()?.widgets.into_iter().map(|widget|serde_json::json!({
                    "id":widget.id,"enabled":widget.enabled,
                    "rect":[widget.rect.left(),widget.rect.top(),widget.rect.width(),widget.rect.height()]
                })).collect();
                std::fs::write(
                    output.join(format!("{variant}-geometry.json")),
                    serde_json::to_vec_pretty(&geometry)?,
                )?;
            }
            images.push((file, scale));
        }
        let reference = scene.reference(*theme).and_then(|relative| {
            copy_reference(references.as_deref(), &output, &variant, relative)
        });
        rows.push(Row {
            variant,
            theme: theme.key(),
            images,
            reference,
        });
        eprintln!("ui-audit: {} rendered", scene.name);
    }
    let index = output.join("index.html");
    std::fs::write(&index, index_html(&rows))?;
    println!("ui-audit: {}", index.display());
    Ok(())
}
