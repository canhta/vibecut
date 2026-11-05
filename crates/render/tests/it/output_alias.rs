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

/// Destination admission: definite path aliases keep `output aliases a source`
/// whatever other sources hold, and a distinct destination is admitted.
#[test]
fn destination_admission_by_path_identity() {
    let same = b"identical source and destination bytes";
    // (destination relative to the directory, sources: (relative path, exists), expected)
    type Case = (
        &'static str,
        &'static [(&'static str, bool)],
        Option<&'static str>,
    );
    let cases: &[Case] = &[
        // An identical sibling is ordered first, as in the clip-export regression:
        // content similarity must not pre-empt the definite alias.
        (
            "output.mp4",
            &[("source.mp4", true), ("output.mp4", true)],
            Some("output aliases a source"),
        ),
        (
            "./source.mp4",
            &[("source.mp4", true)],
            Some("output aliases a source"),
        ),
        (
            "missing.mp4",
            &[("missing.mp4", false)],
            Some("output aliases a source"),
        ),
        ("new-output.mp4", &[("source.mp4", true)], None),
    ];
    for (destination, sources, expected) in cases {
        let directory = tempfile::tempdir().unwrap();
        let mut assets = Vec::new();
        for (index, (name, exists)) in sources.iter().enumerate() {
            let path = directory.path().join(name);
            if *exists {
                std::fs::write(&path, same).unwrap();
                assets.push(asset(index as u64 + 1, &path, same));
            } else {
                assets.push(
                    media::SourceAsset::try_from(media::AssetData {
                        ephemeral: false,
                        id: media::AssetId(index as u64 + 1),
                        path,
                        content_hash: media::ContentHash([0; 32]),
                        probe: media::SourceProbe {
                            format: "fixture".into(),
                            duration: None,
                            streams: Vec::new(),
                        },
                    })
                    .unwrap(),
                );
            }
        }
        let destination = directory.path().join(destination);
        let result = validate_output_path(&project(assets), &destination, &Hooks::default());
        match (expected, result) {
            (Some(message), Err(Error::Invalid(got))) => {
                assert_eq!(*message, got, "{destination:?}")
            }
            (None, Ok(())) => {}
            (_, other) => panic!("{destination:?}: {other:?}"),
        }
        for (name, exists) in *sources {
            let path = directory.path().join(name);
            assert_eq!(path.exists(), *exists, "{name} existence changed");
            if *exists {
                assert_eq!(std::fs::read(&path).unwrap(), same);
            }
        }
        if expected.is_none() {
            assert!(!destination.exists());
        }
    }
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
