#![allow(clippy::unwrap_used, clippy::panic)]
use project::{Metadata, Project, ProjectId, ViewState, WriterLease};
use time::Tick;
#[test]
fn optional_metadata_roundtrips_without_changing_project_bytes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Tiếng Việt.vibecut");
    let mut lease = WriterLease::acquire(&path).unwrap();
    lease
        .save(&Project::empty(ProjectId(1), "Test".into()).unwrap())
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut metadata = lease.metadata();
    metadata.view = ViewState {
        zoom: 2.5,
        scroll: 77.0,
        playhead: Tick(42),
    };
    metadata.scene_views.insert(
        project::SceneId(7),
        ViewState {
            zoom: 3.0,
            ..Default::default()
        },
    );
    lease.save_metadata(&metadata).unwrap();
    drop(lease);
    let lease = WriterLease::acquire(&path).unwrap();
    assert_eq!(lease.metadata(), metadata);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    for value in [b"{".as_slice(), b"{}", &[b' '; 65537]] {
        std::fs::write(root.path().join("Tiếng Việt.vibecut.meta.json"), value).unwrap();
        assert_eq!(lease.metadata().view, ViewState::default());
        assert!(lease.load().is_ok());
    }
    for (field, value) in [
        ("version", serde_json::json!(99)),
        ("created_unix_ms", serde_json::json!(u64::MAX)),
    ] {
        let mut invalid = serde_json::to_value(&metadata).unwrap();
        invalid[field] = value;
        std::fs::write(
            root.path().join("Tiếng Việt.vibecut.meta.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert_eq!(lease.metadata().view, ViewState::default());
    }
}
#[test]
fn metadata_refuses_hostile_values_and_external_project_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    let mut lease = WriterLease::acquire(&path).unwrap();
    lease
        .save(&Project::empty(ProjectId(1), "Test".into()).unwrap())
        .unwrap();
    for view in [
        ViewState {
            zoom: f64::NAN,
            ..Default::default()
        },
        ViewState {
            zoom: 0.0,
            ..Default::default()
        },
        ViewState {
            scroll: f64::INFINITY,
            ..Default::default()
        },
        ViewState {
            playhead: Tick(-1),
            ..Default::default()
        },
    ] {
        assert!(
            lease
                .save_metadata(&Metadata {
                    view,
                    ..Default::default()
                })
                .is_err()
        );
    }
    std::fs::write(&path, b"external replacement").unwrap();
    assert!(matches!(
        lease.save_metadata(&Metadata::default()),
        Err(project::Error::Conflict)
    ));
    assert!(!root.path().join("p.vibecut.meta.json").exists());
}
#[test]
fn metadata_namespace_cannot_be_used_as_a_project_or_escaped_by_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    let mut lease = WriterLease::acquire(&path).unwrap();
    lease
        .save(&Project::empty(ProjectId(1), "Test".into()).unwrap())
        .unwrap();
    let metadata_path = root.path().join("p.vibecut.meta.json");
    assert!(matches!(
        WriterLease::acquire(&metadata_path),
        Err(project::Error::Unsupported(_))
    ));
    #[cfg(unix)]
    {
        let victim = root.path().join("victim");
        std::fs::write(&victim, b"preserve").unwrap();
        std::os::unix::fs::symlink(&victim, &metadata_path).unwrap();
        assert!(lease.save_metadata(&Metadata::default()).is_err());
        assert_eq!(std::fs::read(victim).unwrap(), b"preserve");
    }
}
