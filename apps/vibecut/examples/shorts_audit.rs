//! Offscreen review evidence; deliberately separate from the test suite.
#[path = "../tests/support/shorts.rs"]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use vibecut::{
    catalog::{Locale, Preferences},
    egui,
    playback::{Files, Sources},
    shorts::{Decision, Shorts},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/ui-audit/shorts"));
    std::fs::create_dir_all(&output)?;
    let local_asset = std::env::var_os("SHORTS_AUDIT_VIDEO")
        .map(|path| media::SourceAsset::import(media::AssetId(1), PathBuf::from(path)))
        .transpose()?;
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>Shorts UI audit</title><style>body{font:16px system-ui;background:#18191c;color:white}img{width:48%;vertical-align:top;margin:0.5%}</style><h1>Shorts UI audit</h1><p>Injected candidates; source frames are local when SHORTS_AUDIT_VIDEO is set. No provider calls.</p>",
    );
    for scene in ["pick", "setup", "review"] {
        for dark in [true, false] {
            for locale in [Locale::En, Locale::Vi] {
                for scale in [1.0, 2.0] {
                    for (viewport, width, height) in
                        [("min", 1280.0, 800.0), ("desktop", 1440.0, 1000.0)]
                    {
                        let preferences = Preferences {
                            locale,
                            dark,
                            ..Default::default()
                        };
                        let mut source = fixture::source();
                        let preview: Arc<dyn Sources> = if let Some(asset) = &local_asset {
                            let mut data = source.project.into_data();
                            data.assets = vec![asset.clone()];
                            source.project = project::Project::new(data)?;
                            Arc::new(Files)
                        } else {
                            Arc::new(fixture::Pixels)
                        };
                        let mut app = Shorts::with_generator(
                            preferences,
                            engine::ByteBudget::new(768 << 20)?,
                            Arc::new(fixture::Once(Mutex::new(Some(source)))),
                            preview,
                        )?;
                        app.captions = false;
                        if scene != "pick" {
                            app.pick_video(std::env::temp_dir().join("Interview.mp4"))?;
                        }
                        let mut h = Harness::builder()
                            .with_size(egui::vec2(width, height))
                            .with_pixels_per_point(scale)
                            .build_ui_state(|ui, a: &mut Shorts| a.ui(ui), app);
                        h.run_steps(3);
                        if scene == "review" {
                            h.get(
                                By::new().predicate(|n| n.author_id() == Some("shorts.generate")),
                            )
                            .click();
                            h.run_steps(1);
                            fixture::wait(h.state_mut());
                            h.run_steps(3);
                            h.get(By::new().predicate(|n| n.author_id() == Some("shorts.accept")))
                                .click();
                            h.run_steps(2);
                            if h.state()
                                .reviews()
                                .first()
                                .is_none_or(|r| r.decision != Decision::Accepted)
                            {
                                return Err("review action did not apply".into());
                            }
                            let deadline = Instant::now() + Duration::from_secs(60);
                            while h
                                .state()
                                .preview()
                                .is_none_or(|p| p.playback.frame().is_none())
                            {
                                h.state_mut().tick();
                                if let Some(error) = h.state().error() {
                                    let mut detail = error.to_string();
                                    let mut current = std::error::Error::source(error);
                                    for _ in 0..8 {
                                        let Some(source) = current else { break };
                                        detail.push_str(": ");
                                        detail.push_str(&source.to_string());
                                        current = source.source();
                                    }
                                    return Err(detail.into());
                                }
                                if let Some(error) =
                                    h.state().preview().and_then(|p| p.playback.error())
                                {
                                    let mut detail = error.to_string();
                                    let mut current = std::error::Error::source(error.as_ref());
                                    for _ in 0..8 {
                                        let Some(source) = current else { break };
                                        detail.push_str(": ");
                                        detail.push_str(&source.to_string());
                                        current = source.source();
                                    }
                                    return Err(detail.into());
                                }
                                if Instant::now() > deadline {
                                    return Err("preview unavailable".into());
                                }
                                std::thread::yield_now();
                            }
                            h.run_steps(3);
                        }
                        let theme = if dark { "dark" } else { "light" };
                        let language = if locale == Locale::En { "en" } else { "vi" };
                        let name = format!(
                            "shorts-{scene}-{theme}-{language}-{viewport}@{}x.png",
                            scale as u32
                        );
                        h.render()
                            .map_err(std::io::Error::other)?
                            .save(output.join(&name))?;
                        html.push_str(&format!(
                        "<h2>{scene} · {theme} · {language} · {viewport} · {scale}x</h2><img src='{name}'>"
                    ));
                    }
                }
            }
        }
    }
    std::fs::write(output.join("index.html"), html)?;
    println!("{}", output.join("index.html").display());
    Ok(())
}
