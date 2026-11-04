//! Anti-aliased path rasterizer for glyph outlines and boxes.
//!
//! Paths are flattened to closed polylines. Each pixel row is sampled by [`SUB`] sub-scanlines;
//! an active-edge list accumulates non-zero winding spans with exact horizontal coverage at the
//! span ends. Coverage is linear in area.

/// Vertical sub-samples per pixel row.
pub const SUB: usize = 5;

/// Largest mask allocation accepted by [`Mask::try_new`], the same bound as the renderer's canvas.
pub(crate) const MAX_MASK_PIXELS: usize = 64 << 20;

/// A 2×3 affine transform `(x, y) → (a x + c y + e, b x + d y + f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Xform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Xform {
    /// Apply this transform to a point.
    pub(crate) fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

/// A flattened path: closed contours of points in pixel space, y down.
#[derive(Clone, Debug, Default)]
pub(crate) struct Path {
    pub contours: Vec<Vec<(f32, f32)>>,
    current: (f32, f32),
    open: bool,
    pub detail: f32,
}

impl Path {
    pub fn new() -> Self {
        Self {
            detail: 1.0,
            ..Self::default()
        }
    }

    fn tolerance(&self) -> f32 {
        if self.detail > 0.0 { self.detail } else { 1.0 }
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.contours.push(vec![(x, y)]);
        self.current = (x, y);
        self.open = true;
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        if !self.open {
            self.move_to(self.current.0, self.current.1);
        }
        if (x, y) != self.current
            && let Some(contour) = self.contours.last_mut()
        {
            contour.push((x, y));
        }
        self.current = (x, y);
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (x0, y0) = self.current;
        let delta =
            ((x0 - 2.0 * cx + x).abs() + (y0 - 2.0 * cy + y).abs()).max(0.01) * self.tolerance();
        let steps = ((delta * 2.0).sqrt().ceil() as usize).clamp(1, 64);
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            self.line_to(
                u * u * x0 + 2.0 * u * t * cx + t * t * x,
                u * u * y0 + 2.0 * u * t * cy + t * t * y,
            );
        }
    }

    pub fn cubic_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let (x0, y0) = self.current;
        let delta = ((x0 - 2.0 * c1x + c2x).abs()
            + (y0 - 2.0 * c1y + c2y).abs()
            + (c1x - 2.0 * c2x + x).abs()
            + (c1y - 2.0 * c2y + y).abs())
        .max(0.01)
            * self.tolerance();
        let steps = ((delta * 3.0).sqrt().ceil() as usize).clamp(1, 96);
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line_to(
                a * x0 + b * c1x + c * c2x + d * x,
                a * y0 + b * c1y + c * c2y + d * y,
            );
        }
    }

    /// Close the current contour; contours are always filled as closed polygons.
    pub fn close(&mut self) {
        if self.open {
            if let Some(first) = self.contours.last().and_then(|c| c.first()).copied() {
                self.current = first;
            }
            if self.contours.last().is_some_and(|c| c.len() < 2) {
                self.contours.pop();
            }
        }
        self.open = false;
    }

    /// Bounding box `(x0, y0, x1, y1)` of all points.
    #[cfg(test)]
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut points = self.contours.iter().flatten();
        let first = points.next()?;
        let mut bounds = (first.0, first.1, first.0, first.1);
        for p in points {
            bounds = (
                bounds.0.min(p.0),
                bounds.1.min(p.1),
                bounds.2.max(p.0),
                bounds.3.max(p.1),
            );
        }
        Some(bounds)
    }

    /// Signed area (shoelace, y down: positive is clockwise on screen) of all contours.
    pub fn signed_area(&self) -> f32 {
        self.contours.iter().map(|c| contour_area(c)).sum()
    }

    /// Grow (`d > 0`) or shrink every contour by `d` pixels along its normals. Holes shrink
    /// because they wind the other way.
    pub fn embolden(&mut self, d: f32) {
        if d == 0.0 {
            return;
        }
        let sign = if self.signed_area() >= 0.0 { 1.0 } else { -1.0 };
        for contour in &mut self.contours {
            let n = contour.len();
            if n < 3 {
                continue;
            }
            let source = contour.clone();
            let mut grown = Vec::with_capacity(n);
            for (i, point) in source.iter().enumerate() {
                let previous = if i == 0 {
                    source.last().copied().unwrap_or(*point)
                } else {
                    source.get(i - 1).copied().unwrap_or(*point)
                };
                let next = source.get((i + 1) % n).copied().unwrap_or(*point);
                let n1 = normal(previous, *point, sign);
                let n2 = normal(*point, next, sign);
                let (mx, my) = (n1.0 + n2.0, n1.1 + n2.1);
                let length = (mx * mx + my * my).sqrt();
                let grown_point = if length < 1e-6 {
                    (point.0 + n1.0 * d, point.1 + n1.1 * d)
                } else {
                    let (mx, my) = (mx / length, my / length);
                    let cosine = (mx * n1.0 + my * n1.1).max(0.35);
                    (point.0 + mx * d / cosine, point.1 + my * d / cosine)
                };
                grown.push(grown_point);
            }
            *contour = grown;
        }
    }

    /// Axis-aligned rectangle wound clockwise on screen.
    pub fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Path {
        let mut path = Path::new();
        path.move_to(x0, y0);
        path.line_to(x1, y0);
        path.line_to(x1, y1);
        path.line_to(x0, y1);
        path.close();
        path
    }

    /// Rectangle with rounded corners of radius `r`, clamped to half the shorter side.
    pub fn round_rect(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Path {
        let r = r
            .min((x1 - x0).abs() / 2.0)
            .min((y1 - y0).abs() / 2.0)
            .max(0.0);
        if r < 0.01 {
            return Path::rect(x0, y0, x1, y1);
        }
        let k = 0.552_284_8 * r;
        let mut path = Path::new();
        path.move_to(x0 + r, y0);
        path.line_to(x1 - r, y0);
        path.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        path.line_to(x1, y1 - r);
        path.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        path.line_to(x0 + r, y1);
        path.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        path.line_to(x0, y0 + r);
        path.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        path.close();
        path
    }
}

fn contour_area(contour: &[(f32, f32)]) -> f32 {
    let mut area = 0.0;
    for (a, b) in contour.iter().zip(contour.iter().cycle().skip(1)) {
        area += a.0 * b.1 - b.0 * a.1;
    }
    area / 2.0
}

/// Outward unit normal of edge `p0 → p1` for an outline of orientation `sign`.
fn normal(p0: (f32, f32), p1: (f32, f32), sign: f32) -> (f32, f32) {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1e-9 {
        return (0.0, 0.0);
    }
    (sign * dy / length, -sign * dx / length)
}

/// Coverage mask in `0..=1`, row-major.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Mask {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
}

impl Mask {
    /// Allocate a mask when the dimensions fit the documented cap.
    pub(crate) fn try_new(w: usize, h: usize) -> Option<Self> {
        let pixels = w.checked_mul(h)?;
        (pixels <= MAX_MASK_PIXELS).then(|| Self {
            w,
            h,
            a: vec![0.0; pixels],
        })
    }

    /// Allocate a mask for an already validated canvas.
    pub(crate) fn new(w: usize, h: usize) -> Self {
        Self::try_new(w, h).unwrap_or(Self {
            w: 0,
            h: 0,
            a: Vec::new(),
        })
    }

    /// Coverage at a pixel; out-of-range coordinates read as zero.
    #[cfg(test)]
    pub(crate) fn get(&self, x: usize, y: usize) -> f32 {
        y.checked_mul(self.w)
            .and_then(|row| row.checked_add(x))
            .and_then(|i| self.a.get(i))
            .copied()
            .unwrap_or(0.0)
    }

    /// Add `src` placed at `(ox, oy)`, saturating at 1. Offsets use wide arithmetic so the full
    /// `i32` range is safe.
    #[cfg(test)]
    pub(crate) fn add(&mut self, src: &Mask, ox: i32, oy: i32) {
        let ox = i64::from(ox);
        let oy = i64::from(oy);
        for y in 0..src.h {
            let ty = oy + y as i64;
            if ty < 0 || ty >= self.h as i64 {
                continue;
            }
            let x0 = (-ox).max(0) as usize;
            let x1 = src
                .w
                .min((self.w as i64 - ox).clamp(0, i64::from(u32::MAX)) as usize);
            if x0 >= x1 {
                continue;
            }
            let Some(src_row) = src.a.get(y * src.w + x0..y * src.w + x1) else {
                continue;
            };
            let dest_x = ox + x0 as i64;
            if dest_x < 0 {
                continue;
            }
            let Some(d0) = (ty as usize)
                .checked_mul(self.w)
                .and_then(|row| row.checked_add(dest_x as usize))
            else {
                continue;
            };
            let Some(dst_row) = self.a.get_mut(d0..d0 + (x1 - x0)) else {
                continue;
            };
            for (dst, source) in dst_row.iter_mut().zip(src_row) {
                *dst = (*dst + source).min(1.0);
            }
        }
    }
}

/// Fill `path` with the non-zero winding rule into a `w`×`h` mask.
#[cfg(test)]
pub(crate) fn fill(path: &Path, w: usize, h: usize, dx: f32, dy: f32) -> Mask {
    let mut mask = Mask::new(w, h);
    fill_into(path, &mut mask, dx, dy);
    mask
}

struct Edge {
    ytop: f32,
    ybot: f32,
    x: f32,
    slope: f32,
    direction: i32,
}

/// Fill `path` into `mask`, adding coverage (saturating at 1).
pub(crate) fn fill_into(path: &Path, mask: &mut Mask, dx: f32, dy: f32) {
    let (w, h) = (mask.w, mask.h);
    if w == 0 || h == 0 {
        return;
    }
    let mut edges: Vec<Edge> = Vec::new();
    for contour in &path.contours {
        if contour.len() < 2 {
            continue;
        }
        for (a, b) in contour.iter().zip(contour.iter().cycle().skip(1)) {
            let (x0, y0) = (a.0 + dx, a.1 + dy);
            let (x1, y1) = (b.0 + dx, b.1 + dy);
            if y0 == y1 || !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
                continue;
            }
            let (direction, xa, ya, xb, yb) = if y0 < y1 {
                (1, x0, y0, x1, y1)
            } else {
                (-1, x1, y1, x0, y0)
            };
            if yb <= 0.0 || ya >= h as f32 {
                continue;
            }
            edges.push(Edge {
                ytop: ya,
                ybot: yb,
                x: xa,
                slope: (xb - xa) / (yb - ya),
                direction,
            });
        }
    }
    if edges.is_empty() {
        return;
    }
    edges.sort_by(|a, b| a.ytop.total_cmp(&b.ytop));
    let ymin = edges
        .first()
        .map(|e| e.ytop.floor().max(0.0) as usize)
        .unwrap_or(0);
    let ymax = (edges
        .iter()
        .map(|e| e.ybot)
        .fold(f32::MIN, f32::max)
        .ceil()
        .max(0.0) as usize)
        .min(h);
    let mut next = 0usize;
    let mut active: Vec<usize> = Vec::new();
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    let mut row = vec![0.0f32; w.saturating_add(1)];
    let weight = 1.0 / SUB as f32;
    for y in ymin..ymax {
        let mut low = usize::MAX;
        let mut high = 0usize;
        for k in 0..SUB {
            let sample_y = y as f32 + (k as f32 + 0.5) / SUB as f32;
            while next < edges.len() && edges.get(next).is_some_and(|e| e.ytop <= sample_y) {
                active.push(next);
                next += 1;
            }
            active.retain(|&i| edges.get(i).is_some_and(|e| e.ybot > sample_y));
            crossings.clear();
            for &i in &active {
                if let Some(edge) = edges.get(i)
                    && edge.ytop <= sample_y
                {
                    crossings.push((edge.x + (sample_y - edge.ytop) * edge.slope, edge.direction));
                }
            }
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            let mut span_start = 0.0f32;
            for &(x, direction) in &crossings {
                let before = winding;
                winding += direction;
                if before == 0 && winding != 0 {
                    span_start = x;
                } else if before != 0
                    && winding == 0
                    && let Some((a, b)) = add_span(&mut row, span_start, x, weight, w)
                {
                    low = low.min(a);
                    high = high.max(b);
                }
            }
        }
        if low <= high && low < w {
            let high = high.min(w - 1);
            if let Some(out) = mask.a.get_mut(y * w..(y + 1) * w) {
                for x in low..=high {
                    if let (Some(pixel), Some(extra)) = (out.get_mut(x), row.get(x)) {
                        *pixel = (*pixel + extra).min(1.0);
                    }
                }
            }
            for value in row.iter_mut().skip(low).take(high - low + 1) {
                *value = 0.0;
            }
            if let Some(last) = row.get_mut(w) {
                *last = 0.0;
            }
        }
    }
}

/// Add coverage `weight` over `[x0, x1)` with fractional ends; returns the touched pixel range.
fn add_span(row: &mut [f32], x0: f32, x1: f32, weight: f32, w: usize) -> Option<(usize, usize)> {
    let x0 = x0.clamp(0.0, w as f32);
    let x1 = x1.clamp(0.0, w as f32);
    if x1 <= x0 {
        return None;
    }
    let i0 = x0.floor() as usize;
    let i1 = x1.floor() as usize;
    if i0 == i1 {
        if let Some(value) = row.get_mut(i0) {
            *value += (x1 - x0) * weight;
        }
        return Some((i0, i0));
    }
    if let Some(value) = row.get_mut(i0) {
        *value += (i0 as f32 + 1.0 - x0) * weight;
    }
    if let Some(interior) = row.get_mut(i0 + 1..i1) {
        for value in interior {
            *value += weight;
        }
    }
    if i1 < w
        && let Some(value) = row.get_mut(i1)
    {
        *value += (x1 - i1 as f32) * weight;
    }
    Some((i0, i1.min(w.saturating_sub(1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_coverage_is_exact() {
        let mask = fill(&Path::rect(1.5, 2.0, 4.0, 5.0), 6, 6, 0.0, 0.0);
        assert!((mask.get(1, 3) - 0.5).abs() < 1e-5);
        assert!((mask.get(2, 3) - 1.0).abs() < 1e-5);
        assert_eq!(mask.get(4, 3), 0.0);
        assert_eq!(mask.get(2, 1), 0.0);
        assert_eq!(mask.get(2, 5), 0.0);
        let total: f32 = mask.a.iter().sum();
        assert!((total - 7.5).abs() < 1e-3, "{total}");
    }

    #[test]
    fn opposite_winding_cuts_a_hole() {
        let mut path = Path::rect(0.0, 0.0, 10.0, 10.0);
        path.move_to(3.0, 3.0);
        path.line_to(3.0, 7.0);
        path.line_to(7.0, 7.0);
        path.line_to(7.0, 3.0);
        path.close();
        let mask = fill(&path, 10, 10, 0.0, 0.0);
        assert_eq!(mask.get(5, 5), 0.0);
        assert!((mask.get(1, 5) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn embolden_grows_outlines_and_shrinks_holes() {
        let mut path = Path::rect(4.0, 4.0, 14.0, 14.0);
        path.embolden(1.0);
        let bounds = path.bounds().expect("nonempty path");
        assert!(
            (bounds.0 - 3.0).abs() < 1e-4 && (bounds.2 - 15.0).abs() < 1e-4,
            "{bounds:?}"
        );
    }

    #[test]
    fn oversized_mask_dimensions_are_rejected() {
        assert!(Mask::try_new(usize::MAX, 2).is_none());
        assert!(Mask::try_new(MAX_MASK_PIXELS + 1, 1).is_none());
        assert!(Mask::try_new(4, 4).is_some());
    }

    #[test]
    fn extreme_add_offsets_do_not_overflow() {
        let mut mask = Mask::new(2, 2);
        let src = Mask::new(2, 2);
        mask.add(&src, i32::MIN, i32::MIN);
        mask.add(&src, i32::MAX, i32::MAX);
        mask.add(&src, i32::MIN, i32::MAX);
        assert_eq!(mask.a.len(), 4);
    }
}
