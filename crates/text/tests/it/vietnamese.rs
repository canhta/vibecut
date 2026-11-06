use std::fs;

use text::{Align, Canvas, Color, LayoutOptions, Outline, Paint, StrokeKind, layout, render};
use unicode_normalization::UnicodeNormalization;

const STRESS: &str = "Người Việt ưỡn ngực, quyết định đặt hàng ở Đà Nẵng: Ữ Ỵ Ặ ẫ ợ ự";

fn target_dir() -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.join("target")
}

fn options() -> LayoutOptions<'static> {
    LayoutOptions {
        family: "Inter",
        style: "Regular",
        size: 64.0,
        max_width: 1000.0,
        align: Align::Left,
        line_height: 0.0,
        max_lines: None,
        fallbacks: &[],
        line_break: text::LineBreak::Unicode,
        letter_spacing: 0.0,
    }
}

#[test]
fn nfc_and_nfd_shapes_render_identically_without_tofu() {
    let nfc: String = STRESS.nfc().collect();
    let nfd: String = STRESS.nfd().collect();
    assert_ne!(
        nfc.len(),
        nfd.len(),
        "the forms exercise different byte lengths"
    );

    let a = layout(&nfc, &options()).expect("NFC layout");
    let b = layout(&nfd, &options()).expect("NFD layout");
    assert_eq!(a.text, b.text, "input is normalized to NFC");
    assert_eq!(a.glyphs, b.glyphs, "glyph runs match");
    assert!(!a.glyphs.is_empty());
    for glyph in &a.glyphs {
        assert_ne!(glyph.id, 0, "no tofu: {glyph:?}");
    }

    let paint = Paint {
        fill: Color::WHITE,
        outline: Some(Outline {
            color: Color::BLACK,
            width: 3.0,
            kind: StrokeKind::Outer,
        }),
        background: None,
    };
    let canvas = Canvas {
        width: 1100,
        height: 220,
    };
    let image_a = render(&a, canvas, (20.0, 30.0), &paint).expect("NFC render");
    let image_b = render(&b, canvas, (20.0, 30.0), &paint).expect("NFD render");
    assert_eq!(image_a.pixels, image_b.pixels, "identical coverage");

    let target = target_dir();
    fs::create_dir_all(&target).expect("create target");
    fs::write(
        target.join("text-vi-nfc.png"),
        crate::support::encode(&image_a),
    )
    .expect("write NFC golden");
    fs::write(
        target.join("text-vi-nfd.png"),
        crate::support::encode(&image_b),
    )
    .expect("write NFD golden");
}
