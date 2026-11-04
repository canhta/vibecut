pub(crate) fn progress_for_x(x1: f64, x2: f64, x: f64) -> f64 {
    let bx =
        |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * x1 + 3.0 * (1.0 - s) * s * s * x2 + s * s * s;
    let dbx = |s: f64| {
        3.0 * (1.0 - s) * (1.0 - s) * x1
            + 6.0 * (1.0 - s) * s * (x2 - x1)
            + 3.0 * s * s * (1.0 - x2)
    };
    let mut s = x;
    for _ in 0..8 {
        let d = dbx(s);
        if d.abs() < 1e-9 {
            break;
        }
        let ns = s - (bx(s) - x) / d;
        if !(0.0..=1.0).contains(&ns) {
            break;
        }
        s = ns;
    }
    if (bx(s) - x).abs() > 1e-12 {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            s = (lo + hi) / 2.0;
            if bx(s) < x { lo = s } else { hi = s }
        }
    }
    s
}

pub(crate) fn cubic_value(a: f64, b: f64, c: f64, d: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
}
