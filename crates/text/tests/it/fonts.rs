//! Runtime font registration and letter spacing through the public interface.

use text::{FontData, FontError, LayoutOptions, face, generation, layout, register, resolve};

const SERIF: &[u8] = include_bytes!("../../../../assets/fonts/NotoSerif-Regular.ttf");

fn data(family: &str, style: &str, bytes: &[u8]) -> FontData {
    FontData {
        family: family.into(),
        style: style.into(),
        weight: 400,
        italic: false,
        index: 0,
        bytes: bytes.to_vec(),
    }
}

#[test]
fn registered_family_resolves_and_never_changes_bundled_resolution() {
    let before = resolve("Inter", "Bold");
    let generation_before = generation();
    let id = register(data("Registered Sample", "Regular", SERIF)).expect("registers");
    assert!(generation() > generation_before);

    let resolved = resolve("registered sample", "Regular");
    assert_eq!((resolved.face, resolved.missing), (id, false));
    assert_eq!(face(id).family, "Registered Sample");
    assert_eq!(
        face(id).bytes(),
        SERIF,
        "previews read the registered bytes"
    );
    let lines = layout(
        "Hello",
        &LayoutOptions {
            family: "Registered Sample",
            ..Default::default()
        },
    )
    .expect("layout");
    assert!(!lines.missing_font);
    assert!(lines.glyphs.iter().all(|glyph| glyph.face == id));

    assert_eq!(resolve("Inter", "Bold"), before);
    // The same file again is the same face and does not move the generation.
    let again = generation();
    assert_eq!(
        register(data("Registered Sample", "Regular", SERIF)),
        Ok(id)
    );
    assert_eq!(generation(), again);
}

#[test]
fn registration_refuses_hostile_input() {
    let cases: [(FontData, FontError); 7] = [
        (data("", "Regular", SERIF), FontError::InvalidName),
        (data("Hostile", "  ", SERIF), FontError::InvalidName),
        (
            FontData {
                weight: 0,
                ..data("Hostile", "Regular", SERIF)
            },
            FontError::InvalidWeight,
        ),
        (data("inter", "Regular", SERIF), FontError::BundledFamily),
        (
            data("Hostile", "Regular", b"not a font"),
            FontError::Unreadable,
        ),
        (
            data("Hostile", "Regular", &vec![0; text::MAX_FONT_BYTES + 1]),
            FontError::TooLarge,
        ),
        (
            FontData {
                index: 99,
                ..data("Hostile", "Regular", SERIF)
            },
            FontError::Unreadable,
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(register(input), Err(expected));
    }

    register(data("Conflicting", "Regular", SERIF)).expect("first registration");
    let other = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");
    assert_eq!(
        register(data("Conflicting", "Regular", other)),
        Err(FontError::Conflict)
    );
}

#[test]
fn letter_spacing_adds_one_advance_per_cluster_and_refuses_bad_values() {
    let width = |spacing: f32| {
        layout(
            "Hello wörld",
            &LayoutOptions {
                letter_spacing: spacing,
                ..Default::default()
            },
        )
        .map(|laid| laid.lines[0].width)
    };
    let base = width(0.0).expect("layout");
    let clusters = 11.0;
    for spacing in [5.0, -3.0, 0.5] {
        let spaced = width(spacing).expect("layout");
        assert!(
            (spaced - (base + clusters * spacing)).abs() < 0.01,
            "{spacing}: {spaced} vs {base}"
        );
    }
    for bad in [f32::NAN, f32::INFINITY, 4097.0, -4097.0] {
        assert!(
            matches!(width(bad), Err(text::TextError::InvalidLetterSpacing)),
            "{bad}"
        );
    }
}
