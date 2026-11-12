//! Builds the assets-window audit scene, thumbnail fixture.
use super::super::Failure;
use super::super::application::{App, AppHarness, Theme, harness_sized};
use super::super::fixture::{CompletedProbe, ProgressProbe, empty_editor};
use egui_kittest::kittest::{By, Queryable};
use media::MediaError;
use media::SourceAsset;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use time::Tick;
use vibecut::i18n::Locale;

/// Uses the area slot at the measured editor size, independent of sibling work.
pub(super) struct EffectsClock(std::sync::atomic::AtomicU64);
impl engine::jobs::Clock for EffectsClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}
pub(super) fn assets_scene(theme: Theme, scale: f32, state: &str) -> Result<AppHarness, Failure> {
    let locale = if state.ends_with("-vi") {
        Locale::Vi
    } else {
        Locale::En
    };
    let state = state.strip_suffix("-vi").unwrap_or(state);
    let phase3 = state.ends_with("-phase3");
    let state = state.strip_suffix("-phase3").unwrap_or(state);
    let mut sound_library = None;
    let mut editor = empty_editor(theme, locale)?;
    let clock = Arc::new(EffectsClock(std::sync::atomic::AtomicU64::new(0)));
    editor.set_clock(clock.clone());
    if matches!(state, "media" | "offline") {
        editor.set_import_probe(Arc::new(CompletedProbe))?;
        editor.set_thumbnailer(Arc::new(AssetsThumbnail))?;
        editor.drop_files(vec![
            std::env::temp_dir().join("Interview.mp4"),
            std::env::temp_dir().join("audio-music.wav"),
        ])?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while editor.importing() {
            editor.tick();
            if Instant::now() >= deadline {
                return Err("assets audit import timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    editor.preferences.assets_tab = match state {
        "effects" => vibecut::AssetsTab::Effects,
        "transitions" => vibecut::AssetsTab::Transitions,
        "text" => vibecut::AssetsTab::Text,
        "captions" => vibecut::AssetsTab::Captions,
        "offline" => vibecut::AssetsTab::Sounds,
        "music" => vibecut::AssetsTab::Sounds,
        _ => vibecut::AssetsTab::Media,
    };
    if state == "offline" {
        let directory = tempfile::tempdir()?;
        let library = Arc::new(libraries::sounds::SoundLibrary::open(directory.path())?);
        library.save(&libraries::sounds::SoundResult {
            id: 1,
            name: "Night Drive".into(),
            author: "BCD Studio".into(),
            licence: "Creative Commons 0".into(),
            attribution: "Night Drive by BCD Studio (Creative Commons 0)".into(),
            source_url: "https://freesound.org/s/1/".into(),
            audio_url: "https://cdn.example/1.mp3".into(),
            duration_ms: 31_000,
            downloads: 100,
        })?;
        editor.set_sound_library(library)?;
        sound_library = Some(directory);
    }
    if state == "music" {
        let directory = tempfile::tempdir()?;
        let library = Arc::new(libraries::music::MusicLibrary::open(directory.path())?);
        for (id, title, author, licence, duration) in [
            (
                "audit-1",
                "Night Walk",
                "Example Artist",
                "CC BY 4.0",
                31_000,
            ),
            ("audit-2", "Paper Planes", "Studio Nine", "CC0 1.0", 128_000),
            ("audit-3", "Quiet Harbour", "Mara Vale", "CC BY 4.0", 94_000),
        ] {
            library.save(&libraries::music::MusicResult {
                id: id.into(),
                title: title.into(),
                author: author.into(),
                licence: licence.into(),
                licence_url: if licence == "CC0 1.0" {
                    "https://creativecommons.org/publicdomain/zero/1.0/".into()
                } else {
                    "https://creativecommons.org/licenses/by/4.0/".into()
                },
                attribution: format!("\"{title}\" by {author} is licensed under {licence}."),
                source_url: format!("https://commons.example/wiki/File:{id}.ogg"),
                audio_url: format!("https://cdn.example/{id}.ogg"),
                duration_ms: duration,
            })?;
        }
        let client = libraries::music::MusicClient::new(None, Arc::new(MusicAuditSource));
        editor.set_music(vibecut::Music::with_client(
            Arc::new(vibecut::services::music::MusicSession::new(library)),
            client,
            engine::ByteBudget::new(128 << 20)?,
        )?);
        sound_library = Some(directory);
    }
    if state == "loading" {
        editor.set_import_probe(Arc::new(ProgressProbe::new()))?;
        editor.drop_files(vec![
            std::env::temp_dir().join("one.mp4"),
            std::env::temp_dir().join("two.mp4"),
        ])?;
    }
    if state == "error" {
        editor.start_import(vec![std::env::temp_dir().join("missing-audit-media.mp4")])?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while editor.importing() {
            editor.tick();
            if Instant::now() >= deadline {
                return Err("assets audit refusal timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let mut app = harness_sized(App::Assets(Box::new(editor)), scale, [1727.0, 1029.0]);
    app.state_mut()._sound_library = sound_library;
    app.run_steps(4);
    if state == "music" {
        app.get(By::new().predicate(|node| node.author_id() == Some("audio.section.music")))
            .click();
        app.run_steps(40);
    }
    if phase3 {
        if let App::Assets(editor) = &mut app.state_mut().app {
            editor.settle_previews(vibecut::effect_previews::SETTLE_BUDGET)?;
        }
        clock.0.store(100, Ordering::SeqCst);
        app.run_steps(1);
    }
    Ok(app)
}

/// Recorded music page for the audit render; it never opens a socket.
struct MusicAuditSource;
impl transport::HttpTransport for MusicAuditSource {
    fn send(
        &self,
        request: transport::HttpRequest,
    ) -> Result<transport::HttpResponse, transport::Error> {
        self.send_bounded(request, 1 << 20)
    }
    fn send_bounded(
        &self,
        _: transport::HttpRequest,
        _: usize,
    ) -> Result<transport::HttpResponse, transport::Error> {
        let tracks = [
            (
                "audit-1",
                "Night Walk",
                "Example Artist",
                "by",
                "4.0",
                31_000,
            ),
            (
                "audit-2",
                "Paper Planes",
                "Studio Nine",
                "cc0",
                "1.0",
                128_000,
            ),
            ("audit-3", "Quiet Harbour", "Mara Vale", "by", "4.0", 94_000),
        ];
        let results: Vec<_> = tracks
            .iter()
            .map(|(id, title, author, licence, version, duration)| {
                let family = if *licence == "cc0" { "CC0" } else { "CC BY" };
                let licence_url = if *licence == "cc0" {
                    "https://creativecommons.org/publicdomain/zero/1.0/"
                } else {
                    "https://creativecommons.org/licenses/by/4.0/"
                };
                serde_json::json!({
                    "id": id,
                    "title": title,
                    "creator": author,
                    "license": licence,
                    "license_version": version,
                    "license_url": licence_url,
                    "attribution": format!("\"{title}\" by {author} is licensed under {family} {version}."),
                    "foreign_landing_url": format!("https://commons.example/wiki/File:{id}.ogg"),
                    "url": format!("https://cdn.example/{id}.ogg"),
                    "duration": duration,
                })
            })
            .collect();
        let body = serde_json::json!({
            "result_count": 3,
            "page_count": 1,
            "page_size": 15,
            "page": 1,
            "results": results,
        });
        Ok(transport::HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: serde_json::to_vec(&body).map_err(|_| transport::Error::Payload)?,
        })
    }
}

/// Own generated pixels, injected through the real thumbnail publication seam.
pub(super) struct AssetsThumbnail;
impl vibecut::ThumbnailSource for AssetsThumbnail {
    fn thumbnail(
        &self,
        _: &SourceAsset,
        _: Tick,
        _: u32,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<media::Thumbnail> {
        if cancelled.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        Ok(media::Thumbnail {
            width: 120,
            height: 73,
            frame: Tick::ZERO,
            pixels: [80, 103, 126, 255].repeat(120 * 73),
        })
    }
}
