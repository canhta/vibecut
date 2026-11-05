//! Offscreen presentation evidence only; never part of the test or CI oracle.
#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use std::{path::PathBuf, sync::Arc};
use vibecut::{
    Editor,
    catalog::{Locale, Preferences},
    egui,
    playback::{Files, Playback},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/ui-audit/export"));
    std::fs::create_dir_all(&output)?;
    for (scale, dark, locale) in [(1.0, true, Locale::En), (2.0, false, Locale::Vi)] {
        for empty in [false, true] {
            let mut d = project::Project::empty(project::ProjectId(18), "Video — Hà Nội".into())?
                .into_data();
            d.canvas.width = 64;
            d.canvas.height = 64;
            if !empty {
                d.timeline.main.items.push(project::Item {
                    id: project::ItemId(1),
                    source_asset: None,
                    source_range: time::SourceRange::new(
                        time::Tick::ZERO,
                        time::Tick(time::TICKS_PER_SECOND * 10),
                    )?,
                    start: time::Tick::ZERO,
                    content: project::ItemParams::Overlay(project::OverlayParams {
                        color: project::Color([35, 50, 65, 255]),
                    }),
                    animations: Default::default(),
                    effects: vec![],
                    masks: vec![],
                });
            }
            let session = engine::Session::new(project::Project::new(d)?, 10)?;
            let playback = Playback::new(
                &session,
                engine::ByteBudget::new(768 << 20)?,
                Arc::new(Files),
            )?;
            let mut editor = Editor::new(
                session,
                playback,
                Preferences {
                    locale,
                    dark,
                    ..Default::default()
                },
            );
            editor.activate("file.export")?;
            editor.export_picked_file(output.join(if dark {
                "video-dark.mp4"
            } else {
                "video-light.mp4"
            }))?;
            let mut h = Harness::builder()
                .with_size(egui::vec2(1280.0, 800.0))
                .with_pixels_per_point(scale)
                .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
            h.run_steps(4);
            for expanded in [false, true] {
                if expanded {
                    for id in ["export.format", "export.quality", "export.audio"] {
                        h.get(By::new().predicate(|n| n.author_id() == Some(id)))
                            .click();
                        h.run_steps(4);
                    }
                }
                let path = output.join(format!(
                    "export-{}-{}-{}-{scale}x.png",
                    if empty { "empty" } else { "ready" },
                    if expanded { "expanded" } else { "collapsed" },
                    if dark { "dark-en" } else { "light-vi" }
                ));
                h.render()?.save(&path)?;
                println!("observed: {}", path.display());
            }
            if !empty {
                if h.state()
                    .export
                    .as_ref()
                    .unwrap()
                    .inspect()
                    .destination
                    .as_ref()
                    .is_some_and(|p| p.exists())
                {
                    h.get(By::new().predicate(|n| n.author_id() == Some("export.replace")))
                        .click();
                    h.run_steps(4);
                }
                h.get(By::new().predicate(|n| n.author_id() == Some("export.start")))
                    .click();
                h.run_steps(4);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                while h.state().export.as_ref().unwrap().active() {
                    if std::time::Instant::now() >= deadline {
                        return Err("audit export timeout".into());
                    }
                    h.run_steps(1);
                    std::thread::yield_now();
                }
                let job = h.state().export.as_ref().unwrap().inspect().job.unwrap();
                if job.state != engine::jobs::JobState::Completed {
                    return Err(format!("audit export failed: {:?}", job.error).into());
                }
                println!(
                    "observed: manual widget export completed {:?}",
                    h.state().export.as_ref().unwrap().inspect().destination
                );
            }
        }
    }
    Ok(())
}
