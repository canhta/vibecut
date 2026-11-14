//! Translated names of the built-in text and caption looks.

use super::Text;

/// The translated name of one built-in text look.
pub const fn text_look(look: project::TextLook) -> Text {
    use project::TextLook as Look;
    match look {
        Look::CleanWhite => Text::TextLookCleanWhite,
        Look::CleanBold => Text::TextLookCleanBold,
        Look::PosterAnton => Text::TextLookPosterAnton,
        Look::InkBlack => Text::TextLookInkBlack,
        Look::HandNote => Text::TextLookHandNote,
        Look::ScriptCream => Text::TextLookScriptCream,
        Look::OutlineBlack => Text::TextLookOutlineBlack,
        Look::OutlineYellow => Text::TextLookOutlineYellow,
        Look::OutlineThin => Text::TextLookOutlineThin,
        Look::OutlineNavy => Text::TextLookOutlineNavy,
        Look::OutlineScript => Text::TextLookOutlineScript,
        Look::OutlineHand => Text::TextLookOutlineHand,
        Look::SoftShadow => Text::TextLookSoftShadow,
        Look::HardShadow => Text::TextLookHardShadow,
        Look::LongShadow => Text::TextLookLongShadow,
        Look::ScriptShadow => Text::TextLookScriptShadow,
        Look::HandShadow => Text::TextLookHandShadow,
        Look::NeonCyan => Text::TextLookNeonCyan,
        Look::NeonPink => Text::TextLookNeonPink,
        Look::GlowGold => Text::TextLookGlowGold,
        Look::GlowWhite => Text::TextLookGlowWhite,
        Look::GlowMint => Text::TextLookGlowMint,
        Look::LabelBlack => Text::TextLookLabelBlack,
        Look::LabelWhite => Text::TextLookLabelWhite,
        Look::LabelYellow => Text::TextLookLabelYellow,
        Look::LabelRed => Text::TextLookLabelRed,
        Look::LabelBlue => Text::TextLookLabelBlue,
        Look::LabelPill => Text::TextLookLabelPill,
        Look::LabelMint => Text::TextLookLabelMint,
        Look::LabelShifted => Text::TextLookLabelShifted,
    }
}

/// The translated name of one built-in caption look.
pub const fn caption_look(look: project::CaptionLook) -> Text {
    use project::CaptionLook as Look;
    match look {
        Look::PlainBottom => Text::CaptionLookPlainBottom,
        Look::PlainMiddle => Text::CaptionLookPlainMiddle,
        Look::PlainTop => Text::CaptionLookPlainTop,
        Look::PlainBold => Text::CaptionLookPlainBold,
        Look::PlainLarge => Text::CaptionLookPlainLarge,
        Look::PlainInk => Text::CaptionLookPlainInk,
        Look::PlainCream => Text::CaptionLookPlainCream,
        Look::PlainPoster => Text::CaptionLookPlainPoster,
        Look::OutlineBottom => Text::CaptionLookOutlineBottom,
        Look::OutlineThin => Text::CaptionLookOutlineThin,
        Look::OutlineThick => Text::CaptionLookOutlineThick,
        Look::OutlineYellow => Text::CaptionLookOutlineYellow,
        Look::OutlineTop => Text::CaptionLookOutlineTop,
        Look::OutlineMiddle => Text::CaptionLookOutlineMiddle,
        Look::OutlinePoster => Text::CaptionLookOutlinePoster,
        Look::OutlineHand => Text::CaptionLookOutlineHand,
        Look::LabelBlack => Text::CaptionLookLabelBlack,
        Look::LabelWhite => Text::CaptionLookLabelWhite,
        Look::LabelYellow => Text::CaptionLookLabelYellow,
        Look::LabelPill => Text::CaptionLookLabelPill,
        Look::LabelRed => Text::CaptionLookLabelRed,
        Look::LabelBlue => Text::CaptionLookLabelBlue,
        Look::LabelMint => Text::CaptionLookLabelMint,
        Look::LabelTop => Text::CaptionLookLabelTop,
        Look::KaraokeGold => Text::CaptionLookKaraokeGold,
        Look::KaraokeCyan => Text::CaptionLookKaraokeCyan,
        Look::KaraokePink => Text::CaptionLookKaraokePink,
        Look::KaraokeGreen => Text::CaptionLookKaraokeGreen,
        Look::KaraokeBox => Text::CaptionLookKaraokeBox,
        Look::KaraokePoster => Text::CaptionLookKaraokePoster,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use std::collections::BTreeSet;

    #[test]
    fn every_look_has_a_distinct_key_and_an_english_row_equal_to_its_name() {
        let mut keys = BTreeSet::new();
        for look in project::TextLook::ALL {
            let key = text_look(look);
            assert!(keys.insert(key.key()), "{}", look.name());
            assert_eq!(key.text(Locale::En), look.name());
            assert_ne!(key.text(Locale::Vi), look.name(), "{} vi", look.name());
        }
        for look in project::CaptionLook::ALL {
            let key = caption_look(look);
            assert!(keys.insert(key.key()), "{}", look.name());
            assert_eq!(key.text(Locale::En), look.name());
        }
    }
}
