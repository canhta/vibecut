//! Verifies transcript listing and word-range deletion.
use crate::cli::fixtures::{exec, new_project};
use crate::cli::process::{cli, code, error_json, os, stdout_json};
use serde_json::json;
use std::fs;
use std::path::Path;
use time::Tick;

/// One caption item over a 1200-tick source with four measured Vietnamese words.
/// Returns the allocated item ID.
fn word_project(path: &Path, edited: bool) -> i64 {
    new_project(path);
    let asset =
        testkit::source::asset(7, std::env::temp_dir().join("vibecut-transcript-words.mp4"))
            .format("mp4")
            .duration(Tick(1200))
            .content_hash(media::ContentHash::from_bytes(b"words"))
            .build();
    exec(path, "import.add_asset", json!({"asset": asset}));
    exec(
        path,
        "timeline.add_track",
        json!({"name":"Captions","kind":"text"}),
    );
    let inserted = exec(
        path,
        "timeline.insert",
        json!({
            "placement": {"mode": "auto"},
            "item": {
                "animations": {},
                "source_asset": 7,
                "source_range": {"start": 0, "end": 1200},
                "start": 0,
                "content": {"kind": "caption", "params": {"word_highlight": false, "cues": {"cues": [{
                    "id": "câu-1",
                    "range": {"start": 0, "end": 1200},
                    "source_lang": "vi",
                    "texts": {"vi": {
                        "text": "Một hai ba bốn",
                        "edited": edited,
                        "translation_ref": null
                    }},
                    "word_timing": {"kind": "unverified"},
                    "word_ranges": [
                        {"start": 50, "end": 150},
                        {"start": 150, "end": 250},
                        {"start": 550, "end": 650},
                        {"start": 800, "end": 900}
                    ]
                }]}}}
            }
        }),
    );
    inserted["placements"][0]["item"].as_i64().unwrap()
}

#[test]
fn transcript_lists_words_and_deletes_a_range_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("words.vibecut");
    let item = word_project(&path, false);
    let before = fs::read(&path).unwrap();

    let listed = cli(&[os("transcript"), path.as_os_str(), os("--json")]);
    assert_eq!(
        code(&listed),
        0,
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let value = stdout_json(&listed);
    assert_eq!(fs::read(&path).unwrap(), before, "listing writes nothing");
    let cues = value["transcript"]["cues"].as_array().unwrap();
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0]["item"], item);
    assert_eq!(cues[0]["id"], "câu-1");
    assert_eq!(cues[0]["output"], json!({"start":0,"end":1200}));
    let words = cues[0]["words"].as_array().unwrap();
    assert_eq!(words.len(), 4);
    assert_eq!(words[0]["text"], "Một");
    assert_eq!(words[0]["output"], json!({"start":50,"end":150}));
    assert_eq!(words[0]["protected"], false);

    let deleted = cli(&[
        os("transcript"),
        path.as_os_str(),
        os("--item"),
        os(&item.to_string()),
        os("--cue"),
        os("câu-1"),
        os("--words"),
        os("1-2"),
        os("--json"),
    ]);
    assert_eq!(
        code(&deleted),
        0,
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    let value = stdout_json(&deleted);
    assert_eq!(value["deleted"], 2);
    assert_eq!(value["changed"], true);
    assert_eq!(value["remaining_words"], 2);
    assert_ne!(fs::read(&path).unwrap(), before);

    let listed = cli(&[os("transcript"), path.as_os_str(), os("--json")]);
    assert_eq!(code(&listed), 0);
    let value = stdout_json(&listed);
    let mut words: Vec<serde_json::Value> = value["transcript"]["cues"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|cue| cue["words"].as_array().unwrap().clone())
        .collect();
    words.sort_by_key(|word| word["id"]["word"].as_u64().unwrap());
    assert_eq!(words.len(), 2);
    assert_eq!(words[0]["id"]["word"], 0);
    assert_eq!(words[1]["id"]["word"], 3);
    assert_eq!(words[1]["output"]["start"], 600);
    let loaded = project::load(&path).unwrap().project;
    assert_eq!(loaded.duration(), Tick(1000));
}

#[test]
fn transcript_refuses_protected_words_and_bad_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.vibecut");
    let item = word_project(&path, true);
    let before = fs::read(&path).unwrap();
    let protected = cli(&[
        os("transcript"),
        path.as_os_str(),
        os("--item"),
        os(&item.to_string()),
        os("--cue"),
        os("câu-1"),
        os("--words"),
        os("0-0"),
        os("--json"),
    ]);
    assert_eq!(code(&protected), 5);
    assert_eq!(error_json(&protected)["error"]["code"], "rejected");
    assert_eq!(fs::read(&path).unwrap(), before);

    for words in ["2-1", "0-9", "bad"] {
        let output = cli(&[
            os("transcript"),
            path.as_os_str(),
            os("--item"),
            os(&item.to_string()),
            os("--cue"),
            os("câu-1"),
            os("--words"),
            os(words),
            os("--json"),
        ]);
        assert_eq!(code(&output), 2, "{words}");
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let missing = cli(&[
        os("transcript"),
        path.as_os_str(),
        os("--item"),
        os(&item.to_string()),
        os("--json"),
    ]);
    assert_eq!(code(&missing), 2);
}
