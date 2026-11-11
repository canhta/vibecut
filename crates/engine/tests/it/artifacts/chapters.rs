#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use ai::chapters::{Chapter, ChapterDocument};
use captions::{LanguageTag, NfcText};
use engine::{Error as EngineError, chapter_sidecars};
use project::{Project, ProjectId};
use std::path::PathBuf;
use time::{TICKS_PER_SECOND, Tick};

struct Hooks;
struct Lease;
impl render::Reservation for Lease {}
impl render::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(Lease))
    }
}

fn project() -> Project {
    Project::empty(ProjectId(1), "Mục lục".into()).unwrap()
}

fn with_source(project: Project, path: PathBuf) -> Project {
    let mut data = project.into_data();
    data.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(7),
            path,
            content_hash: media::ContentHash::from_bytes(b"source"),
            probe: media::SourceProbe {
                format: "mp4".into(),
                duration: Some(Tick(TICKS_PER_SECOND)),
                streams: Vec::new(),
            },
        })
        .unwrap(),
    );
    Project::new(data).unwrap()
}

fn document() -> ChapterDocument {
    ChapterDocument::new(
        LanguageTag::parse("vi").unwrap(),
        Tick(TICKS_PER_SECOND * 60),
        "Hành trình học tập".into(),
        "Một cuộc trò chuyện về thói quen.".into(),
        vec![
            Chapter {
                start: Tick::ZERO,
                segment_id: NfcText::from("s0"),
                title: "Mở đầu".into(),
            },
            Chapter {
                start: Tick(TICKS_PER_SECOND * 10),
                segment_id: NfcText::from("s2"),
                title: "Thói quen".into(),
            },
        ],
        vec![ai::chapters::ChapterTranslation {
            language: LanguageTag::parse("en").unwrap(),
            title: "A learning journey".into(),
            description: "A conversation about habits.".into(),
            chapters: vec!["Opening".into(), "Habits".into()],
        }],
    )
    .unwrap()
}

#[test]
fn unicode_sidecars_round_trip_and_leave_project_json_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let project = project();
    let before = serde_json::to_vec(project.data()).unwrap();
    let stem = dir.path().join("Mục lục tiếng Việt");
    let paths = chapter_sidecars::write(
        &project,
        &[],
        &Hooks,
        &stem,
        &document(),
        project::atomic::Overwrite::Refuse,
    )
    .unwrap();
    assert_eq!(paths.text, dir.path().join("Mục lục tiếng Việt.txt"));
    assert_eq!(paths.json, dir.path().join("Mục lục tiếng Việt.json"));
    let text = std::fs::read_to_string(&paths.text).unwrap();
    assert!(text.starts_with("Hành trình học tập"));
    assert!(text.contains("00:00:10 Thói quen"));
    assert!(text.contains("[en]"));
    let json = std::fs::read(&paths.json).unwrap();
    assert_eq!(ChapterDocument::from_json(&json).unwrap(), document());
    assert_eq!(serde_json::to_vec(project.data()).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn project_metadata_and_source_aliases_publish_neither_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Nguồn tiếng Việt.txt");
    std::fs::write(&source, b"source bytes").unwrap();
    let with_asset = with_source(project(), source.clone());
    let stem = dir.path().join("Nguồn tiếng Việt");
    let error = chapter_sidecars::write(
        &with_asset,
        &[],
        &Hooks,
        &stem,
        &document(),
        project::atomic::Overwrite::Refuse,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        EngineError::Runtime(engine::RuntimeError::Render(render::Error::Invalid(_)))
            | EngineError::Runtime(engine::RuntimeError::Edit(engine::EditError::Invalid(_)))
    ));
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
    assert!(!dir.path().join("Nguồn tiếng Việt.json").exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    let protected = dir.path().join("project.json");
    let error = chapter_sidecars::write(
        &project(),
        &[protected],
        &Hooks,
        &dir.path().join("project"),
        &document(),
        project::atomic::Overwrite::Refuse,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        EngineError::Runtime(engine::RuntimeError::Edit(engine::EditError::Invalid(_)))
    ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn directory_export_derives_the_project_name_and_refuses_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let project = project();
    let before = serde_json::to_vec(project.data()).unwrap();
    let paths =
        chapter_sidecars::write_to_directory(&project, &[], dir.path(), &document()).unwrap();
    assert_eq!(paths.text, dir.path().join("Mục lục.txt"));
    assert_eq!(paths.json, dir.path().join("Mục lục.json"));
    assert_eq!(
        ChapterDocument::from_json(&std::fs::read(&paths.json).unwrap()).unwrap(),
        document()
    );
    assert_eq!(serde_json::to_vec(project.data()).unwrap(), before);

    let again = chapter_sidecars::write_to_directory(&project, &[], dir.path(), &document());
    assert!(matches!(
        again,
        Err(EngineError::Edit(engine::EditError::Invalid(_)))
    ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);

    std::fs::remove_file(&paths.text).unwrap();
    std::fs::remove_file(&paths.json).unwrap();
    let existing = dir.path().join("Mục lục.json");
    std::fs::write(&existing, b"previous json").unwrap();
    let refused = chapter_sidecars::write_to_directory(&project, &[], dir.path(), &document());
    assert!(matches!(
        refused,
        Err(EngineError::Edit(engine::EditError::Invalid(_)))
    ));
    assert!(!dir.path().join("Mục lục.txt").exists());
    assert_eq!(std::fs::read(&existing).unwrap(), b"previous json");

    let missing = dir.path().join("missing");
    assert!(matches!(
        chapter_sidecars::write_to_directory(&project, &[], &missing, &document()),
        Err(EngineError::Edit(engine::EditError::Invalid(_)))
    ));
}

#[test]
fn existing_sidecars_are_preserved_without_overwrite_authorization() {
    let dir = tempfile::tempdir().unwrap();
    let stem = dir.path().join("Mục lục");
    let paths = chapter_sidecars::sidecar_paths(&stem).unwrap();
    std::fs::write(&paths.text, b"previous text").unwrap();
    std::fs::write(&paths.json, b"previous json").unwrap();
    let error = chapter_sidecars::write(
        &project(),
        &[],
        &Hooks,
        &stem,
        &document(),
        project::atomic::Overwrite::Refuse,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        EngineError::Runtime(engine::RuntimeError::Edit(engine::EditError::Project(
            project::Error::Io(_)
        )))
    ));
    assert_eq!(std::fs::read(&paths.text).unwrap(), b"previous text");
    assert_eq!(std::fs::read(&paths.json).unwrap(), b"previous json");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
