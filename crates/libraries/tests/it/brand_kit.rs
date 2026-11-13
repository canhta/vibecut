//! Brand kit store: kit lifecycle, ordered entries, verified media, limits and folder archives.
#![allow(clippy::unwrap_used, clippy::panic)]
use engine::BrandKitError;
use engine::control::BrandEntryState;
use libraries::brand_kit::{BrandKit, MAX_ENTRIES, MAX_KITS, MAX_NAME_BYTES};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn store(root: &Path) -> BrandKit {
    BrandKit::open(root).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn source_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn kits_are_created_renamed_selected_deleted_and_bounded_by_names_and_count() {
    let dir = tempfile::tempdir().unwrap();
    let kit = store(dir.path());
    let first = kit.create("Studio").unwrap();
    let second = kit.create("Client").unwrap();
    assert!(matches!(
        kit.create("  ").unwrap_err(),
        BrandKitError::Invalid(_)
    ));
    assert!(matches!(
        kit.create("a/b").unwrap_err(),
        BrandKitError::Invalid(_)
    ));
    assert!(matches!(
        kit.create(&"x".repeat(MAX_NAME_BYTES + 1)).unwrap_err(),
        BrandKitError::Invalid(_)
    ));
    assert!(matches!(
        kit.create("Studio").unwrap_err(),
        BrandKitError::Invalid(_)
    ));

    kit.set_active(Some(first)).unwrap();
    let view = kit.active_view().unwrap().unwrap();
    assert_eq!((view.id, view.name.as_str()), (first, "Studio"));
    kit.rename(second, "Client A").unwrap();
    assert!(
        kit.kits()
            .unwrap()
            .iter()
            .any(|item| item.name == "Client A")
    );
    kit.delete(second).unwrap();
    assert!(matches!(
        kit.rename(second, "Gone").unwrap_err(),
        BrandKitError::NotFound
    ));

    for index in 0..MAX_KITS - 1 {
        kit.create(&format!("kit {index}")).unwrap();
    }
    assert!(matches!(
        kit.create("overflow").unwrap_err(),
        BrandKitError::Limit
    ));

    kit.delete(first).unwrap();
    assert!(kit.active_view().unwrap().is_none());
}

#[test]
fn colours_and_fonts_keep_insertion_order_and_refuse_duplicates_and_absent_names() {
    let dir = tempfile::tempdir().unwrap();
    let kit = store(dir.path());
    let id = kit.create("Brand").unwrap();
    kit.add_color(id, "Lime", [0xB5, 0xFF, 0x3C, 255]).unwrap();
    kit.add_color(id, "Pink", [0xFF, 0x2D, 0x87, 255]).unwrap();
    assert!(matches!(
        kit.add_color(id, "Lime", [0; 4]).unwrap_err(),
        BrandKitError::Invalid(_)
    ));
    kit.add_font(id, "Inter").unwrap();
    kit.set_active(Some(id)).unwrap();

    let view = kit.active_view().unwrap().unwrap();
    let names: Vec<&str> = view.colors.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Lime", "Pink"]);
    assert_eq!(view.fonts, ["Inter"]);

    kit.remove_color(id, "Lime").unwrap();
    assert!(matches!(
        kit.remove_color(id, "Lime").unwrap_err(),
        BrandKitError::NotFound
    ));
    kit.remove_font(id, "Inter").unwrap();
    assert!(kit.active_view().unwrap().unwrap().fonts.is_empty());

    for index in 0..MAX_ENTRIES {
        kit.add_font(id, &format!("Family {index}")).unwrap();
    }
    assert!(matches!(
        kit.add_font(id, "overflow").unwrap_err(),
        BrandKitError::Limit
    ));
}

#[test]
fn media_is_copied_and_verified_and_a_missing_or_corrupt_file_stays_listed() {
    let dir = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();
    let logo = source_file(sources.path(), "logo.svg", b"<svg/>");
    let kit = store(dir.path());
    let id = kit.create("Brand").unwrap();
    kit.set_active(Some(id)).unwrap();
    let media_id = kit.add_media(id, &logo, "logo.svg").unwrap();

    let view = kit.active_view().unwrap().unwrap();
    assert_eq!(view.media.len(), 1);
    assert_eq!(view.media[0].id, media_id);
    assert_eq!(view.media[0].size, 6);
    assert_eq!(view.media[0].sha256, sha256_hex(b"<svg/>"));
    assert_eq!(view.media[0].state, BrandEntryState::Available);

    let stored = dir.path().join("media").join(&view.media[0].sha256);
    fs::write(&stored, b"tampered-bytes").unwrap();
    let corrupt = kit.active_view().unwrap().unwrap();
    assert_eq!(corrupt.media[0].state, BrandEntryState::Corrupt);

    fs::remove_file(&stored).unwrap();
    let missing = kit.active_view().unwrap().unwrap();
    assert_eq!(missing.media.len(), 1);
    assert_eq!(missing.media[0].state, BrandEntryState::Missing);

    assert!(matches!(
        kit.add_media(id, sources.path(), "folder").unwrap_err(),
        BrandKitError::Invalid(_)
    ));
    kit.remove_media(id, media_id).unwrap();
    assert!(kit.active_view().unwrap().unwrap().media.is_empty());
}

#[test]
fn the_active_selection_survives_reopening_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let kit = store(dir.path());
        let id = kit.create("Brand").unwrap();
        kit.add_color(id, "Pink", [255, 45, 135, 255]).unwrap();
        kit.set_active(Some(id)).unwrap();
        id
    };
    let reopened = store(dir.path());
    let view = reopened.active_view().unwrap().unwrap();
    assert_eq!(view.id, id);
    assert_eq!(view.colors[0].name, "Pink");
}

#[test]
fn archive_round_trips_byte_equal_and_refuses_unverified_or_escaping_records() {
    let dir = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();
    let kit = store(dir.path());
    let id = kit.create("Brand").unwrap();
    kit.add_color(id, "Lime", [1, 2, 3, 4]).unwrap();
    let logo = source_file(sources.path(), "mark.svg", b"<svg>mark</svg>");
    kit.add_media(id, &logo, "mark.svg").unwrap();

    let out = dir.path().join("archive");
    kit.export_kit(id, &out).unwrap();
    assert!(matches!(
        kit.export_kit(id, &out).unwrap_err(),
        BrandKitError::Invalid(_)
    ));

    let other_dir = tempfile::tempdir().unwrap();
    let other = store(other_dir.path());
    let copied = other.import_kit(&out).unwrap();
    let again = other_dir.path().join("again");
    other.export_kit(copied, &again).unwrap();
    assert_eq!(
        fs::read(out.join("kit.json")).unwrap(),
        fs::read(again.join("kit.json")).unwrap()
    );
    for entry in fs::read_dir(out.join("media")).unwrap() {
        let name = entry.unwrap().file_name();
        assert_eq!(
            fs::read(out.join("media").join(&name)).unwrap(),
            fs::read(again.join("media").join(&name)).unwrap()
        );
    }

    let media_name = fs::read_dir(out.join("media"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    fs::write(out.join("media").join(&media_name), b"altered").unwrap();
    let third_dir = tempfile::tempdir().unwrap();
    let third = store(third_dir.path());
    assert!(matches!(
        third.import_kit(&out).unwrap_err(),
        BrandKitError::Integrity
    ));
    assert!(third.kits().unwrap().is_empty());

    let escaping = dir.path().join("escaping");
    kit.export_kit(id, &escaping).unwrap();
    let record = escaping.join("kit.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    value["kit"]["media"][0]["sha256"] = serde_json::json!("../../outside");
    fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        third.import_kit(&escaping).unwrap_err(),
        BrandKitError::Invalid(_)
    ));
}
