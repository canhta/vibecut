#![allow(clippy::unwrap_used, reason = "synthetic bundle fixtures")]
use media::{AssetId, ContentHash};
use project::{Error, Project, ProjectId, bundle};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use time::Tick;

fn write(path: &Path, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    path.to_path_buf()
}
fn asset(id: u64, path: &Path) -> media::SourceAsset {
    testkit::source::asset(id, path)
        .format("mp4")
        .duration(Tick(1000))
        .hash(ContentHash::read(path).unwrap().0)
        .build()
}
fn project(assets: Vec<media::SourceAsset>) -> Project {
    let mut data = Project::empty(ProjectId(7), "Dự án 中".into())
        .unwrap()
        .into_data();
    data.assets = assets;
    Project::new(data).unwrap()
}
/// Four present assets (two share a file name, two share content) and one whose
/// media is deleted before export.
fn media_project(root: &Path) -> Project {
    let a = write(&root.join("dir a/clip.mp4"), b"alpha");
    let b = write(&root.join("dir b/clip.mp4"), b"beta");
    let c = write(&root.join("dir c/clip.mp4"), b"alpha");
    let d = write(&root.join("Thư mục/Dự án 中.mp4"), b"delta");
    let gone = write(&root.join("gone/lost.mp4"), b"lost");
    let project = project(vec![
        asset(1, &a),
        asset(2, &b),
        asset(3, &c),
        asset(4, &d),
        asset(5, &gone),
    ]);
    fs::remove_file(gone).unwrap();
    project
}
fn leftovers(parent: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
fn manifest(bundle: &Path) -> Value {
    serde_json::from_slice(&fs::read(bundle.join("bundle.json")).unwrap()).unwrap()
}
fn set_manifest(bundle: &Path, value: &Value) {
    fs::write(
        bundle.join("bundle.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

#[test]
fn round_trip_copies_media_once_relinks_by_hash_and_reports_missing() {
    let root = tempfile::tempdir().unwrap();
    let original = media_project(root.path());
    let out = root.path().join("out");
    fs::create_dir(&out).unwrap();
    let exported = bundle::export(&original, &out.join("Gói dự án 1.vibecutbundle")).unwrap();
    assert_eq!(exported.copied(), 3);
    assert_eq!(exported.missing(), [AssetId(5)]);
    let media_files = fs::read_dir(exported.path().join("media")).unwrap().count();
    assert_eq!(media_files, 3);
    // Source paths never reach the bundle.
    let text =
        String::from_utf8(fs::read(exported.path().join("project.vibecut")).unwrap()).unwrap();
    assert!(!text.contains("dir a"));

    let imported = bundle::import(exported.path(), &out.join("Đã nhập")).unwrap();
    assert_eq!(imported.missing(), [AssetId(5)]);
    assert_eq!(
        imported.project_path(),
        out.canonicalize().unwrap().join("Đã nhập/project.vibecut")
    );
    let (before, after) = (original.data(), imported.project().data());
    assert_ne!(before.id, after.id);
    assert_eq!(before.name, after.name);
    assert_eq!(before.scenes, after.scenes);
    for (old, new) in before.assets.iter().zip(&after.assets) {
        assert_eq!(old.id(), new.id());
        assert_eq!(old.content_hash(), new.content_hash());
        assert_eq!(old.probe(), new.probe());
        assert!(
            new.path()
                .starts_with(out.canonicalize().unwrap().join("Đã nhập/media"))
        );
    }
    for (index, bytes) in [
        (0, &b"alpha"[..]),
        (1, &b"beta"[..]),
        (2, &b"alpha"[..]),
        (3, &b"delta"[..]),
    ] {
        let path = after.assets.get(index).unwrap().path();
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    let lost = after.assets.get(4).unwrap();
    assert!(!lost.path().exists());
    assert_eq!(
        project::load(imported.project_path()).unwrap().project,
        *imported.project()
    );
    // The missing asset is a relink state: identical content relinks, other content does not.
    let found = write(&root.path().join("found/elsewhere.mp4"), b"lost");
    assert_eq!(lost.relink(found).unwrap().id(), AssetId(5));
    let other = write(&root.path().join("found/other.mp4"), b"other");
    assert!(lost.relink(other).is_err());
}

#[test]
fn export_refuses_an_existing_destination_and_changed_media_without_leftovers() {
    let root = tempfile::tempdir().unwrap();
    let source = write(&root.path().join("m/clip.mp4"), b"one");
    let original = project(vec![asset(1, &source)]);
    let out = root.path().join("out");
    fs::create_dir(&out).unwrap();
    fs::create_dir(out.join("taken")).unwrap();
    fs::write(out.join("taken/keep"), b"x").unwrap();
    assert!(bundle::export(&original, &out.join("taken")).is_err());
    assert_eq!(fs::read(out.join("taken/keep")).unwrap(), b"x");
    fs::write(&source, b"two").unwrap();
    assert!(matches!(
        bundle::export(&original, &out.join("bundle")),
        Err(Error::Invalid(_))
    ));
    assert_eq!(leftovers(&out), ["taken"]);
}

#[test]
fn hostile_or_inconsistent_bundles_are_refused_before_anything_is_published() {
    let root = tempfile::tempdir().unwrap();
    let original = media_project(root.path());
    type Mutation = fn(&mut Value);
    let rows: [(&str, Mutation); 14] = [
        ("zip-slip name", |m| {
            m["media"][0]["name"] = json!("../evil")
        }),
        ("parent name", |m| m["media"][0]["name"] = json!("..")),
        ("separator name", |m| m["media"][0]["name"] = json!("a/b")),
        ("backslash name", |m| m["media"][0]["name"] = json!("a\\b")),
        ("drive name", |m| m["media"][0]["name"] = json!("C:evil")),
        ("duplicate id", |m| m["media"][1]["asset"] = json!(1)),
        ("missing entry", |m| {
            m["media"].as_array_mut().unwrap().pop();
        }),
        ("huge entry", |m| {
            m["media"][0]["bytes"] = json!(bundle::MAX_MEDIA_BYTES + 1)
        }),
        ("wrong size", |m| m["media"][0]["bytes"] = json!(6)),
        ("wrong hash", |m| {
            m["media"][0]["hash"] = json!("0".repeat(64))
        }),
        ("uppercase hash", |m| {
            let hash = m["media"][0]["hash"].as_str().unwrap().to_uppercase();
            m["media"][0]["hash"] = json!(hash);
        }),
        ("unknown field", |m| m["extra"] = json!(1)),
        ("other format", |m| m["format"] = json!("other")),
        ("unknown entry field", |m| {
            m["media"][0]["file"] = json!("../x")
        }),
    ];
    for (label, mutate) in rows {
        let out = tempfile::tempdir().unwrap();
        let made = bundle::export(&original, &out.path().join("b")).unwrap();
        let mut value = manifest(made.path());
        mutate(&mut value);
        set_manifest(made.path(), &value);
        let result = bundle::import(made.path(), &out.path().join("imported"));
        assert!(result.is_err(), "{label} must be refused");
        assert_eq!(leftovers(out.path()), ["b"], "{label} left files behind");
    }
}

#[test]
fn newer_and_older_bundles_and_tampered_media_are_typed_refusals() {
    let root = tempfile::tempdir().unwrap();
    let original = media_project(root.path());
    let fresh = || {
        let out = tempfile::tempdir().unwrap();
        let made = bundle::export(&original, &out.path().join("b")).unwrap();
        (out, made)
    };
    for (version, newer) in [
        (project::SCHEMA_VERSION + 1, true),
        (project::SCHEMA_VERSION - 1, false),
    ] {
        let (out, made) = fresh();
        let mut value = manifest(made.path());
        value["schema_version"] = json!(version);
        set_manifest(made.path(), &value);
        let result = bundle::import(made.path(), &out.path().join("imported"));
        assert!(
            matches!(
                (newer, result),
                (true, Err(Error::TooNew { .. })) | (false, Err(Error::TooOld { .. }))
            ),
            "schema {version}"
        );
    }
    // The envelope's own version is checked as well.
    let (out, made) = fresh();
    let text = fs::read_to_string(made.path().join("project.vibecut")).unwrap();
    let newer = text.replacen(
        &format!("\"schema_version\": {}", project::SCHEMA_VERSION),
        &format!("\"schema_version\": {}", project::SCHEMA_VERSION + 1),
        1,
    );
    assert_ne!(text, newer);
    fs::write(made.path().join("project.vibecut"), newer).unwrap();
    assert!(matches!(
        bundle::import(made.path(), &out.path().join("imported")),
        Err(Error::TooNew { .. })
    ));
    // Same length, different bytes: the copy's hash no longer matches.
    let (out, made) = fresh();
    let stored = fs::read_dir(made.path().join("media"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("-Dự án 中.mp4"))
        .unwrap();
    fs::write(&stored, b"DELTA").unwrap();
    assert!(matches!(
        bundle::import(made.path(), &out.path().join("imported")),
        Err(Error::Invalid(_))
    ));
    // An existing destination is never replaced.
    let (out, made) = fresh();
    fs::create_dir(out.path().join("imported")).unwrap();
    assert!(bundle::import(made.path(), &out.path().join("imported")).is_err());
    assert_eq!(leftovers(out.path()), ["b", "imported"]);
}

#[test]
fn media_deleted_from_a_bundle_imports_as_missing_and_extra_files_are_ignored() {
    let root = tempfile::tempdir().unwrap();
    let original = media_project(root.path());
    let out = tempfile::tempdir().unwrap();
    let made = bundle::export(&original, &out.path().join("b")).unwrap();
    for entry in fs::read_dir(made.path().join("media")).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    fs::write(made.path().join("media/stray.bin"), b"stray").unwrap();
    let imported = bundle::import(made.path(), &out.path().join("imported")).unwrap();
    assert_eq!(imported.missing().len(), 5);
    assert_eq!(
        fs::read_dir(out.path().join("imported/media"))
            .unwrap()
            .count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn links_inside_a_bundle_are_refused() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let original = media_project(root.path());
    let outside = write(&root.path().join("outside/secret.txt"), b"alpha");
    let fresh = || {
        let out = tempfile::tempdir().unwrap();
        let made = bundle::export(&original, &out.path().join("b")).unwrap();
        (out, made)
    };
    let (out, made) = fresh();
    let stored = fs::read_dir(made.path().join("media"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(&stored).unwrap();
    symlink(&outside, &stored).unwrap();
    assert!(bundle::import(made.path(), &out.path().join("imported")).is_err());
    assert_eq!(leftovers(out.path()), ["b"]);

    let (out, made) = fresh();
    fs::rename(made.path().join("media"), out.path().join("real-media")).unwrap();
    symlink(out.path().join("real-media"), made.path().join("media")).unwrap();
    assert!(bundle::import(made.path(), &out.path().join("imported")).is_err());

    let (out, made) = fresh();
    let alias = out.path().join("alias");
    symlink(made.path(), &alias).unwrap();
    assert!(bundle::import(&alias, &out.path().join("imported")).is_err());

    let (out, made) = fresh();
    fs::remove_file(made.path().join("project.vibecut")).unwrap();
    symlink(&outside, made.path().join("project.vibecut")).unwrap();
    assert!(bundle::import(made.path(), &out.path().join("imported")).is_err());
}
