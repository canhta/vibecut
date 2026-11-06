use std::fs;

use text::{
    Align, Canvas, Color, LayoutOptions, Outline, Paint, Rectangle, StrokeKind, layout, render,
};

const VI_GOLDEN: u64 = 2654565656580623616;

const LINES: &str = "Người Việt ưỡn ngực, quyết định\nđặt hàng ở Đà Nẵng: Ữ Ỵ Ặ ẫ ợ ự";

fn target_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target")
}

#[test]
fn two_line_vietnamese_frame_overlay_is_a_repeatable_golden() {
    let options = LayoutOptions {
        family: "Inter",
        style: "SemiBold",
        size: 64.0,
        max_width: 1040.0,
        align: Align::Center,
        line_height: 110.0,
        max_lines: Some(2),
        fallbacks: &[],
        line_break: text::LineBreak::Unicode,
        letter_spacing: 0.0,
    };
    let laid_out = layout(LINES, &options).expect("layout");
    assert_eq!(laid_out.lines.len(), 2, "{:?}", laid_out.lines);
    assert!(!laid_out.overflow);
    for line in &laid_out.lines {
        assert!(line.width <= options.max_width + 0.01, "{line:?}");
    }
    assert!(laid_out.lines.get(1).is_some_and(
        |line| line.baseline > laid_out.lines.first().map_or(0.0, |first| first.baseline)
    ));
    assert!(laid_out.glyphs.iter().all(|glyph| glyph.id != 0));

    let paint = Paint {
        fill: Color::WHITE,
        outline: Some(Outline {
            color: Color::BLACK,
            width: 4.0,
            kind: StrokeKind::Outer,
        }),
        background: Some(Rectangle::uniform(Color::rgba(0, 0, 0, 150), 28.0, 18.0)),
        decoration: text::Decoration::None,
    };
    let canvas = Canvas {
        width: 1080,
        height: 1920,
    };
    let image = render(&laid_out, canvas, (20.0, 820.0), &paint).expect("render");
    assert_eq!(image.pixels.len(), 1080 * 1920 * 4);
    let covered = image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] > 0)
        .count();
    assert!(covered > 50_000, "visible overlay coverage: {covered}");
    for pixel in image.pixels.as_chunks::<4>().0 {
        assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
    }

    assert_eq!(
        crate::support::checksum(&image),
        VI_GOLDEN,
        "re-bless only after visual inspection"
    );

    let bare = render(
        &laid_out,
        canvas,
        (20.0, 820.0),
        &Paint {
            fill: Color::WHITE,
            outline: None,
            background: None,
            decoration: text::Decoration::None,
        },
    )
    .expect("text-only render");
    let bare_covered = bare
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] > 0)
        .count();
    assert!(bare_covered > 10_000, "glyph-only coverage: {bare_covered}");
    assert!(
        covered > bare_covered,
        "outline and box add coverage: {covered} vs {bare_covered}"
    );
    let clip_canvas = Canvas {
        width: 400,
        height: 300,
    };
    for origin in [(-200.0, 0.0), (350.0, 0.0), (0.0, -200.0)] {
        let clipped = render(&laid_out, clip_canvas, origin, &paint).expect("clipped render");
        assert_eq!(clipped.pixels.len(), 400 * 300 * 4);
        for pixel in clipped.pixels.as_chunks::<4>().0 {
            assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
        }
    }

    let target = target_dir();
    fs::create_dir_all(&target).expect("create target");
    let first = crate::support::encode(&image);
    fs::write(target.join("text-vi.png"), &first).expect("write overlay");
    assert!(first.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));

    let repeated = render(&laid_out, canvas, (20.0, 820.0), &paint).expect("repeat render");
    let second = crate::support::encode(&repeated);
    fs::write(target.join("text-vi-repeat.png"), &second).expect("write repeat overlay");
    assert_eq!(
        first, second,
        "rendering and PNG encoding are deterministic"
    );
}
