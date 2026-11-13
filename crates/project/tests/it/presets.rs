#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
//! Built-in text and caption looks: shelf counts, our names, storable values and what each shelf carries.
use captions::NfcText;
use project::*;
use std::collections::BTreeSet;

/// Vietnamese two-line sample; the golden renders of every look use this text.
const SAMPLE: &str = "Xin chào các bạn\nHôm nay thật vui";

#[test]
fn text_shelves_hold_the_specified_number_of_looks() {
    let presets = text_presets();
    let count = |shelf: TextShelf| presets.iter().filter(|p| p.shelf() == shelf).count();
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
    assert_eq!(presets.len(), 30);
}

#[test]
fn caption_shelves_hold_the_specified_number_of_looks() {
    let presets = caption_presets();
    let count = |shelf: CaptionShelf| presets.iter().filter(|p| p.shelf() == shelf).count();
    assert_eq!(
        [
            count(CaptionShelf::Plain),
            count(CaptionShelf::Outline),
            count(CaptionShelf::Label),
            count(CaptionShelf::Karaoke),
        ],
        [8, 8, 8, 6]
    );
    assert_eq!(presets.len(), 30);
}

#[test]
fn look_names_are_unique_within_each_catalogue_and_storable_as_preset_names() {
    let text: Vec<&str> = text_presets().iter().map(BuiltinTextPreset::name).collect();
    let caption: Vec<&str> = caption_presets()
        .iter()
        .map(BuiltinCaptionPreset::name)
        .collect();
    for names in [text, caption] {
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "every look name is distinct");
        for name in names {
            assert!(
                SavedTextPreset::valid_name(&NfcText::from(name)),
                "{name} is a storable name"
            );
        }
    }
}

#[test]
fn every_text_look_is_a_valid_style_and_its_shelf_carries_its_feature() {
    for preset in text_presets() {
        let style = preset.style();
        assert!(style.valid(), "{} is a valid text style", preset.name());
        match preset.shelf() {
            TextShelf::Plain => assert!(
                style.stroke.is_none()
                    && style.shadow.is_none()
                    && !style.glow
                    && style.background.is_none(),
                "{} is fill only",
                preset.name()
            ),
            TextShelf::Outline => assert!(
                style
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.width > 0.0),
                "{} has a stroke",
                preset.name()
            ),
            TextShelf::Shadow => assert!(style.shadow.is_some(), "{} has a shadow", preset.name()),
            TextShelf::Glow => assert!(style.glow, "{} has a glow", preset.name()),
            TextShelf::Label => assert!(style.background.is_some(), "{} has a box", preset.name()),
        }
    }
}

#[test]
fn applying_a_text_look_keeps_the_item_content_and_copies_its_style() {
    for preset in text_presets() {
        let mut params = TextParams::new(SAMPLE);
        preset.style().apply_to(&mut params);
        assert_eq!(params.content, NfcText::from(SAMPLE), "{}", preset.name());
        assert_eq!(
            TextStyle::from_params(&params),
            *preset.style(),
            "{}",
            preset.name()
        );
    }
}

#[test]
fn text_look_styles_round_trip_through_their_wire_form() {
    for preset in text_presets() {
        let json = serde_json::to_string(preset.style()).unwrap();
        let back: TextStyle = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, preset.style(), "{}", preset.name());
    }
}

#[test]
fn every_caption_look_is_a_valid_style_and_its_shelf_carries_its_feature() {
    for preset in caption_presets() {
        let style = preset.style();
        assert!(
            style.is_valid(),
            "{} is a valid caption style",
            preset.name()
        );
        match preset.shelf() {
            CaptionShelf::Plain => assert!(
                style.outline.is_none() && style.background.is_none(),
                "{} is fill only",
                preset.name()
            ),
            CaptionShelf::Outline => {
                assert!(style.outline.is_some(), "{} has an outline", preset.name())
            }
            CaptionShelf::Label => {
                assert!(style.background.is_some(), "{} has a box", preset.name())
            }
            CaptionShelf::Karaoke => assert!(
                style.outline.is_some() || style.background.is_some(),
                "{} sits on an outline or a box",
                preset.name()
            ),
        }
    }
}
