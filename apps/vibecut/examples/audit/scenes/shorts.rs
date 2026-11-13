//! Builds the Shorts-window audit scene, generation fixture.
use super::super::Failure;
use super::super::application::{App, AppHarness, AuditApp, Theme, preferences};
use super::super::fixture::shorts as shorts_fixture;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable};
use engine::ByteBudget;
use media::AssetId;
use media::SourceAsset;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use vibecut::egui;
use vibecut::i18n::Locale;
use vibecut::services::playback::Files;

/// Shared fixture, real widget path; no provider calls or visible native window.
pub(super) fn shorts_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
    scene: &str,
) -> Result<AppHarness, Failure> {
    use vibecut::{
        services::playback::Sources,
        shorts::{Decision, Importer, State as Shorts},
    };
    static ASSET: std::sync::OnceLock<SourceAsset> = std::sync::OnceLock::new();
    let source = shorts_fixture::source();
    let (preview, importer): (Arc<dyn Sources>, Importer) =
        if let Some(path) = std::env::var_os("SHORTS_AUDIT_VIDEO") {
            let asset = match ASSET.get() {
                Some(asset) => asset.clone(),
                None => {
                    let asset = SourceAsset::import(AssetId(1), PathBuf::from(path))?;
                    let _ = ASSET.set(asset.clone());
                    asset
                }
            };
            (Arc::new(Files), Arc::new(move |_| Ok(asset.clone())))
        } else {
            (
                Arc::new(shorts_fixture::Pixels),
                shorts_fixture::importer_at(0),
            )
        };
    let prefs = preferences(theme, locale);
    let opener = if scene == "generating" {
        // A generation that never finishes, so the audit can draw the working state.
        shorts_fixture::staged(|_, hooks| {
            while !hooks.cancelled() {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(engine::control::Error::Runtime(
                engine::RuntimeError::Cancelled,
            ))
        })
    } else {
        shorts_fixture::once(source).0
    };
    let mut shorts = Shorts::with_access(
        prefs.look(),
        ByteBudget::new(768 << 20)?,
        opener,
        importer,
        preview,
    )?;
    shorts.captions = false;
    if scene != "pick" {
        shorts.pick_video(std::env::temp_dir().join("Interview.mp4"))?;
    }
    let mut app = Harness::builder()
        .with_size(egui::vec2(1718.0, 989.0))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp {
            app: App::Shorts(Box::new(shorts)),
            _model_cache: None,
            _sound_library: None,
        });
    app.run_steps(3);
    if scene == "generating" {
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.generate")))
            .click();
        app.run_steps(4);
    }
    if matches!(scene, "review" | "layout" | "trim" | "results") {
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.generate")))
            .click();
        app.run_steps(1);
        let App::Shorts(shorts) = &mut app.state_mut().app else {
            return Err("missing Shorts audit app".into());
        };
        shorts_fixture::wait(shorts);
        app.run_steps(3);
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.candidate.0")))
            .click();
        app.run_steps(2);
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.accept")))
            .click();
        app.run_steps(2);
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let App::Shorts(shorts) = &mut app.state_mut().app else {
                return Err("missing Shorts audit app".into());
            };
            shorts.tick();
            if let Some(error) = shorts.error() {
                return Err(std::io::Error::other(error.to_string()).into());
            }
            if let Some(error) = shorts.preview().and_then(|p| p.error()) {
                return Err(std::io::Error::other(error).into());
            }
            if shorts.preview().is_some_and(|p| p.frame().is_some()) {
                if shorts
                    .reviews()
                    .first()
                    .is_none_or(|r| r.decision != Decision::Accepted)
                {
                    return Err("Shorts review action did not apply".into());
                }
                break;
            }
            if std::time::Instant::now() > deadline {
                return Err("Shorts preview timeout".into());
            }
            std::thread::yield_now();
        }
        app.run_steps(3);
        if scene == "results" {
            app.get(By::new().predicate(|n| n.author_id() == Some("shorts.grid")))
                .click();
            app.run_steps(3);
        }
        let tool = match scene {
            "layout" => Some("shorts.tool.layout"),
            "trim" => Some("shorts.tool.trim"),
            _ => None,
        };
        if let Some(tool) = tool {
            app.get(By::new().predicate(move |n| n.author_id() == Some(tool)))
                .click();
            app.run_steps(3);
        }
    }
    Ok(app)
}
