#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
//! Built-in text and caption looks: shelf counts, our names, wire ids, storable values and what each shelf carries.
use captions::NfcText;
use project::*;
use std::collections::BTreeSet;

/// Vietnamese two-line sample; the golden renders of every look use this text.
const SAMPLE: &str = "Xin chào các bạn\nHôm nay thật vui";

#[test]
fn text_shelves_hold_the_specified_number_of_looks() {
    let count = |shelf: TextShelf| TextLook::ALL.iter().filter(|l| l.shelf() == shelf).count();
    assert_eq!(
        [
            count(TextShelf::Plain),
            count(TextShelf::Outline),
            count(TextShelf::Shadow),
            count(TextShelf::Glow),
            count(TextShelf::Label),
        ],
        [6, 6, 5, 5, 8]
    );
}

#[test]
fn caption_shelves_hold_the_specified_number_of_looks() {
    let count = |shelf: CaptionShelf| {
        CaptionLook::ALL
            .iter()
            .filter(|l| l.shelf() == shelf)
            .count()
    };
    assert_eq!(
        [
            count(CaptionShelf::Plain),
            count(CaptionShelf::Outline),
            count(CaptionShelf::Label),
            count(CaptionShelf::Karaoke),
        ],
        [8, 8, 8, 6]
    );
}

#[test]
fn look_ids_are_distinct_and_names_are_unique_and_storable() {
    let text_ids: BTreeSet<String> = TextLook::ALL
        .iter()
        .map(|l| serde_json::to_string(l).unwrap())
        .collect();
    assert_eq!(text_ids.len(), TextLook::ALL.len());
    let caption_ids: BTreeSet<String> = CaptionLook::ALL
        .iter()
        .map(|l| serde_json::to_string(l).unwrap())
        .collect();
    assert_eq!(caption_ids.len(), CaptionLook::ALL.len());
    assert_eq!(
        serde_json::to_string(&TextLook::CleanWhite).unwrap(),
        "\"clean_white\""
    );
    assert_eq!(
        serde_json::to_string(&CaptionLook::KaraokeGold).unwrap(),
        "\"karaoke_gold\""
    );
    let text_names: BTreeSet<&str> = TextLook::ALL.iter().map(|l| l.name()).collect();
    let caption_names: BTreeSet<&str> = CaptionLook::ALL.iter().map(|l| l.name()).collect();
    assert_eq!(text_names.len(), TextLook::ALL.len());
    assert_eq!(caption_names.len(), CaptionLook::ALL.len());
    for name in text_names.iter().chain(&caption_names) {
        assert!(
            SavedTextPreset::valid_name(&NfcText::from(*name)),
            "{name} is a storable name"
        );
    }
}

#[test]
fn every_text_look_is_a_valid_style_and_its_shelf_carries_its_feature() {
    for look in TextLook::ALL {
        let style = look.style();
        assert!(style.valid(), "{} is a valid text style", look.name());
        match look.shelf() {
            TextShelf::Plain => assert!(
                style.stroke.is_none()
                    && style.shadow.is_none()
                    && !style.glow
                    && style.background.is_none(),
                "{} is fill only",
                look.name()
            ),
            TextShelf::Outline => assert!(
                style
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.width > 0.0),
                "{} has a stroke",
                look.name()
            ),
            TextShelf::Shadow => assert!(style.shadow.is_some(), "{} has a shadow", look.name()),
            TextShelf::Glow => assert!(style.glow, "{} has a glow", look.name()),
            TextShelf::Label => assert!(style.background.is_some(), "{} has a box", look.name()),
        }
    }
}

#[test]
fn applying_a_text_look_keeps_the_item_content_and_copies_its_style() {
    for look in TextLook::ALL {
        let mut params = TextParams::new(SAMPLE);
        look.style().apply_to(&mut params);
        assert_eq!(params.content, NfcText::from(SAMPLE), "{}", look.name());
        assert_eq!(
            TextStyle::from_params(&params),
            look.style(),
            "{}",
            look.name()
        );
    }
}

#[test]
fn every_caption_look_is_a_valid_style_and_its_shelf_carries_its_feature() {
    for look in CaptionLook::ALL {
        let style = look.style();
        assert!(style.is_valid(), "{} is a valid caption style", look.name());
        let cue = look.cue_style();
        assert!(cue.is_valid(), "{} is a valid cue style", look.name());
        assert_eq!(
            cue.max_lines,
            Some(style.max_lines as u8),
            "{}",
            look.name()
        );
        assert_eq!(
            cue.word_highlight_color,
            Some(style.word_highlight_color),
            "{}",
            look.name()
        );
        match look.shelf() {
            CaptionShelf::Plain => assert!(
                style.outline.is_none() && style.background.is_none(),
                "{} is fill only",
                look.name()
            ),
            CaptionShelf::Outline => {
                assert!(style.outline.is_some(), "{} has an outline", look.name())
            }
            CaptionShelf::Label => assert!(style.background.is_some(), "{} has a box", look.name()),
            CaptionShelf::Karaoke => assert!(
                style.outline.is_some() || style.background.is_some(),
                "{} sits on an outline or a box",
                look.name()
            ),
        }
    }
}
