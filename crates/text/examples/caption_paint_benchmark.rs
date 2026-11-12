//! Same-input CPU caption shaping/paint timing and process-memory probe; no native window.
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let active = std::env::args()
        .nth(2)
        .is_some_and(|value| value == "active");
    let destination = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if let Some(path) = &destination {
        std::fs::create_dir_all(path)?;
    }
    println!(
        "active={active} pid={} arch={} profile={} cold_repeats=3 warm_repeats=8 canvas=640x240",
        std::process::id(),
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for (name, value) in [
        ("vi-en", "Tiếng Việt đẹp và rõ\nClear English captions"),
        ("zh-en", "中文词语清晰可读\nClear English captions"),
        ("combining", "Tiếng Việt\r\nEnglish office affine"),
    ] {
        let options = text::LayoutOptions {
            size: 44.0,
            max_width: 620.0,
            align: text::Align::Center,
            ..Default::default()
        };
        let mut paint = text::Paint {
            fill: text::Color::rgba(245, 245, 250, 180),
            outline: Some(text::Outline {
                color: text::Color::rgba(0, 0, 0, 200),
                width: 2.0,
                kind: text::StrokeKind::Outer,
            }),
            background: Some(text::Rectangle::uniform(
                text::Color::rgba(20, 30, 50, 100),
                4.0,
                3.0,
            )),
            ..Default::default()
        };
        if active {
            paint.span_fill = Some(text::SpanFill {
                start: 0,
                end: if name == "zh-en" { 6 } else { "Tiếng".len() },
                rgb: [255, 215, 0],
            });
        }
        let canvas = text::Canvas {
            width: 640,
            height: 240,
        };
        let started = Instant::now();
        for _ in 0..3 {
            let layout = text::layout(value, &options)?;
            std::hint::black_box(text::render(&layout, canvas, (10.0, 10.0), &paint)?);
        }
        let cold = started.elapsed().as_micros();
        let layout = text::layout(value, &options)?;
        let started = Instant::now();
        for _ in 0..8 {
            std::hint::black_box(text::render(&layout, canvas, (10.0, 10.0), &paint)?);
        }
        let warm = started.elapsed().as_micros();
        let image = text::render(&layout, canvas, (10.0, 10.0), &paint)?;
        let hash = image
            .pixels
            .iter()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
        if let Some(path) = &destination {
            std::fs::write(path.join(format!("{name}.rgba")), &image.pixels)?;
        }
        println!(
            "fixture={name} cold_total_us={cold} warm_total_us={warm} glyphs={} fnv1a={hash:016x}",
            layout.glyphs.len()
        );
    }
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status
            .lines()
            .filter(|line| line.starts_with("VmHWM:") || line.starts_with("VmRSS:"))
        {
            println!("{line}");
        }
    }
    Ok(())
}
