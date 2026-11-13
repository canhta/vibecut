//! Verifies native music command behaviour.
use crate::cli::process::{cli, os};
use serde_json::{Value, json};

fn wav_fixture() -> Vec<u8> {
    let samples = 800u32;
    let data_len = samples * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8000u32.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.extend(std::iter::repeat_n(0u8, data_len as usize));
    bytes
}
fn track_json(id: &str, audio_url: &str) -> String {
    json!({
        "id": id,
        "title": "Night Walk",
        "author": "Example Artist",
        "licence": "CC BY 4.0",
        "licence_url": "https://creativecommons.org/licenses/by/4.0/",
        "attribution": "\"Night Walk\" by Example Artist is licensed under CC BY 4.0.",
        "source_url": format!("https://source.example/{id}"),
        "audio_url": audio_url,
        "duration_ms": 100,
    })
    .to_string()
}

#[test]
fn music_cli_saves_searches_and_adds_a_cached_track_offline() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("music.vibecut");
    let output = cli(&[os("project"), os("new"), project.as_os_str()]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let library = dir.path().join("library");
    let json = track_json("track-5", "https://cdn.example/5.mp3");
    let output = cli(&[
        os("music"),
        os("save"),
        library.as_os_str(),
        os(&json),
        os("--json"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["licence"], "CC BY 4.0");
    assert_eq!(
        json["licence_url"],
        "https://creativecommons.org/licenses/by/4.0/"
    );
    let library_handle = libraries::music::MusicLibrary::open(&library).unwrap();
    library_handle
        .store_cached("https://cdn.example/5.mp3", &wav_fixture())
        .unwrap();
    drop(library_handle);

    let output = cli(&[
        os("music"),
        os("search"),
        os("--library"),
        library.as_os_str(),
        os("--saved"),
        os("Night"),
        os("--json"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["source"], "local");
    assert_eq!(value["results"][0]["author"], "Example Artist");
    assert_eq!(value["results"][0]["licence"], "CC BY 4.0");
    assert_eq!(
        value["results"][0]["licence_url"],
        "https://creativecommons.org/licenses/by/4.0/"
    );
    assert_eq!(
        value["results"][0]["attribution"],
        "\"Night Walk\" by Example Artist is licensed under CC BY 4.0."
    );

    let output = cli(&[
        os("music"),
        os("add"),
        project.as_os_str(),
        library.as_os_str(),
        os("track-5"),
        os("--start"),
        os("0"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let loaded = project::load(&project).unwrap().project;
    let data = loaded.data();
    assert_eq!(data.assets.len(), 1);
    let item = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| matches!(item.content, project::ItemParams::Audio(_)))
        .unwrap();
    let project::ItemParams::Audio(params) = &item.content else {
        panic!("audio item");
    };
    let project::AudioSource::Music { music } = &params.source else {
        panic!("music source");
    };
    assert_eq!(music.id.as_str(), "track-5");
    assert_eq!(music.licence.as_str(), "CC BY 4.0");
    assert_eq!(
        music.licence_url,
        "https://creativecommons.org/licenses/by/4.0/"
    );
    let media = data.assets[0].path().to_owned();
    // The saved project names its media by the resolved path of the project folder.
    let folder = dir
        .path()
        .canonicalize()
        .unwrap()
        .join("music.vibecut.media");
    assert!(
        media.starts_with(&folder),
        "{} lies outside the project's media folder",
        media.display()
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("token"),
        "no credential is printed"
    );

    let output = cli(&[
        os("music"),
        os("unsave"),
        library.as_os_str(),
        os("track-5"),
        os("--json"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["removed"], true);
    assert!(media.is_file(), "unsave must not remove project media");
}
#[test]
fn music_cli_refuses_missing_cache_and_invalid_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("music.vibecut");
    assert_eq!(
        cli(&[os("project"), os("new"), project.as_os_str()])
            .status
            .code(),
        Some(0)
    );
    let library = dir.path().join("library");
    let json = track_json("track-6", "https://cdn.example/6.mp3");
    assert_eq!(
        cli(&[
            "music".as_ref(),
            "save".as_ref(),
            library.as_os_str(),
            os(&json)
        ])
        .status
        .code(),
        Some(0)
    );
    let output = cli(&[
        os("music"),
        os("add"),
        project.as_os_str(),
        library.as_os_str(),
        os("track-6"),
    ]);
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not cached"));
    assert_eq!(
        cli(&[os("music"), os("search"), os("query")]).status.code(),
        Some(2)
    );
    assert_eq!(
        cli(&[
            os("music"),
            os("save"),
            library.as_os_str(),
            os("{not json"),
        ])
        .status
        .code(),
        Some(2)
    );
}
#[test]
fn music_cli_add_keeps_absolute_media_for_a_relative_project_path() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("rel.vibecut");
    assert_eq!(
        cli(&[os("project"), os("new"), project.as_os_str()])
            .status
            .code(),
        Some(0)
    );
    let library = dir.path().join("library");
    let json = track_json("track-7", "https://cdn.example/7.mp3");
    assert_eq!(
        cli(&[os("music"), os("save"), library.as_os_str(), os(&json)])
            .status
            .code(),
        Some(0)
    );
    let handle = libraries::music::MusicLibrary::open(&library).unwrap();
    handle
        .store_cached("https://cdn.example/7.mp3", &wav_fixture())
        .unwrap();
    drop(handle);
    let output = std::process::Command::new(crate::cli::process::bin())
        .current_dir(dir.path())
        .args([
            os("music"),
            os("add"),
            os("rel.vibecut"),
            library.as_os_str(),
            os("track-7"),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let loaded = project::load(&project).unwrap().project;
    let media = loaded.data().assets[0].path().to_owned();
    assert!(media.is_absolute(), "{media:?}");
    assert!(media.is_file(), "{media:?}");
    assert_eq!(
        media.parent().unwrap().file_name().unwrap(),
        "rel.vibecut.media"
    );
}
