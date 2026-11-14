//! Measures the colour scope plots against a 1080p frame.
use render::{ScopeKind, plot_scope};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "os={} arch={} profile={} frame=1920x1080 repetitions=10",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let (width, height) = (1920_usize, 1080_usize);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for n in 0..width * height {
        let (x, y) = (n % width, n / width);
        rgba.extend_from_slice(&[
            (x * 255 / width) as u8,
            (y * 255 / height) as u8,
            (n % 256) as u8,
            255,
        ]);
    }
    for kind in ScopeKind::ALL {
        let mut times = Vec::new();
        for _ in 0..10 {
            let start = Instant::now();
            let plot = plot_scope(kind, width, height, &rgba)?;
            times.push(start.elapsed());
            std::hint::black_box(plot);
        }
        times.sort();
        let median = times.get(times.len() / 2).copied().unwrap_or_default();
        println!("{kind:?} median_ms={:.2}", median.as_secs_f64() * 1000.0);
    }
    Ok(())
}
