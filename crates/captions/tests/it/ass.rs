#![allow(clippy::unwrap_used, reason = "validated test fixtures")]
use captions::{
    AssWarningKind, Cue, CueStyleId, CueTrack, HorizontalAlignment, LanguageTag, MAX_CUE_STYLES,
    VerticalAlignment, ass::MAX_ASS_BYTES, parse_ass,
};
use time::{Rounding, SourceRange, Tick};

fn tick(seconds: f64) -> Tick {
    Tick::from_seconds_f64(seconds, Rounding::Nearest).unwrap()
}

/// The style a parsed cue references in the import's shared table.
fn cue_style(import: &captions::AssImport, cue: usize) -> &captions::CueStyle {
    let style = import.cues.get(cue).unwrap().style.0;
    import.styles.get(usize::from(style)).unwrap()
}

fn doc(play_res: &str, style: &str, dialogue: &str) -> String {
    format!(
        "[Script Info]\nScriptType: v4.00+\n{play_res}\n\n[V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         {style}\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n{dialogue}\n"
    )
}

#[test]
fn styled_ass_maps_supported_fields_and_per_cue_overrides() {
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,-1,0,100,100,0,0,1,2,0,7,100,200,50,1",
        "Dialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\i0\\b1\\fs30\\an2\\c&H0000FF&}Hello\\Nworld",
    );
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.cues.len(), 1);
    assert_eq!(import.styles.len(), 1);
    assert_eq!(import.skipped_cues, 0);
    let cue = &import.cues[0];
    assert_eq!(cue.range.start(), tick(1.5));
    assert_eq!(cue.range.end(), tick(3.0));
    assert_eq!(cue.text.as_str(), "Hello\nworld");
    let style = cue_style(&import, 0);
    assert_eq!(style.font_family.as_str(), "Arial");
    assert!((style.font_size_ratio - 30.0 / 1080.0).abs() < 1e-6);
    assert_eq!(style.primary_color, [255, 0, 0, 255]);
    assert_eq!(style.outline_color, [0, 0, 0, 255]);
    assert_eq!(style.background_color, [0, 0, 0, 127]);
    assert_eq!(style.border_style, 1);
    assert_eq!(style.alignment.numpad, 2);
    assert_eq!(style.alignment.horizontal, HorizontalAlignment::Center);
    assert_eq!(style.alignment.vertical, VerticalAlignment::Bottom);
    assert!((style.margin_left_ratio - 100.0 / 1920.0).abs() < 1e-6);
    assert!((style.margin_right_ratio - 200.0 / 1920.0).abs() < 1e-6);
    assert!((style.margin_vertical_ratio - 50.0 / 1080.0).abs() < 1e-6);
    assert!(style.bold);
    assert!(!style.italic);
    assert!(style.underline);
    assert!(!style.strikeout);
    assert!(style.is_valid());
}

#[test]
fn play_resolution_scales_ratio_for_small_scripts() {
    let text = doc(
        "PlayResX: 384\nPlayResY: 288",
        "Style: Small,Arial,18.5,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,1,0,0,0,1",
        "Dialogue: 0,0:00:00.00,0:00:02.00,Small,,0,0,0,,Small",
    );
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.play_res_x, 384);
    assert_eq!(import.play_res_y, 288);
    let style = cue_style(&import, 0);
    assert!((style.font_size_ratio - 18.5 / 288.0).abs() < 1e-6);
    assert_eq!(style.alignment.numpad, 1);
    assert_eq!(style.alignment.horizontal, HorizontalAlignment::Left);
    assert_eq!(style.alignment.vertical, VerticalAlignment::Bottom);
    assert!(!style.bold);
}

#[test]
fn bom_crlf_and_utf16_are_decoded() {
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
        "Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Café",
    )
    .replace('\n', "\r\n");
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(text.as_bytes());
    let import = parse_ass(&bom).unwrap();
    assert_eq!(import.cues[0].text.as_str(), "Café");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == AssWarningKind::Bom)
    );
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == AssWarningKind::LineEndings)
    );

    let mut utf16 = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let import = parse_ass(&utf16).unwrap();
    assert_eq!(import.cues[0].text.as_str(), "Café");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == AssWarningKind::Encoding)
    );
}

#[test]
fn malformed_styles_and_cues_are_skipped_with_line_numbers() {
    let text = "\
[Script Info]\nPlayResX: 1920\nPlayResY: 1080\n\
[V4+ Styles]\n\
Style: NoFormat,Arial,20\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,not-a-time,0:00:01.00,Default,,0,0,0,,Bad start\n\
Dialogue: 0,0:00:02.00,0:00:02.00,Default,,0,0,0,,Zero\n\
Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,   {\\i1}   \n\
Comment: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,metadata\n\
Picture: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,logo.png\n\
Dialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,Good\n";
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.cues.len(), 1);
    assert_eq!(import.cues[0].text.as_str(), "Good");
    assert_eq!(import.skipped_styles, 1);
    assert_eq!(import.skipped_cues, 3);
    let lines: Vec<_> = import.warnings.iter().map(|w| (w.line, w.kind)).collect();
    assert!(lines.contains(&(5, AssWarningKind::MissingStyleFormat)));
    assert!(lines.contains(&(10, AssWarningKind::MalformedCue)));
    assert!(lines.contains(&(11, AssWarningKind::MalformedCue)));
    assert!(lines.contains(&(12, AssWarningKind::EmptyText)));
    assert!(
        !lines.iter().any(|(line, _)| *line == 13),
        "comments are not warnings"
    );
    assert!(lines.contains(&(14, AssWarningKind::UnsupportedEvent)));
}

#[test]
fn short_style_records_are_counted_as_malformed() {
    let text = "\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize\n\
Style: Broken\n";
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.styles.len(), 0);
    assert_eq!(import.skipped_styles, 1);
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.line == 3 && w.kind == AssWarningKind::MalformedStyle)
    );
}

#[test]
fn transform_and_karaoke_tags_are_ignored_with_warnings_never_rendered() {
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
        "Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,{\\pos(10,20)\\fad(100,200)\\k20}Hello",
    );
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.cues[0].text.as_str(), "Hello");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.line == 12 && w.kind == AssWarningKind::UnsupportedOverride)
    );
}

#[test]
fn missing_style_falls_back_to_default_with_a_warning() {
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
        "Dialogue: 0,0:00:00.00,0:00:01.00,Unknown,,0,0,0,,Hi",
    );
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.cues.len(), 1);
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == AssWarningKind::MissingStyle)
    );
}

#[test]
fn truncated_input_never_panics() {
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
        "Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{\\i1unclosed",
    );
    for cut in [0, 1, text.len() / 2, text.len().saturating_sub(1)] {
        let _ = parse_ass(&text.as_bytes()[..cut]);
    }
    let import = parse_ass(text.as_bytes()).unwrap();
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == AssWarningKind::TruncatedOverride)
    );
}

#[test]
fn oversized_documents_and_cue_counts_fail_typed() {
    let oversized = vec![b'x'; MAX_ASS_BYTES + 1];
    assert!(matches!(
        parse_ass(&oversized),
        Err(captions::Error::TooLarge)
    ));

    let mut text = String::from(
        "[Script Info]\nPlayResX: 1920\nPlayResY: 1080\n[Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for index in 0..=captions::ass::MAX_ASS_CUES {
        text.push_str(&format!(
            "Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Cue {index}\n"
        ));
    }
    assert!(matches!(
        parse_ass(text.as_bytes()),
        Err(captions::Error::TooLarge)
    ));
}

#[test]
fn warning_retention_is_bounded_and_counted() {
    let mut text = String::from(
        "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for _ in 0..(captions::ass::MAX_ASS_WARNINGS + 25) {
        text.push_str("Dialogue: 0,bad,0:00:01.00,Default,,0,0,0,,Skipped\n");
    }
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.warnings.len(), captions::ass::MAX_ASS_WARNINGS);
    assert_eq!(import.suppressed_warnings, 25);
}

#[test]
fn deeply_repeated_override_tags_are_bounded() {
    let mut dialogue = String::from("Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,");
    for _ in 0..20_000 {
        dialogue.push_str("{\\b1}");
    }
    dialogue.push_str("Text");
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
        &dialogue,
    );
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.cues.len(), 1);
    assert_eq!(import.cues[0].text.as_str(), "Text");
}

#[test]
fn parse_time_scales_linearly() {
    fn fixture(cues: usize) -> String {
        let mut text = String::from(
            "[Script Info]\nPlayResX: 1920\nPlayResY: 1080\n[V4+ Styles]\n\
             Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
             Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1\n\
             [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        );
        for index in 0..cues {
            let start = index as f64;
            let end = start + 0.75;
            let stamp = |value: f64| {
                let minutes = (value / 60.0) as u64;
                let seconds = value - minutes as f64 * 60.0;
                format!("0:{minutes:02}:{seconds:05.2}")
            };
            text.push_str(&format!(
                "Dialogue: 0,{}, {},Default,,0,0,0,,{{\\i1}}Line {index}\\Nsecond\n",
                stamp(start),
                stamp(end)
            ));
        }
        text
    }
    fn measure(cues: usize) -> u128 {
        let text = fixture(cues);
        let started = std::time::Instant::now();
        let import = parse_ass(text.as_bytes()).unwrap();
        assert_eq!(import.cues.len(), cues);
        started.elapsed().as_nanos()
    }
    let _ = measure(50);
    let small = measure(200).max(1);
    let middle = measure(800).max(1);
    let large = measure(3_200).max(1);
    assert!(
        middle < small.saturating_mul(16),
        "small={small} middle={middle}"
    );
    assert!(
        large < small.saturating_mul(64),
        "small={small} large={large}"
    );
}

#[test]
fn empty_and_minimal_documents_are_not_errors() {
    let import = parse_ass(b"").unwrap();
    assert!(import.cues.is_empty());
    assert_eq!(import.play_res_x, 384);
    assert_eq!(import.play_res_y, 288);
    let import = parse_ass(b"[Script Info]\nTitle: X\n").unwrap();
    assert!(import.cues.is_empty());
}

const DEFAULT_STYLE: &str = "Style: Default,Arial,60,&H80FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1";

fn one_cue(style: &str, text: &str) -> captions::AssImport {
    let dialogue = format!("Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{text}");
    parse_ass(doc("PlayResX: 1920\nPlayResY: 1080", style, &dialogue).as_bytes()).unwrap()
}

#[test]
fn override_tags_follow_ass_semantics() {
    // Tags inside a `\t(...)` transform never reach the static style.
    let import = one_cue(DEFAULT_STYLE, "{\\t(0,500,\\fs90\\i1)\\u1}Hi");
    let style = cue_style(&import, 0);
    assert!((style.font_size_ratio - 60.0 / 1080.0).abs() < 1e-6);
    assert!(!style.italic && style.underline);
    // A bare `\b` restores the style value; repeating it never toggles.
    let import = one_cue(DEFAULT_STYLE, "{\\b0}{\\b}{\\b}Hi");
    assert!(cue_style(&import, 0).bold);
    // `\c` keeps the style's alpha.
    let import = one_cue(DEFAULT_STYLE, "{\\c&H0000FF&}Hi");
    assert_eq!(cue_style(&import, 0).primary_color, [255, 0, 0, 127]);
    // Vector drawings are shapes, not text.
    let import = one_cue(DEFAULT_STYLE, "{\\p1}m 0 0 l 100 0 100 100{\\p0}Logo");
    assert_eq!(import.cues[0].text.as_str(), "Logo");
    let import = one_cue(DEFAULT_STYLE, "{\\p1}m 0 0 l 100 0 100 100");
    assert!(import.cues.is_empty());
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == AssWarningKind::EmptyText)
    );
}

#[test]
fn hostile_style_values_are_replaced_with_a_warning() {
    for (style, flagged) in [
        (
            "Style: Default,Arial,1e9,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
            true,
        ),
        (
            "Style: Default,Arial,-5,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,1e12,0,0,1",
            true,
        ),
        (
            "Style: Default,Arial,1e999,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
            true,
        ),
        (DEFAULT_STYLE, false),
    ] {
        let import = one_cue(style, "Hi");
        let cue = cue_style(&import, 0);
        assert!(cue.is_valid(), "{style}");
        assert!(cue.font_size_ratio > 0.01, "{style}");
        let warned = import.warnings.iter().any(|w| {
            matches!(
                w.kind,
                AssWarningKind::Limit | AssWarningKind::MalformedStyle
            )
        });
        assert_eq!(warned, flagged, "{style}");
    }
    // BorderStyle 0 is not a reason to drop the whole style.
    let import = one_cue(
        "Style: Default,Courier,40,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,0,0,0,2,0,0,0,1",
        "Hi",
    );
    assert_eq!(cue_style(&import, 0).font_family.as_str(), "Courier");
    assert!(import.warnings.is_empty());
    // Mismatched numpad/horizontal/vertical never validates.
    let mut style = cue_style(&import, 0).clone();
    style.alignment.horizontal = HorizontalAlignment::Right;
    assert!(!style.is_valid());
}

#[test]
fn cue_track_shares_one_bounded_style_table() {
    let import = one_cue(DEFAULT_STYLE, "Hi");
    let style = cue_style(&import, 0).clone();
    let mut track = CueTrack::default();
    let id = track.intern_style(style.clone()).unwrap();
    assert_eq!(id, CueStyleId(0));
    assert_eq!(track.intern_style(style.clone()), Some(id));
    assert_eq!(track.styles.len(), 1);
    track.cues.push(Cue {
        word_timing: captions::CueWordTiming::Unverified,
        id: "1".into(),
        range: SourceRange::new(Tick::ZERO, Tick(1)).unwrap(),
        source_lang: LanguageTag::parse("en").unwrap(),
        texts: std::collections::BTreeMap::new(),
        style: Some(id),
    });
    assert_eq!(track.style_of(&track.cues[0]), Some(&style));
    let json = serde_json::to_string(&track).unwrap();
    assert_eq!(serde_json::from_str::<CueTrack>(&json).unwrap(), track);
    // A stale id resolves to nothing; the table stops at its cap.
    let mut dangling = track.clone();
    dangling.cues[0].style = Some(CueStyleId(u16::MAX));
    assert!(dangling.style_of(&dangling.cues[0]).is_none());
    let mut full = CueTrack {
        styles: vec![style.clone(); MAX_CUE_STYLES],
        cues: Vec::new(),
    };
    let mut other = style;
    other.bold = !other.bold;
    assert!(full.intern_style(other).is_none());
}

#[test]
fn ass_import_interns_one_bounded_table_entry_per_style() {
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1\n\
         Style: Alt,Courier,40,&H000000FF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,8,0,0,0,1",
        "Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,One\n\
         Dialogue: 0,0:00:01.00,0:00:02.00,Alt,,0,0,0,,Two\n\
         Dialogue: 0,0:00:02.00,0:00:03.00,Default,,0,0,0,,Three\n\
         Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,{\\fs30}Four\n\
         Dialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,{\\fs30}Five",
    );
    let import = parse_ass(text.as_bytes()).unwrap();
    assert_eq!(import.cues.len(), 5);
    assert_eq!(
        import.styles.len(),
        3,
        "two ASS styles plus one override result"
    );
    assert_eq!(import.cues[0].style, CueStyleId(0));
    assert_eq!(import.cues[2].style, import.cues[0].style);
    assert_eq!(import.cues[1].style, CueStyleId(1));
    assert_eq!(import.cues[3].style, import.cues[4].style);
    assert_ne!(import.cues[3].style, import.cues[0].style);
    assert!((cue_style(&import, 3).font_size_ratio - 30.0 / 1080.0).abs() < 1e-6);

    // The shared table is capped: more distinct used styles refuse the whole
    // document instead of growing it without bound.
    let mut dialogue = String::new();
    for index in 1..=MAX_CUE_STYLES + 1 {
        dialogue.push_str(&format!(
            "Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{{\\fs{index}}}Cue {index}\n"
        ));
    }
    let text = doc(
        "PlayResX: 1920\nPlayResY: 1080",
        "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1",
        &dialogue,
    );
    assert!(matches!(
        parse_ass(text.as_bytes()),
        Err(captions::Error::TooLarge)
    ));
}

#[test]
fn legacy_ssa_styles_use_ssa_alignment_and_decimal_colours() {
    let text = "[V4 Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, TertiaryColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, AlphaLevel, Encoding\n\
Style: Default,Arial,20,16711680,0,255,0,0,0,1,2,0,6,0,0,0,0,0\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Hi\n";
    let document = parse_ass(text.as_bytes()).unwrap();
    let style = cue_style(&document, 0);
    // SSA 6 is top centre, i.e. numpad 8.
    assert_eq!(style.alignment.numpad, 8);
    assert_eq!(style.alignment.vertical, VerticalAlignment::Top);
    // Decimal BGR 16711680 = 0x00FF0000 is blue; TertiaryColour is the outline.
    assert_eq!(style.primary_color, [0, 0, 255, 255]);
    assert_eq!(style.outline_color, [255, 0, 0, 255]);
}
