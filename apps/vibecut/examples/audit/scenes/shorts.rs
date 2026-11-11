use super::super::Failure;
use super::super::application::*;
use crate::shorts_fixture;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable};
use engine::ByteBudget;
use media::AssetId;
use media::SourceAsset;
use project::Project;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use vibecut::egui;
use vibecut::i18n::Locale;
use vibecut::services::playback::Files;

/// A generation that never finishes, so the audit can draw the working state.
pub(super) struct Hold;
impl vibecut::shorts::Generator for Hold {
    fn run(
        &self,
        _request: vibecut::shorts::GenerationRequest,
        _budget: ByteBudget,
        _events: &vibecut::shorts::TaskEvents,
        cancel: &engine::transcript::Cancellation,
    ) -> engine::Result<vibecut::shorts::ReviewSource> {
        while !cancel.cancelled() {
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(engine::EditError::Cancelled)
    }
}

/// Shared fixture, real widget path; no provider calls or visible native window.
pub(super) fn shorts_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
    scene: &str,
) -> Result<AppHarness, Failure> {
    use vibecut::{
        services::playback::Sources,
        shorts::{Decision, State as Shorts},
    };
    static ASSET: std::sync::OnceLock<SourceAsset> = std::sync::OnceLock::new();
    let mut source = shorts_fixture::source();
    let preview: Arc<dyn Sources> = if let Some(path) = std::env::var_os("SHORTS_AUDIT_VIDEO") {
        let asset = match ASSET.get() {
            Some(asset) => asset.clone(),
            None => {
                let asset = SourceAsset::import(AssetId(1), PathBuf::from(path))?;
                let _ = ASSET.set(asset.clone());
                asset
            }
        };
        let mut data = source.project.into_data();
        data.assets = vec![asset];
        source.project = Project::new(data)?;
        Arc::new(Files)
    } else {
        Arc::new(shorts_fixture::Pixels)
    };
    let prefs = preferences(theme, locale);
    let generator: Arc<dyn vibecut::shorts::Generator> = if scene == "generating" {
        Arc::new(Hold)
    } else {
        Arc::new(shorts_fixture::Once(Mutex::new(Some(source))))
    };
    let mut shorts = Shorts::with_generator(
        prefs.look(),
        ByteBudget::new(768 << 20)?,
        generator,
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
