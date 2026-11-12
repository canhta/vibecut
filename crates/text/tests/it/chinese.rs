use std::error::Error;
use std::fs;

use text::{
    Align, Canvas, Color, LayoutOptions, LineBreak, Outline, Paint, StrokeKind, layout, render,
};

const ZH_GOLDEN: u64 = 481282238247508613;

const SIMPLIFIED: &str = "越南语、英语和中文三种语言的视频都可以转录、翻译并制作双语字幕。";
const TRADITIONAL: &str = "越南語、英語和中文三種語言的影片都可以轉錄、翻譯並製作雙語字幕。";

fn options() -> LayoutOptions<'static> {
    LayoutOptions {
        family: "Inter",
        style: "Regular",
        size: 56.0,
        max_width: 620.0,
        align: Align::Left,
        line_height: 0.0,
        max_lines: None,
        fallbacks: &["VibeCut Sans CJK"],
        line_break: LineBreak::Unicode,
        letter_spacing: 0.0,
    }
}

fn target_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target")
}

fn check(text: &str) -> Result<Vec<u32>, Box<dyn Error>> {
    let laid_out = layout(text, &options())?;
    assert!(
        laid_out.lines.len() > 1,
        "wraps without spaces: {:?}",
        laid_out.lines
    );
    for line in &laid_out.lines {
        assert!(line.width <= options().max_width + 0.01, "{line:?}");
        assert!(text.is_char_boundary(line.range.start));
        assert!(text.is_char_boundary(line.range.end));
    }
    let mut cjk = 0usize;
    for glyph in &laid_out.glyphs {
        assert_ne!(glyph.id, 0, "no tofu: {glyph:?}");
        if text
            .get(glyph.cluster..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(is_han)
        {
            assert_eq!(
                text::face(glyph.face).family,
                "VibeCut Sans CJK",
                "{glyph:?}"
            );
            cjk += 1;
        }
    }
    assert!(cjk >= 10, "Han glyphs from the CJK face: {cjk}");
    Ok(laid_out.glyphs.iter().map(|glyph| glyph.id).collect())
}

fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

#[test]
fn simplified_and_traditional_chinese_wrap_and_render() -> Result<(), Box<dyn Error>> {
    let simplified = layout(SIMPLIFIED, &options())?;
    let simplified_ids = check(SIMPLIFIED)?;
    let traditional_ids = check(TRADITIONAL)?;
    assert!(!simplified_ids.is_empty() && !traditional_ids.is_empty());

    let paint = Paint {
        span_fill: None,
        fill: Color::WHITE,
        outline: Some(Outline {
            color: Color::BLACK,
            width: 4.0,
            kind: StrokeKind::Outer,
        }),
        shadow: None,
        glow: None,
        background: Some(text::Rectangle::uniform(
            Color::rgba(0, 0, 0, 150),
            24.0,
            16.0,
        )),
        decoration: text::Decoration::None,
    };
    let canvas = Canvas {
        width: 760,
        height: 420,
    };
    let image = render(&simplified, canvas, (30.0, 40.0), &paint)?;
    assert_eq!(
        crate::support::checksum(&image),
        ZH_GOLDEN,
        "re-bless only after visual inspection"
    );

    let target = target_dir();
    fs::create_dir_all(&target)?;
    fs::write(target.join("text-zh.png"), crate::support::encode(&image))?;

    let repeated = render(&simplified, canvas, (30.0, 40.0), &paint)?;
    assert_eq!(
        crate::support::encode(&repeated),
        crate::support::encode(&image)
    );

    // The profile's line-breaking mode is data: breaking every grapheme stays inside clusters.
    let grapheme_options = LayoutOptions {
        line_break: LineBreak::Grapheme,
        letter_spacing: 0.0,
        ..options()
    };
    let grapheme = layout(SIMPLIFIED, &grapheme_options)?;
    for line in &grapheme.lines {
        assert!(SIMPLIFIED.is_char_boundary(line.range.start));
        assert!(SIMPLIFIED.is_char_boundary(line.range.end));
    }
    Ok(())
}
