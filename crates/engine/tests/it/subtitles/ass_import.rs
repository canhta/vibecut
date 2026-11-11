#![allow(clippy::unwrap_used, reason = "validated fixtures")]
use engine::{AssImportRequest, Command, Error as EngineError, prepare_ass_import};
use std::fs;
use time::Tick;

const ASS: &str = "\
[Script Info]\nPlayResX: 1920\nPlayResY: 1080\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Hello\n\
Dialogue: 0,bad,0:00:02.00,Default,,0,0,0,,Skipped\n";

#[test]
fn prepare_reads_counts_warnings_and_builds_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("captions.ass");
    fs::write(&path, ASS).unwrap();
    let report = prepare_ass_import(&AssImportRequest {
        path,
        language: captions::LanguageTag::parse("en").unwrap(),
        start: Tick(10),
        track: None,
        name: None,
        asset_id: media::AssetId(7),
    })
    .unwrap();
    assert_eq!(report.cues, 1);
    assert_eq!(report.styles, 1);
    assert_eq!(report.skipped_cues, 1);
    assert!(!report.warnings.is_empty());
    let Command::ImportCaptions { styles, cues, .. } = &report.command else {
        panic!("wrong command")
    };
    assert_eq!(styles.len(), 1);
    assert_eq!(styles[0].font_family.as_str(), "Arial");
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text.as_str(), "Hello");
    assert_eq!(cues[0].style, captions::CueStyleId(0));
}

#[test]
fn prepare_rejects_oversized_files_and_documents_without_cues() {
    let dir = tempfile::tempdir().unwrap();
    let large = dir.path().join("large.ass");
    fs::write(&large, vec![b'x'; captions::ass::MAX_ASS_BYTES + 1]).unwrap();
    let request = |path| AssImportRequest {
        path,
        language: captions::LanguageTag::parse("en").unwrap(),
        start: Tick::ZERO,
        track: None,
        name: None,
        asset_id: media::AssetId(1),
    };
    assert!(matches!(
        prepare_ass_import(&request(large)),
        Err(EngineError::Control(engine::control::ControlError::Limit))
    ));
    let empty = dir.path().join("empty.ass");
    fs::write(&empty, b"[Script Info]\nTitle: none\n").unwrap();
    assert!(matches!(
        prepare_ass_import(&request(empty)),
        Err(EngineError::Edit(engine::EditError::Unsupported(_)))
    ));
}
