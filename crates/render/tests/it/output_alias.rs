#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::common::*;
use render::{Error, validate_output_path};
use std::path::Path;

fn asset(id: u64, path: &Path, bytes: &[u8]) -> media::SourceAsset {
    media::SourceAsset::try_from(media::AssetData {
        ephemeral: false,
        id: media::AssetId(id),
        path: std::fs::canonicalize(path).unwrap(),
        content_hash: media::ContentHash::from_bytes(bytes),
        probe: media::SourceProbe {
            format: "fixture".into(),
            duration: None,
            streams: Vec::new(),
        },
    })
    .unwrap()
}

fn project(assets: Vec<media::SourceAsset>) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(0), "alias".into())
        .unwrap()
        .into_data();
    data.assets = assets;
    project::Project::new(data).unwrap()
}

#[test]
fn exact_path_alias_wins_over_an_identical_sibling_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    let destination = directory.path().join("output.mp4");
    std::fs::write(&source, b"identical source and destination bytes").unwrap();
    std::fs::write(&destination, b"identical source and destination bytes").unwrap();
    // The identical sibling is ordered first, as it is in the clip-export
    // regression: content similarity must not pre-empt the definite alias.
    let project = project(vec![
        asset(1, &source, b"identical source and destination bytes"),
        asset(2, &destination, b"identical source and destination bytes"),
    ]);
    let error = validate_output_path(&project, &destination, &Hooks::default()).unwrap_err();
    assert!(
        matches!(error, Error::Invalid("output aliases a source")),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(&source).unwrap(),
        b"identical source and destination bytes"
    );
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"identical source and destination bytes"
    );
}

#[cfg(not(unix))]
#[test]
fn content_equal_distinct_destination_is_still_refused() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    let destination = directory.path().join("distinct.mp4");
    std::fs::write(&source, b"same content in a distinct file").unwrap();
    std::fs::write(&destination, b"same content in a distinct file").unwrap();
    let project = project(vec![asset(1, &source, b"same content in a distinct file")]);
    let error = validate_output_path(&project, &destination, &Hooks::default()).unwrap_err();
    assert!(
        matches!(error, Error::Invalid("output may alias source content")),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(&source).unwrap(),
        b"same content in a distinct file"
    );
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"same content in a distinct file"
    );
}

#[test]
fn dot_component_alias_is_refused_after_normalization() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    std::fs::write(&source, b"source bytes").unwrap();
    let destination = directory.path().join(".").join("source.mp4");
    let project = project(vec![asset(1, &source, b"source bytes")]);
    let error = validate_output_path(&project, &destination, &Hooks::default()).unwrap_err();
    assert!(
        matches!(error, Error::Invalid("output aliases a source")),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
}

#[test]
fn missing_source_path_still_refuses_its_own_destination() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.mp4");
    let project = project(vec![
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(1),
            path: path.clone(),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: Vec::new(),
            },
        })
        .unwrap(),
    ]);
    let error = validate_output_path(&project, &path, &Hooks::default()).unwrap_err();
    assert!(
        matches!(error, Error::Invalid("output aliases a source")),
        "{error:?}"
    );
    assert!(!path.exists());
}

#[test]
fn missing_destination_that_is_not_a_source_is_admitted() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    std::fs::write(&source, b"source bytes").unwrap();
    let destination = directory.path().join("new-output.mp4");
    let project = project(vec![asset(1, &source, b"source bytes")]);
    validate_output_path(&project, &destination, &Hooks::default()).unwrap();
    assert!(!destination.exists());
}

#[test]
fn cancellation_prevents_alias_checks_and_publication() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    std::fs::write(&source, b"source bytes").unwrap();
    let project = project(vec![asset(1, &source, b"source bytes")]);
    let hooks = Hooks::default();
    hooks.state.store(1, std::sync::atomic::Ordering::SeqCst);
    let error = validate_output_path(&project, &source, &hooks).unwrap_err();
    assert!(matches!(error, Error::Cancelled), "{error:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
}
