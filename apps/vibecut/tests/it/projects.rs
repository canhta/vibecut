#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Command, NewItem, Session, library::ProjectLibrary};
use project::{Color, ItemParams, OverlayParams, ProjectId, Thumbnail, WriterLease};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    app::Workspace,
    egui,
    home::{ProjectsSortKey, ProjectsView},
    preferences::Preferences,
};

const CREATED: u64 = 1_780_000_000_000;
const MODIFIED: u64 = 1_780_086_400_000;

fn by(id: &'static str) -> By<'static> {
    By::new().predicate(|node| node.author_id() == Some(id))
}

struct Fixture {
    directory: tempfile::TempDir,
    projects: PathBuf,
}

impl Fixture {
    fn new(projects: &[(&str, Thumbnail)]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("projects");
        {
            let mut library = ProjectLibrary::open(
                &root,
                ByteBudget::new(128 << 20).unwrap(),
                engine::footage::uncorrected(),
            )
            .unwrap();
            for (index, (name, thumbnail)) in projects.iter().enumerate() {
                let id = ProjectId(index as u64 + 1);
                library
                    .execute(
                        id,
                        Command::CreateProject {
                            id,
                            name: (*name).into(),
                            fps: 30,
                        },
                    )
                    .unwrap();
                let path = library
                    .list()
                    .unwrap()
                    .into_iter()
                    .find(|entry| entry.id == id)
                    .unwrap()
                    .path;
                customize(&path, thumbnail.clone(), 0);
            }
        }
        Self {
            directory,
            projects: root,
        }
    }

    fn recovery(&self) -> PathBuf {
        self.directory.path().join("recovery")
    }

    fn build(&self, preferences: Preferences) -> Harness<'static, Workspace> {
        let app =
            Workspace::with_projects(self.recovery(), self.projects.clone(), preferences, None)
                .unwrap();
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .build_ui_state(|ui, app: &mut Workspace| app.ui(ui), app);
        idle(&mut harness);
        harness
    }

    fn path(&self, index: usize) -> PathBuf {
        let name = format!("{:016x}", index + 1);
        self.projects.join(name).join("project.vibecut")
    }
}

fn customize(path: &Path, thumbnail: Thumbnail, duration: i64) {
    let mut lease = WriterLease::acquire(path).unwrap();
    let mut project = lease.load().unwrap().project;
    if duration > 0 {
        let mut session = Session::new(project, 10).unwrap();
        session
            .apply(Command::InsertItem {
                placement: Default::default(),
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, Tick(duration)).unwrap(),
                    start: Tick::ZERO,
                    content: ItemParams::Overlay(OverlayParams {
                        color: Color([20, 120, 200, 255]),
                    }),
                },
            })
            .unwrap();
        project = session.project().as_ref().clone();
    }
    lease.save(&project).unwrap();
    let mut metadata = lease.metadata();
    metadata.created_unix_ms = CREATED;
    metadata.modified_unix_ms = MODIFIED;
    metadata.thumbnail = thumbnail;
    lease.save_metadata(&metadata).unwrap();
}

fn idle(harness: &mut Harness<'static, Workspace>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        harness.run_steps(1);
        if !harness.state().busy() {
            // A queued click may only start its worker after the frame that handled it.
            harness.run_steps(2);
            if !harness.state().busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "project page worker timeout");
    }
    harness.run_steps(2);
}

fn missing() -> Thumbnail {
    Thumbnail::Missing
}

#[test]
fn search_sort_and_view_persist_through_the_preference_store() {
    let fixture = Fixture::new(&[
        ("Alpha", missing()),
        ("Beta", missing()),
        ("Gamma", missing()),
    ]);
    let mut harness = fixture.build(Preferences::default());
    let ids = harness
        .state()
        .projects
        .displayed_ids(harness.state().preferences.projects);
    assert_eq!(
        ids,
        vec![ProjectId(3), ProjectId(2), ProjectId(1)],
        "created sort defaults to newest first"
    );

    harness.get(by("projects.search")).click();
    harness.run_steps(1);
    harness.get(by("projects.search")).type_text("bet");
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects),
        vec![ProjectId(2)]
    );
    assert!(
        harness
            .query_all(by("projects.card.0000000000000001"))
            .count()
            == 0
    );
    assert!(
        harness
            .query_all(by("projects.card.0000000000000003"))
            .count()
            == 0
    );

    harness.get(by("projects.search")).click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects)
            .len(),
        3
    );

    harness.get(by("projects.search")).type_text("zzz");
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects)
            .is_empty()
    );
    assert!(harness.query_by_label("No matches").is_some());
    harness.get(by("projects.search")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(2);
    harness.get(by("projects.search")).type_text("   ");
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects)
            .len(),
        3,
        "blank search is treated as no filter"
    );
    harness.get(by("projects.search")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(2);

    harness.get(by("projects.sort")).click();
    harness.run_steps(2);
    assert!(
        harness.query_all(by("projects.sort.modified")).count() == 1,
        "modified is offered in the sort menu"
    );
    harness.get(by("projects.sort.name")).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().preferences.projects.sort,
        ProjectsSortKey::Name
    );
    assert_eq!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects),
        vec![ProjectId(1), ProjectId(2), ProjectId(3)]
    );
    harness.get(by("projects.sort")).click();
    harness.run_steps(2);
    harness.get(by("projects.sort.name")).click();
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects),
        vec![ProjectId(3), ProjectId(2), ProjectId(1)]
    );

    harness.get(by("projects.view.list")).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().preferences.projects.view,
        ProjectsView::List
    );

    let store = vibecut::preferences::PreferenceStore::new(fixture.projects.join("prefs.json"));
    store.save(&harness.state().preferences).unwrap();
    let loaded = store.load_with_warning().0;
    assert_eq!(loaded.projects, harness.state().preferences.projects);

    let unicode = Fixture::new(&[("Tiếng Việt", missing())]);
    let mut harness = unicode.build(Preferences::default());
    for query in ["tiếng", "tie\u{302}\u{301}ng"] {
        harness.get(by("projects.search")).click();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get(by("projects.search")).type_text(query);
        harness.run_steps(2);
        assert_eq!(
            harness
                .state()
                .projects
                .displayed_ids(harness.state().preferences.projects),
            vec![ProjectId(1)],
            "canonically equivalent Vietnamese search must match the stored name"
        );
    }
}

#[test]
fn shift_selection_enables_bulk_actions_and_delete_confirmation_lists_names() {
    let fixture = Fixture::new(&[
        ("Alpha", missing()),
        ("Beta", missing()),
        ("Gamma", missing()),
    ]);
    let mut harness = fixture.build(Preferences::default());
    assert!(
        harness
            .get(by("projects.toolbar.duplicate"))
            .accesskit_node()
            .is_disabled(),
        "bulk duplicate is disabled without a selection"
    );
    assert!(
        harness
            .get(by("projects.toolbar.delete"))
            .accesskit_node()
            .is_disabled(),
        "bulk delete is disabled without a selection"
    );
    harness.get(by("projects.card.0000000000000001")).click();
    harness.run_steps(2);
    assert_eq!(harness.state().projects.selection().len(), 1);
    harness
        .get(by("projects.card.0000000000000003"))
        .click_modifiers(egui::Modifiers::SHIFT);
    harness.run_steps(2);
    assert_eq!(
        harness.state().projects.selection().len(),
        3,
        "shift-click selects the inclusive display range"
    );
    for (target, expected) in [
        ("projects.card.0000000000000002", 2),
        ("projects.card.0000000000000003", 3),
    ] {
        harness
            .get(by(target))
            .click_modifiers(egui::Modifiers::SHIFT);
        harness.run_steps(2);
        assert_eq!(
            harness.state().projects.selection().len(),
            expected,
            "reversing the range replaces rather than accumulates targets"
        );
    }
    assert!(
        !harness
            .get(by("projects.toolbar.duplicate"))
            .accesskit_node()
            .is_disabled()
    );

    harness.get(by("projects.toolbar.delete")).click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Alpha, Beta, Gamma").is_some());
    assert!(
        harness.query_all(by("projects.delete.input")).count() == 0,
        "the shipped typed DELETE gate is not copied"
    );
    assert!(
        harness
            .query_by_label(
                "Moved projects stay in Trash for 30 days, then they are permanently deleted."
            )
            .is_some(),
        "the move-to-trash confirmation states the retention period"
    );
    harness.get(by("projects.delete.cancel")).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().projects.entries().len(),
        3,
        "cancelling the confirmation keeps every selected project"
    );
    harness.get(by("projects.toolbar.delete")).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.query_all(by("projects.delete.confirm")).count() == 0);
    assert_eq!(harness.state().projects.entries().len(), 3);
    harness.get(by("projects.toolbar.delete")).click();
    harness.run_steps(2);
    let paths: Vec<PathBuf> = (0..3).map(|index| fixture.path(index)).collect();
    harness.get(by("projects.delete.confirm")).click();
    idle(&mut harness);
    assert!(harness.state().projects.entries().is_empty());
    assert!(paths.iter().all(|path| !path.exists()));
}

#[test]
fn rename_blank_collision_cancel_and_success_never_bypass_the_command() {
    let fixture = Fixture::new(&[("Alpha", missing()), ("Beta", missing())]);
    let mut harness = fixture.build(Preferences::default());
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.rename"))
        .click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Rename project").is_some());
    harness.get(by("projects.rename.cancel")).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().projects.entries()[0].name.as_str(),
        "Alpha",
        "cancel leaves the project name unchanged"
    );

    open_rename(&mut harness, 1);
    harness.get(by("projects.rename.input")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.get(by("projects.rename.save")).click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label("Enter a name that isn't used yet.")
            .is_some()
    );
    assert_eq!(harness.state().projects.entries()[0].name.as_str(), "Alpha");

    harness.get(by("projects.rename.input")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get(by("projects.rename.input")).type_text("Beta");
    harness.get(by("projects.rename.save")).click();
    idle(&mut harness);
    assert!(
        harness
            .query_by_label("Enter a name that isn't used yet.")
            .is_some(),
        "a colliding name stays in the dialog with its error"
    );
    assert_eq!(harness.state().projects.entries()[0].name.as_str(), "Alpha");

    harness.get(by("projects.rename.input")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get(by("projects.rename.input"))
        .type_text("../<demo> & tiếng Việt");
    harness.get(by("projects.rename.save")).click();
    idle(&mut harness);
    assert_eq!(
        harness.state().projects.entries()[0].name.as_str(),
        "../<demo> & tiếng Việt"
    );
    assert!(
        fixture.path(0).exists(),
        "names are display text, never paths"
    );
    open_rename(&mut harness, 1);
    harness.get(by("projects.rename.input")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get(by("projects.rename.input")).type_text("Delta");
    harness.get(by("projects.rename.save")).click();
    idle(&mut harness);
    assert!(harness.query_by_label("Rename project").is_none());
    assert_eq!(harness.state().projects.entries()[0].name.as_str(), "Delta");
}

fn open_rename(harness: &mut Harness<'static, Workspace>, index: u64) {
    let menu = match index {
        1 => "projects.card.0000000000000001.menu",
        _ => "projects.card.0000000000000002.menu",
    };
    let rename = match index {
        1 => "projects.card.0000000000000001.rename",
        _ => "projects.card.0000000000000002.rename",
    };
    harness.get(by(menu)).click();
    harness.run_steps(2);
    harness.get(by(rename)).click();
    harness.run_steps(2);
}

#[test]
fn info_dialog_shows_main_duration_created_modified_and_hides_the_id() {
    let fixture = Fixture::new(&[("Alpha", missing())]);
    customize(&fixture.path(0), missing(), TICKS_PER_SECOND * 5);
    let mut harness = fixture.build(Preferences::default());
    assert!(
        harness.query_by_label("Modified May 29, 2026").is_none(),
        "modified date belongs to the info dialog, not the card"
    );
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.info"))
        .click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Duration 00:05").is_some());
    assert!(harness.query_by_label("Created May 28, 2026").is_some());
    assert!(harness.query_by_label("Modified May 29, 2026").is_some());
    assert!(harness.query_by_label("Project ID 00000001").is_none());
    assert_eq!(
        harness.state().projects.entries()[0].short_id.len(),
        8,
        "the display id is exactly eight characters"
    );
}

#[test]
fn missing_and_unavailable_thumbnails_render_placeholders_and_ready_does_not() {
    let fixture = Fixture::new(&[
        ("Missing", Thumbnail::Missing),
        ("Unavailable", Thumbnail::Unavailable),
        (
            "Ready",
            Thumbnail::Ready {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            },
        ),
    ]);
    let mut harness = fixture.build(Preferences::default());
    let held = WriterLease::acquire(&fixture.path(0)).unwrap();
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.thumbnail.refresh"))
        .click();
    idle(&mut harness);
    assert!(harness.query_by_label("Couldn't refresh.").is_some());
    assert!(matches!(
        harness.state().projects.entries()[0].thumbnail,
        Thumbnail::Missing
    ));
    drop(held);
    assert_eq!(
        harness
            .query_all(By::new().predicate(|node| {
                node.author_id()
                    .is_some_and(|id| id.ends_with(".thumbnail.placeholder"))
            }))
            .count(),
        2
    );
    assert!(
        harness
            .query_all(by("projects.card.0000000000000003.thumbnail.placeholder"))
            .count()
            == 0
    );
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.thumbnail.refresh"))
        .click();
    idle(&mut harness);
    assert!(
        !matches!(
            harness.state().projects.entries()[0].thumbnail,
            Thumbnail::Missing
        ),
        "the page refreshes project-owned thumbnails through project.thumbnail"
    );
    assert!(
        harness.query_by_label("Couldn't refresh.").is_none(),
        "a successful refresh clears the previous refusal"
    );
}

#[test]
fn duplicate_reports_progress_and_failure_without_mutating_the_source() {
    let fixture = Fixture::new(&[("Alpha", missing())]);
    let mut harness = fixture.build(Preferences::default());
    harness.get(by("projects.card.0000000000000001")).click();
    harness.run_steps(2);
    harness.get(by("projects.toolbar.duplicate")).click();
    harness.run_steps(1);
    assert!(harness.query_all(by("projects.toolbar.duplicate")).count() == 1);
    assert!(
        harness
            .query_all(By::new().predicate(|node| {
                node.author_id()
                    .is_some_and(|id| id == "projects.toolbar.duplicate")
            }))
            .count()
            == 1
    );
    let progress = harness.query_all(By::new().predicate(|node| {
        node.label()
            .is_some_and(|label| label.starts_with("Duplicating"))
    }));
    let progress_seen = progress.count();
    idle(&mut harness);
    assert!(
        progress_seen == 1 || harness.state().projects.entries().len() == 2,
        "duplicate either shows its in-progress state or has already completed"
    );
    assert_eq!(harness.state().projects.entries().len(), 2);

    drop(harness);
    for held_index in [0, 1] {
        let fixture = Fixture::new(&[("Alpha", missing()), ("Beta", missing())]);
        let mut harness = fixture.build(Preferences::default());
        let held = WriterLease::acquire(&fixture.path(held_index)).unwrap();
        harness.get(by("projects.card.0000000000000001")).click();
        harness.run_steps(2);
        harness
            .get(by("projects.card.0000000000000002"))
            .click_modifiers(egui::Modifiers::SHIFT);
        harness.run_steps(2);
        harness.get(by("projects.toolbar.duplicate")).click();
        idle(&mut harness);
        assert!(
            harness
                .query_by_label("Couldn't copy some projects.")
                .is_some()
        );
        let entries = harness.state().projects.entries();
        assert_eq!(
            entries.len(),
            3,
            "one locked source must not prevent the other copy"
        );
        assert_eq!(entries[0].name.as_str(), "Alpha");
        assert_eq!(entries[1].name.as_str(), "Beta");
        assert_eq!(
            entries[2].name.as_str(),
            if held_index == 0 { "Beta" } else { "Alpha" }
        );
        assert!(fixture.path(0).exists() && fixture.path(1).exists());
        drop(held);
    }
}

#[test]
fn delete_failure_keeps_the_project_and_reports_it() {
    for names in [vec!["Alpha"], vec!["Alpha", "Beta"]] {
        let projects: Vec<_> = names.iter().map(|name| (*name, missing())).collect();
        let fixture = Fixture::new(&projects);
        let path = fixture.path(0);
        let mut harness = fixture.build(Preferences::default());
        let held = WriterLease::acquire(&path).unwrap();
        harness.get(by("projects.card.0000000000000001")).click();
        harness.run_steps(2);
        if names.len() == 2 {
            harness
                .get(by("projects.card.0000000000000002"))
                .click_modifiers(egui::Modifiers::SHIFT);
            harness.run_steps(2);
        }
        harness.get(by("projects.toolbar.delete")).click();
        harness.run_steps(2);
        harness.get(by("projects.delete.confirm")).click();
        idle(&mut harness);
        assert!(
            harness
                .query_by_label("Couldn't move some projects to Trash.")
                .is_some()
        );
        assert_eq!(harness.state().projects.entries().len(), 1);
        assert_eq!(harness.state().projects.entries()[0].id, ProjectId(1));
        assert!(path.exists());
        if names.len() == 2 {
            assert!(!fixture.path(1).exists());
        }
        drop(held);
    }
}

#[test]
fn new_project_is_created_in_the_managed_catalog_and_opened() {
    let fixture = Fixture::new(&[]);
    let mut harness = fixture.build(Preferences::default());
    assert!(harness.query_by_label("No projects yet").is_some());
    harness.get(by("project.create")).click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Loading project…").is_some());
    idle(&mut harness);
    assert!(harness.state().error().is_none());
    assert_eq!(
        harness
            .state()
            .focused_editor()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "New project"
    );
    assert_eq!(
        std::fs::read_dir(&fixture.projects)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count(),
        1,
        "the lifecycle command published one managed project"
    );
}

#[test]
fn old_schema_and_corrupt_projects_are_listed_with_a_reason_and_never_block_new_project() {
    let fixture = Fixture::new(&[("Alpha", missing())]);
    let old = fixture.projects.join("0000000000000002");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(
        old.join("project.vibecut"),
        br#"{"format":"vibecut.project","schema_version":1,"generator":"old","project":{}}"#,
    )
    .unwrap();
    let corrupt = fixture.projects.join("0000000000000003");
    std::fs::create_dir(&corrupt).unwrap();
    std::fs::write(corrupt.join("project.vibecut"), b"not a project").unwrap();
    let mut harness = fixture.build(Preferences::default());
    assert!(harness.query_by_label("Couldn't load projects.").is_none());
    assert_eq!(
        harness
            .state()
            .projects
            .displayed_ids(harness.state().preferences.projects),
        [ProjectId(1)]
    );
    assert!(
        harness
            .query_by_label("Couldn't read project 0000000000000002")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Couldn't read project 0000000000000003")
            .is_some()
    );
    assert!(
        harness
            .query_by_label(&format!(
                "project schema 1 is older than supported schema {}",
                project::SCHEMA_VERSION
            ))
            .is_some()
    );
    harness.get(by("project.create")).click();
    harness.run_steps(2);
    idle(&mut harness);
    assert!(harness.state().error().is_none());
    assert_eq!(
        harness
            .state()
            .focused_editor()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "New project"
    );
    assert!(old.join("project.vibecut").exists());
    assert!(corrupt.join("project.vibecut").exists());
}

#[test]
fn card_menu_opens_the_managed_project_through_the_lifecycle_adapter() {
    let fixture = Fixture::new(&[("Alpha", missing())]);
    let mut harness = fixture.build(Preferences::default());
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.open"))
        .click();
    idle(&mut harness);
    assert!(harness.state().error().is_none());
    assert_eq!(
        harness
            .state()
            .focused_editor()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "Alpha"
    );
}

#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-01-projects.png"]
fn projects_page_screenshot() {
    let fixture = Fixture::new(&[
        ("Product launch", missing()),
        ("Cooking short", ready_thumbnail([220, 90, 40])),
        ("Monthly recap", ready_thumbnail([40, 120, 200])),
        ("Podcast clip", ready_thumbnail([90, 160, 90])),
    ]);
    customize(
        &fixture.path(0),
        ready_thumbnail([200, 60, 120]),
        TICKS_PER_SECOND * 42,
    );
    let mut harness = fixture.build(Preferences::default());
    harness.get(by("projects.card.0000000000000002")).click();
    harness.run_steps(3);
    let image = harness.render().expect("offscreen render");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-01-projects.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image.save(&path).unwrap();
    eprintln!("projects screenshot: {}", path.display());
    assert!(image.width() > 0 && image.height() > 0);
}

#[test]
fn trash_lists_moved_projects_with_retention_restore_and_delete_forever() {
    let fixture = Fixture::new(&[("Alpha", missing()), ("Beta", missing())]);
    let mut harness = fixture.build(Preferences::default());
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.delete"))
        .click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Move to Trash?").is_some());
    harness.get(by("projects.delete.confirm")).click();
    idle(&mut harness);
    assert_eq!(harness.state().projects.entries().len(), 1);

    // The Trash toggle swaps the grid and each row keeps its remaining days.
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    assert!(harness.state().projects.trash_mode());
    assert_eq!(harness.query_all(by("projects.trash")).count(), 1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert_eq!(
        harness
            .state()
            .projects
            .trash()
            .map(|entry| entry.trash_days_left(now))
            .collect::<Vec<_>>(),
        vec![Some(30)],
        "the trashed row reports the retention it was moved with"
    );
    harness.get(by("projects.search")).click();
    harness.get(by("projects.search")).type_text("beta");
    harness.run_steps(2);
    assert!(
        harness.query_by_label("No matches").is_some(),
        "search filters Trash rows by name"
    );
    harness.get(by("projects.search")).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(2);

    // A trashed card offers restore and delete forever, never open.
    harness
        .get(by("projects.card.0000000000000001"))
        .click_modifiers(egui::Modifiers::NONE);
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000001.menu"))
        .click();
    harness.run_steps(2);
    assert!(
        harness
            .query_all(by("projects.card.0000000000000001.open"))
            .count()
            == 0,
        "a trashed project cannot be opened"
    );
    assert!(
        harness
            .query_all(by("projects.card.0000000000000001.restore"))
            .count()
            == 1
    );
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get(by("projects.toolbar.restore")).click();
    idle(&mut harness);
    assert_eq!(harness.state().projects.trash().count(), 0);
    assert_eq!(harness.state().projects.entries().len(), 2);
    assert!(fixture.path(0).exists(), "restore puts the directory back");
    assert!(harness.query_by_label("Trash is empty").is_some());

    // Delete forever asks for the permanent confirmation and removes the bytes.
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    harness
        .get(by("projects.card.0000000000000002.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000002.delete"))
        .click();
    harness.run_steps(2);
    harness.get(by("projects.delete.confirm")).click();
    idle(&mut harness);
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    harness
        .get(by("projects.card.0000000000000002"))
        .click_modifiers(egui::Modifiers::NONE);
    harness.run_steps(2);
    harness.get(by("projects.toolbar.purge")).click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Delete forever?").is_some());
    assert!(
        harness
            .query_by_label("These projects are deleted permanently and can't be restored.")
            .is_some()
    );
    harness.get(by("projects.delete.confirm")).click();
    idle(&mut harness);
    assert_eq!(harness.state().projects.trash().count(), 0);
    assert_eq!(harness.state().projects.entries().len(), 1);
    assert!(
        !fixture.projects.join(".trash-0000000000000002").exists(),
        "delete forever removes the trashed directory"
    );
    assert!(harness.query_by_label("Trash is empty").is_some());
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    assert!(!harness.state().projects.trash_mode());
    assert!(harness.query_by_label("No projects yet").is_none());
}

#[test]
fn trash_listing_failure_reports_and_retry_keeps_the_page_usable() {
    let fixture = Fixture::new(&[("Alpha", missing())]);
    let mut harness = fixture.build(Preferences::default());
    let corrupt = fixture.projects.join(".trash-0000000000000007");
    std::fs::create_dir(&corrupt).unwrap();
    std::fs::write(corrupt.join("project.vibecut"), b"not a project").unwrap();
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    assert!(harness.query_by_label("Couldn't load Trash.").is_some());
    assert!(harness.state().projects.trash().count() == 0);
    std::fs::remove_dir_all(&corrupt).unwrap();
    // Retry in Trash reloads the Trash listing; the active catalog stays usable.
    harness.get(by("projects.retry")).click();
    idle(&mut harness);
    assert!(harness.query_by_label("Trash is empty").is_some());
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    assert_eq!(harness.state().projects.entries().len(), 1);
    assert!(!harness.state().projects.trash_mode());
}

#[test]
fn restore_surfaces_a_trash_listing_failure_and_retry_recovers() {
    let fixture = Fixture::new(&[("Alpha", missing()), ("Beta", missing())]);
    let mut harness = fixture.build(Preferences::default());
    for id in [1u64, 2] {
        let menu = match id {
            1 => "projects.card.0000000000000001.menu",
            _ => "projects.card.0000000000000002.menu",
        };
        let delete = match id {
            1 => "projects.card.0000000000000001.delete",
            _ => "projects.card.0000000000000002.delete",
        };
        harness.get(by(menu)).click();
        harness.run_steps(2);
        harness.get(by(delete)).click();
        harness.run_steps(2);
        harness.get(by("projects.delete.confirm")).click();
        idle(&mut harness);
    }
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    let beta_path = fixture
        .projects
        .join(".trash-0000000000000002/project.vibecut");
    let beta_project = std::fs::read(&beta_path).unwrap();
    std::fs::write(&beta_path, b"not a project").unwrap();

    // Alpha restores, but the refreshed Trash listing fails on Beta. The page
    // must keep the previous rows, report the failure and stop treating the
    // catalog as authoritative until Retry succeeds.
    harness
        .get(by("projects.card.0000000000000001"))
        .click_modifiers(egui::Modifiers::NONE);
    harness.run_steps(2);
    harness.get(by("projects.toolbar.restore")).click();
    idle(&mut harness);
    assert_eq!(harness.state().projects.entries().len(), 1);
    assert!(harness.query_by_label("Couldn't load Trash.").is_some());
    assert!(
        harness.state().projects.settled_ids().is_none(),
        "a failed restore listing is not an authoritative catalog"
    );
    assert_eq!(
        harness.state().projects.trash().count(),
        2,
        "the previous Trash rows stay until a refresh succeeds"
    );

    std::fs::write(&beta_path, &beta_project).unwrap();
    harness.get(by("projects.retry")).click();
    idle(&mut harness);
    assert!(harness.query_by_label("Couldn't load Trash.").is_none());
    assert!(harness.state().projects.settled_ids().is_some());
    assert_eq!(
        harness
            .state()
            .projects
            .trash()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![ProjectId(2)]
    );
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    assert_eq!(harness.state().projects.entries().len(), 1);
}

#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-199-trash.png"]
fn trash_screenshot() {
    let fixture = Fixture::new(&[
        ("Product launch", missing()),
        ("Cooking short", ready_thumbnail([220, 90, 40])),
        ("Monthly recap", ready_thumbnail([40, 120, 200])),
    ]);
    let mut harness = fixture.build(Preferences::default());
    harness
        .get(by("projects.card.0000000000000002.menu"))
        .click();
    harness.run_steps(2);
    harness
        .get(by("projects.card.0000000000000002.delete"))
        .click();
    harness.run_steps(2);
    harness.get(by("projects.delete.confirm")).click();
    idle(&mut harness);
    harness.get(by("projects.trash")).click();
    idle(&mut harness);
    harness
        .get(by("projects.card.0000000000000002"))
        .click_modifiers(egui::Modifiers::NONE);
    harness.run_steps(3);
    let image = harness.render().expect("offscreen render");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-199-trash.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image.save(&path).unwrap();
    eprintln!("trash screenshot: {}", path.display());
    assert!(image.width() > 0 && image.height() > 0);
}

fn ready_thumbnail(color: [u8; 3]) -> Thumbnail {
    let width = 96u32;
    let height = 54u32;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let shade = (40 + (x + y) % 60) as u8;
            rgba.extend_from_slice(&[
                color[0].saturating_add(shade / 3),
                color[1].saturating_add(shade / 3),
                color[2].saturating_add(shade / 3),
                255,
            ]);
        }
    }
    Thumbnail::Ready {
        width,
        height,
        rgba,
    }
}

#[test]
fn workspace_home_raw_text_clipboard_routes_into_the_real_search_field() {
    let f = Fixture::new(&[
        ("Tiếng Việt", Thumbnail::Missing),
        ("English", Thumbnail::Missing),
    ]);
    let mut h = f.build(Preferences::default());
    h.get(by("projects.search")).click();
    h.run_steps(2);
    h.event(egui::Event::Paste("Tiếng Việt".into()));
    h.run_steps(2);
    assert_eq!(
        h.state()
            .projects
            .displayed_ids(h.state().preferences.projects),
        [ProjectId(1)]
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.state_mut().text_clipboard =
        vibecut::services::clipboard::TextClipboard::new(|| Some("English".into()));
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::C,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.step();
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c,egui::OutputCommand::CopyText(t) if t=="Tiếng Việt"))
    );
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::V,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.step();
    assert!(
        !h.output()
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
    );
    h.run_steps(2);
    assert_eq!(
        h.state()
            .projects
            .displayed_ids(h.state().preferences.projects),
        [ProjectId(2)]
    );
}
