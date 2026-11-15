//! Builds Home project-library audit scenes.
use super::super::Failure;
use super::super::application::{App, AppHarness, Theme, harness, harness_sized, preferences};
use egui_kittest::kittest::{By, Queryable};
use engine::ByteBudget;
use engine::Command;
use project::ProjectId;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use vibecut::app::Workspace;
use vibecut::egui;
use vibecut::i18n::Locale;

pub(super) fn home(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    home_with(theme, scale, true)
}

/// Home on a first launch: the welcome dialog over the empty library.
pub(super) fn home_welcome(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    home_with(theme, scale, false)
}

fn home_with(theme: Theme, scale: f32, welcome_seen: bool) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let workspace = Workspace::new(
        directory.path().join("recovery"),
        vibecut::preferences::Preferences {
            welcome_seen,
            ..preferences(theme, Locale::En)
        },
        None,
    )?;
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    // Discovery starts in the background; Home shows New/Open once it settles.
    for _ in 0..500 {
        app.run_steps(1);
        if !app.state().busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    Ok(app)
}

/// Home at the measured desktop window size with a full row of projects.
pub(super) const HOME_WINDOW: [f32; 2] = [1728.0, 1030.0];

pub(super) fn home_projects(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("projects");
    {
        let mut library = engine::library::ProjectLibrary::open(
            &root,
            ByteBudget::new(64 << 20)?,
            engine::footage::uncorrected(),
        )?;
        for id in 1..=10u64 {
            library.execute(
                ProjectId(id),
                Command::CreateProject {
                    id: ProjectId(id),
                    name: format!("Draft {id}").into(),
                    fps: 30,
                },
            )?;
        }
    }
    let workspace = Workspace::with_projects(
        directory.path().join("recovery"),
        root,
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness_sized(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
        HOME_WINDOW,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("home audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    Ok(app)
}

pub(super) fn projects_trash(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("projects");
    {
        let mut library = engine::library::ProjectLibrary::open(
            &root,
            ByteBudget::new(64 << 20)?,
            engine::footage::uncorrected(),
        )?;
        for id in 1..=6u64 {
            library.execute(
                ProjectId(id),
                Command::CreateProject {
                    id: ProjectId(id),
                    name: format!("Draft {id}").into(),
                    fps: 30,
                },
            )?;
        }
        for id in [2, 5] {
            library.execute(ProjectId(id), Command::DeleteProject {})?;
        }
    }
    let workspace = Workspace::with_projects(
        directory.path().join("recovery"),
        root,
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness_sized(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
        HOME_WINDOW,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("home trash audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.get(By::new().predicate(|node| node.author_id() == Some("projects.trash")))
        .click();
    app.run_steps(2);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("home trash audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Keep the pointer off the toggle so the capture shows its selected state,
    // not the click's hover surface.
    app.hover_at(egui::pos2(4.0, 4.0));
    app.run_steps(2);
    Ok(app)
}

pub(super) fn projects_scene(
    theme: Theme,
    scale: f32,
    dialog: Option<&str>,
) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("projects");
    {
        let mut library = engine::library::ProjectLibrary::open(
            &root,
            ByteBudget::new(64 << 20)?,
            engine::footage::uncorrected(),
        )?;
        for (id, name) in [
            (1, "Product launch"),
            (2, "Tiếng Việt: bản dựng"),
            (3, "Monthly recap"),
        ] {
            library.execute(
                ProjectId(id),
                Command::CreateProject {
                    id: ProjectId(id),
                    name: name.into(),
                    fps: 30,
                },
            )?;
        }
    }
    let workspace = Workspace::with_projects(
        directory.path().join("recovery"),
        root,
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("projects audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    if let Some(dialog) = dialog {
        app.get(
            By::new()
                .predicate(|node| node.author_id() == Some("projects.card.0000000000000001.menu")),
        )
        .click();
        app.run_steps(2);
        let id = format!("projects.card.0000000000000001.{dialog}");
        app.get(By::new().predicate(move |node| node.author_id() == Some(id.as_str())))
            .click();
        app.run_steps(2);
    }
    Ok(app)
}

/// Home's brand kit page with no kit yet: the intro and every section's add tile (reference
/// 14b/111111).
pub(super) fn home_brand_kit(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    home_brand_kit_with(theme, scale, false)
}

/// Home's brand kit page with a file in each file section and two font families.
pub(super) fn home_brand_kit_filled(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    home_brand_kit_with(theme, scale, true)
}

fn home_brand_kit_with(theme: Theme, scale: f32, filled: bool) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    if filled {
        fill_brand_kit(&directory.path().join("brand_kits"))?;
    }
    let workspace = Workspace::new(
        directory.path().join("recovery"),
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness_sized(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
        HOME_WINDOW,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    for open in [false, true] {
        if open {
            app.get(By::new().predicate(|node| node.author_id() == Some("home.nav.brand_kit")))
                .click();
            app.run_steps(2);
        }
        while app.state().busy() {
            app.run_steps(1);
            if Instant::now() >= deadline {
                return Err("home brand kit audit timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    // Keep the pointer off the rail so the capture shows the selected row, not its hover.
    app.hover_at(egui::pos2(4.0, 4.0));
    app.run_steps(2);
    Ok(app)
}

/// A store at `root` whose active kit holds one file per file section and two font families.
/// The store is closed again so the scene's own process can take its writer lease.
pub(super) fn fill_brand_kit(root: &Path) -> Result<(), Failure> {
    use engine::control::BrandMediaSection;
    let store = libraries::brand_kit::BrandKit::open(root)?;
    let kit = store.create("Brand kit")?;
    store.set_active(Some(kit))?;
    let sources = tempfile::tempdir()?;
    for (section, name) in [
        (BrandMediaSection::Video, "Intro sting.mp4"),
        (BrandMediaSection::Image, "Logo.png"),
        (BrandMediaSection::Sticker, "Badge.png"),
        (BrandMediaSection::Background, "Backdrop.png"),
        (BrandMediaSection::Music, "Theme.mp3"),
    ] {
        let path = sources.path().join(name);
        std::fs::write(&path, name.as_bytes())?;
        store.add_media(kit, section, &path, name)?;
    }
    for family in ["Inter", "Noto Serif"] {
        store.add_font(kit, family)?;
    }
    Ok(())
}
