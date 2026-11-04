use captions::*;
use proptest::prelude::*;
use time::{SourceRange, TICKS_PER_SECOND, Tick};
#[allow(clippy::unwrap_used, reason = "test fixture constructor")]
fn lang(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}
#[allow(clippy::unwrap_used, reason = "test fixture constructor")]
fn read(s: &str) -> Import {
    parse(s.as_bytes(), Format::Srt, lang("vi")).unwrap()
}
fn has(w: &[Warning], kind: WarningKind) -> bool {
    w.iter().any(|w| w.kind == kind)
}
#[test]
fn nfc_persistence_and_protected_edits() {
    let mut track = read("00:00,000 --> 00:01,000\nTiếng Việt\n").track;
    let cue = track.cues.first_mut().unwrap();
    assert_eq!(
        cue.texts.get(&lang("vi")).unwrap().text.as_str(),
        "Tiếng Việt"
    );
    let edited = CueText {
        text: "my edit".into(),
        edited: true,
        translation_ref: Some("ref".into()),
    };
    assert!(cue.set_text(lang("en"), edited.clone(), false));
    assert!(!cue.set_text(
        lang("en"),
        CueText {
            text: "replacement".into(),
            ..edited.clone()
        },
        false
    ));
    assert_eq!(cue.texts.get(&lang("en")), Some(&edited));
    let json = serde_json::to_string(&track).unwrap();
    assert_eq!(serde_json::from_str::<CueTrack>(&json).unwrap(), track);
    assert_eq!(
        serde_json::from_str::<NfcText>("\"e\\u0301\"")
            .unwrap()
            .as_str(),
        "é"
    );
    assert!(serde_json::from_str::<SourceRange>(r#"{"start":1,"end":1}"#).is_err());
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(Confidence::try_from(value).is_err());
    }
    assert!(!format!("{:?}", track).contains("my edit"));
}
#[test]
fn extensible_profiles_and_tag_keys() {
    assert_eq!(lang("ZH-hans"), lang("zh-Hans"));
    for tag in ["", "en_US", "a!", "en--US"] {
        assert!(LanguageTag::parse(tag).is_err());
    }
    assert!(serde_json::from_str::<LanguageTag>("\"en_US\"").is_err());
    let mut registry = LanguageRegistry::standard().unwrap();
    assert_eq!(registry.profile(&lang("zh-Hant-TW")).max_graphemes, 16);
    assert_eq!(registry.profile(&lang("fr")), &registry.generic);
    let mut custom = registry.generic.clone();
    custom.max_graphemes = 31;
    registry.profiles.insert(lang("fr"), custom);
    assert_eq!(registry.profile(&lang("fr-CA")).max_graphemes, 31);
}
#[test]
fn repairs_and_empty_export() {
    let imported = read(
        "4\n00:03,000 --> 00:03,000\nC\n\n00:01,000 --> 00:02,000\nA\n00:01,500 --> 00:01,400\nB\n\n00:04,000 --> 00:05,000\n\ninvalid\n",
    );
    for kind in [
        WarningKind::Duration,
        WarningKind::Overlap,
        WarningKind::NonMonotonic,
        WarningKind::Malformed,
    ] {
        assert!(has(&imported.warnings, kind), "{kind:?}");
    }
    let output = export(&imported.track, Format::Srt, &[lang("vi")]).unwrap();
    assert!(has(&output.warnings, WarningKind::EmptyText));
    assert_eq!(read(&output.text).track.cues.len(), 3);
    let mut track = read("00:00,000 --> 00:01,000\nA\n").track;
    track.cues.first_mut().unwrap().range = SourceRange::new(Tick(-1), Tick(1)).unwrap();
    assert!(has(&track.repair().unwrap(), WarningKind::NegativeStart));
    track.cues.first_mut().unwrap().range =
        SourceRange::new(Tick(i64::MAX - 1), Tick(i64::MAX)).unwrap();
    assert!(export(&track, Format::Srt, &[lang("vi")]).is_err());
}
#[test]
fn encodings_and_truncation() {
    let source = "00:00,000 --> 00:01,000\r\nTiếng Việt\r\n";
    for little in [true, false] {
        let mut bytes = if little {
            vec![255, 254]
        } else {
            vec![254, 255]
        };
        for unit in source.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        let result = parse(&bytes, Format::Srt, lang("vi")).unwrap();
        for kind in [
            WarningKind::Bom,
            WarningKind::Encoding,
            WarningKind::LineEndings,
        ] {
            assert!(has(&result.warnings, kind));
        }
        assert_eq!(
            result
                .track
                .cues
                .first()
                .unwrap()
                .texts
                .get(&lang("vi"))
                .unwrap()
                .text
                .as_str(),
            "Tiếng Việt"
        );
        bytes.push(0);
        assert!(has(
            &parse(&bytes, Format::Srt, lang("vi")).unwrap().warnings,
            WarningKind::Replacement
        ));
    }
    let cp = parse(
        b"00:00,000 --> 00:01,000\n\x93caf\xe9\x94",
        Format::Srt,
        lang("en"),
    )
    .unwrap();
    assert_eq!(
        cp.track
            .cues
            .first()
            .unwrap()
            .texts
            .get(&lang("en"))
            .unwrap()
            .text
            .as_str(),
        "“café”"
    );
    assert!(has(
        &parse(b"\xef\xbb\xbf\xff", Format::Srt, lang("en"))
            .unwrap()
            .warnings,
        WarningKind::Replacement
    ));
    assert!(parse(&vec![0; 16 * 1024 * 1024 + 1], Format::Srt, lang("en")).is_err());
}
#[test]
fn bilingual_vtt_and_rounding() {
    let mut track = read("00:00,000 --> 00:01,000\nA --> B & <x>\n").track;
    let cue = track.cues.first_mut().unwrap();
    cue.set_text(
        lang("zh-Hans"),
        CueText {
            text: "中文".into(),
            edited: false,
            translation_ref: None,
        },
        false,
    );
    let output = export(&track, Format::WebVtt, &[lang("zh-Hans"), lang("vi")]).unwrap();
    assert!(output.text.contains("中文\nA --&gt; B &amp; &lt;x&gt;"));
    let decoded = parse(output.text.as_bytes(), Format::WebVtt, lang("vi")).unwrap();
    assert_eq!(
        export(&decoded.track, Format::WebVtt, &[lang("vi")])
            .unwrap()
            .text,
        output.text
    );
    assert!(export(&track, Format::Srt, &[]).is_err());
    assert!(export(&track, Format::Srt, &[lang("vi"), lang("vi")]).is_err());
    track.cues.first_mut().unwrap().range = SourceRange::new(
        Tick(TICKS_PER_SECOND / 2000),
        Tick(TICKS_PER_SECOND / 2000 + 1),
    )
    .unwrap();
    let output = export(&track, Format::Srt, &[lang("vi")]).unwrap();
    assert!(output.text.contains("00:00:00,001 --> 00:00:00,002"));
    assert!(has(&output.warnings, WarningKind::RoundedDuration));
}
#[test]
fn metadata_and_hostile_clocks() {
    let result=parse(b"WEBVTT\n\nNOTE ignored\n00:00:04.000 is not timing\n\nSTYLE\n::cue{}\n\nid\n00:00.000 --> 00:01.000 align:start\nx",Format::WebVtt,lang("en")).unwrap();
    assert_eq!(result.track.cues.len(), 1);
    assert!(has(&result.warnings, WarningKind::Metadata));
    for stamp in [
        "999999999999999999:00:00",
        "00:60:00",
        "-00:00:01",
        "00:00:00.1234567890",
        "00:00:00.",
    ] {
        assert!(
            read(&format!("{stamp} --> 00:02,000\nx"))
                .track
                .cues
                .is_empty()
        );
    }
}
proptest! {
    #[test]
    fn canonical_interchange(start in 0_i64..30_000_000, duration in 1_i64..10000, payload in "[a-zé中 ]{1,80}") {
        let source=format!("00:00,000 --> 00:01,000\n{payload}");
        let mut track=read(&source).track;
        track.cues.first_mut().unwrap().range=SourceRange::new(Tick(start*254_016_000),Tick((start+duration)*254_016_000)).unwrap();
        for format in [Format::Srt,Format::WebVtt] {
            let first=export(&track,format,&[lang("vi")]).unwrap();
            let parsed=parse(first.text.as_bytes(),format,lang("vi")).unwrap();
            prop_assert_eq!(export(&parsed.track,format,&[lang("vi")]).unwrap().text,first.text);
        }
    }
    #[test]
    fn hostile_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(),0..2048)) {
        for format in [Format::Srt,Format::WebVtt] { let _=parse(&bytes,format,lang("en")); }
    }
}

#[test]
fn transcript_wire_and_export_safety() {
    let range = SourceRange::new(Tick(0), Tick(TICKS_PER_SECOND)).unwrap();
    let transcript = Transcript {
        source_lang: lang("vi"),
        fingerprint: Fingerprint {
            provider: "local".into(),
            model: "model".into(),
            settings: "e\u{301}".into(),
        },
        segments: vec![Segment {
            id: "segment-1".into(),
            range,
            text: "e\u{301}".into(),
            spans: None,
            words: vec![Word {
                text: "e\u{301}".into(),
                range: Some(range),
                speaker: Some("speaker".into()),
                confidence: Some(Confidence::try_from(0.8).unwrap()),
            }],
        }],
    };
    assert_eq!(
        serde_json::from_str::<Transcript>(&serde_json::to_string(&transcript).unwrap()).unwrap(),
        transcript
    );
    let mut track = read("00:00,000 --> 00:01,000\nx").track;
    let cue = track.cues.first_mut().unwrap();
    let replacement = CueText {
        text: "one\n\ntwo".into(),
        edited: true,
        translation_ref: None,
    };
    assert!(cue.set_text(lang("vi"), replacement, true));
    let output = export(&track, Format::Srt, &[lang("vi"), lang("en")]).unwrap();
    assert!(has(&output.warnings, WarningKind::MissingLanguage));
    assert!(has(&output.warnings, WarningKind::TextLayout));
    assert!(output.text.ends_with("one\ntwo\n\n"));
    track
        .cues
        .first_mut()
        .unwrap()
        .texts
        .get_mut(&lang("vi"))
        .unwrap()
        .text = "00:01,000 --> 00:02,000".into();
    assert!(export(&track, Format::Srt, &[lang("vi")]).is_err());
    assert!(export(&track, Format::WebVtt, &[lang("vi")]).is_ok());
    let header = parse(
        b"WEBVTT\n00:00.000 --> 00:01.000\nx",
        Format::WebVtt,
        lang("en"),
    )
    .unwrap();
    assert_eq!(header.track.cues.len(), 1);
}
