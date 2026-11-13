use text::{Align, LayoutOptions, face, fallback_for, layout, resolve};

fn options(family: &'static str) -> LayoutOptions<'static> {
    LayoutOptions {
        family,
        style: "Regular",
        size: 48.0,
        max_width: 800.0,
        align: Align::Left,
        line_height: 0.0,
        max_lines: None,
        fallbacks: &[],
        line_break: text::LineBreak::Unicode,
        letter_spacing: 0.0,
        case: text::Case::AsTyped,
    }
}

#[test]
fn fallback_for_a_character_the_requested_face_lacks_is_deterministic() {
    let inter = resolve("Inter", "Regular").face;
    let serif = resolve("Noto Serif", "Regular").face;
    let inter_face = face(inter);
    let serif_face = face(serif);
    let candidate = (0x20u32..0x3000)
        .filter_map(char::from_u32)
        .find(|c| c.is_alphanumeric() && !inter_face.has_char(*c) && serif_face.has_char(*c));

    if let Some(c) = candidate {
        assert_eq!(
            fallback_for(c, inter, &[serif]),
            serif,
            "the serif face covers {c:?}"
        );
        assert_eq!(
            fallback_for(c, serif, &[]),
            serif,
            "a covered character stays on its face"
        );
        let text = c.to_string();
        let first = layout(&text, &options("Inter")).expect("layout");
        let second = layout(&text, &options("Inter")).expect("repeat layout");
        assert_eq!(first.glyphs, second.glyphs);
        assert_eq!(first.glyphs.first().map(|glyph| glyph.face), Some(serif));
        assert!(first.glyphs.iter().all(|glyph| glyph.id != 0));
    } else {
        // Every probed character is covered by Inter, so test the uncovered path instead.
        let uncovered = '\u{5d0}';
        assert!(!inter_face.has_char(uncovered));
        assert_eq!(fallback_for(uncovered, inter, &[]), inter);
        let first = layout(&uncovered.to_string(), &options("Inter")).expect("layout");
        let second = layout(&uncovered.to_string(), &options("Inter")).expect("repeat layout");
        assert_eq!(first.glyphs, second.glyphs);
    }
}
