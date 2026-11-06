use super::process::{cli, os};
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
fn sound_json(id: u64, audio_url: &str) -> String {
    json!({
        "id": id,
        "name": "Whoosh",
        "author": "Freesound user",
        "licence": "Creative Commons 0",
        "attribution": "Whoosh by Freesound user (Creative Commons 0)",
        "source_url": format!("https://freesound.org/s/{id}/"),
        "audio_url": audio_url,
        "duration_ms": 100,
        "downloads": 10,
    })
    .to_string()
}

#[test]
fn sounds_cli_saves_searches_and_adds_a_cached_sound_offline() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("sounds.vibecut");
    let output = cli(&[os("project"), os("new"), project.as_os_str()]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let library = dir.path().join("library");
    let json = sound_json(5, "https://cdn.example/5.mp3");
    let output = cli(&[os("sounds"), os("save"), library.as_os_str(), os(&json)]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let library_handle = engine::sounds::SoundLibrary::open(&library).unwrap();
    library_handle
        .store_cached("https://cdn.example/5.mp3", &wav_fixture())
        .unwrap();
    drop(library_handle);

    let output = cli(&[
        os("sounds"),
        os("search"),
        os("--library"),
        library.as_os_str(),
        os("Whoosh"),
        os("--json"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["source"], "local");
    assert_eq!(value["results"][0]["author"], "Freesound user");
    assert_eq!(value["results"][0]["licence"], "Creative Commons 0");
    assert_eq!(
        value["results"][0]["attribution"],
        "Whoosh by Freesound user (Creative Commons 0)"
    );

    let output = cli(&[
        os("sounds"),
        os("add"),
        project.as_os_str(),
        library.as_os_str(),
        os("5"),
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
    let project::AudioSource::Library { sound } = &params.source else {
        panic!("library source");
    };
    assert_eq!(sound.id, 5);
    assert_eq!(sound.licence.as_str(), "Creative Commons 0");
    let media = data.assets[0].path().to_owned();
    assert!(media.starts_with(dir.path().join("sounds.vibecut.media")));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("secret"));
    assert!(!stderr.contains("secret"));

    let output = cli(&[
        os("sounds"),
        os("unsave"),
        library.as_os_str(),
        os("5"),
        os("--json"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["removed"], true);
    assert!(media.is_file(), "unsave must not remove project media");
}
#[test]
fn sounds_cli_refuses_missing_cache_and_invalid_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("sounds.vibecut");
    assert_eq!(
        cli(&[os("project"), os("new"), project.as_os_str()])
            .status
            .code(),
        Some(0)
    );
    let library = dir.path().join("library");
    let json = sound_json(6, "https://cdn.example/6.mp3");
    assert_eq!(
        cli(&[
            "sounds".as_ref(),
            "save".as_ref(),
            library.as_os_str(),
            os(&json)
        ])
        .status
        .code(),
        Some(0)
    );
    let output = cli(&[
        os("sounds"),
        os("add"),
        project.as_os_str(),
        library.as_os_str(),
        os("6"),
    ]);
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not cached"));
    assert_eq!(
        cli(&[os("sounds"), os("search"), os("query")])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        cli(&[
            os("sounds"),
            os("save"),
            library.as_os_str(),
            os("{not json"),
        ])
        .status
        .code(),
        Some(2)
    );
}
