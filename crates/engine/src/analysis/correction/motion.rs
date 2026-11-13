//! Measures the camera motion between consecutive frames of one clip.
use render::stabilize::Pose;

/// Analysis grid width; a narrower source keeps its own width.
const GRID_WIDTH: usize = 480;
/// Block edge, in grid pixels.
const BLOCK: usize = 32;
/// Search radius, in grid pixels; 8 is 32 pixels at 1080p.
const RANGE: i32 = 8;
const SEARCH: usize = 2 * RANGE as usize + 1;
/// Blocks keep this far from the grid edge so every candidate lies inside the frame.
const MARGIN: usize = RANGE as usize + 4;
/// Fewer surviving blocks than this leave the frame unmeasured (identity step).
const MIN_BLOCKS: usize = 6;

/// A grid-scale luma frame stream measured against its predecessor: block matching with a
/// sub-pixel parabola, then a robust similarity fit. All arithmetic is integer SAD or `f64` in a
/// fixed order, so the same frames always give the same poses.
pub(super) struct Motion {
    source: (usize, usize),
    width: usize,
    height: usize,
    blocks: Vec<(usize, usize)>,
    previous: Vec<u8>,
    pose: Pose,
}
impl Motion {
    /// A stream of `width` x `height` frames, or `None` when the grid is too small to hold one
    /// block. Fewer than `MIN_BLOCKS` blocks never fit a step, so such a stream stays at identity.
    pub(super) fn new(width: u32, height: u32) -> Option<Self> {
        let (source_width, source_height) = (width as usize, height as usize);
        if source_width == 0 || source_height == 0 {
            return None;
        }
        let grid_width = GRID_WIDTH.min(source_width);
        let grid_height = ((grid_width * source_height + source_width / 2) / source_width).max(1);
        let mut blocks = Vec::new();
        let mut y = MARGIN;
        while y + BLOCK + MARGIN <= grid_height {
            let mut x = MARGIN;
            while x + BLOCK + MARGIN <= grid_width {
                blocks.push((x, y));
                x += BLOCK;
            }
            y += BLOCK;
        }
        (!blocks.is_empty()).then_some(Self {
            source: (source_width, source_height),
            width: grid_width,
            height: grid_height,
            blocks,
            previous: Vec::new(),
            pose: Pose::IDENTITY,
        })
    }
    /// The analysis grid every translation is measured in.
    pub(super) fn grid(&self) -> [u32; 2] {
        [self.width as u32, self.height as u32]
    }
    /// Feed the next frame, packed RGBA8, and get its pose relative to the first frame. `None`
    /// when the frame is not the size this stream started with.
    pub(super) fn push(&mut self, rgba: &[u8], width: u32, height: u32) -> Option<Pose> {
        if (width as usize, height as usize) != self.source {
            return None;
        }
        let current = self.luma(rgba)?;
        if !self.previous.is_empty() {
            let points: Vec<([f64; 2], [f64; 2])> = self
                .blocks
                .iter()
                .filter_map(|&block| {
                    let moved = match_block(&self.previous, &current, self.width, block)?;
                    let at = [
                        (block.0 + BLOCK / 2) as f64 - self.width as f64 / 2.0,
                        (block.1 + BLOCK / 2) as f64 - self.height as f64 / 2.0,
                    ];
                    Some((at, [at[0] + moved[0], at[1] + moved[1]]))
                })
                .collect();
            if let Some(step) = fit(&points) {
                self.pose = self.pose.then(step.a, step.b, step.tx, step.ty);
            }
        }
        self.previous = current;
        Some(self.pose)
    }
    /// Box-average the frame's luma onto the grid with integer arithmetic.
    fn luma(&self, rgba: &[u8]) -> Option<Vec<u8>> {
        let (width, height) = self.source;
        if rgba.len() != width * height * 4 {
            return None;
        }
        let (grid_width, grid_height) = (self.width, self.height);
        let columns: Vec<usize> = (0..width).map(|x| x * grid_width / width).collect();
        let mut sums = vec![0u32; grid_width * grid_height];
        let mut column_counts = vec![0u32; grid_width];
        for &column in &columns {
            *column_counts.get_mut(column)? += 1;
        }
        let mut row_counts = vec![0u32; grid_height];
        for (y, row) in rgba.chunks_exact(width * 4).enumerate() {
            let grid_row = y * grid_height / height;
            *row_counts.get_mut(grid_row)? += 1;
            let target = sums.get_mut(grid_row * grid_width..(grid_row + 1) * grid_width)?;
            for (pixel, &column) in row.chunks_exact(4).zip(&columns) {
                let &[r, g, b, _] = pixel else {
                    return None;
                };
                let luma = (77 * u32::from(r) + 150 * u32::from(g) + 29 * u32::from(b)) >> 8;
                *target.get_mut(column)? += luma;
            }
        }
        Some(
            sums.chunks_exact(grid_width)
                .zip(&row_counts)
                .flat_map(|(row, &rows)| {
                    row.iter()
                        .zip(&column_counts)
                        .map(move |(&sum, &columns)| (sum / (rows * columns).max(1)) as u8)
                })
                .collect(),
        )
    }
}

/// Sum of absolute differences between the block of `previous` at `block` and the block of
/// `current` displaced by `(dx, dy)`.
fn sad(
    previous: &[u8],
    current: &[u8],
    width: usize,
    block: (usize, usize),
    dx: i32,
    dy: i32,
) -> Option<u32> {
    let column = usize::try_from(i32::try_from(block.0).ok()? + dx).ok()?;
    let mut total = 0u32;
    for row in 0..BLOCK {
        let line = usize::try_from(i32::try_from(block.1 + row).ok()? + dy).ok()?;
        let before = previous
            .get((block.1 + row) * width + block.0..)?
            .get(..BLOCK)?;
        let after = current.get(line * width + column..)?.get(..BLOCK)?;
        total += before
            .iter()
            .zip(after)
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum::<u32>();
    }
    Some(total)
}

/// Displacement of one block from `previous` to `current` in grid pixels, with sub-pixel
/// refinement; `None` for a flat block, an ambiguous match or a best match on the search edge.
fn match_block(
    previous: &[u8],
    current: &[u8],
    width: usize,
    block: (usize, usize),
) -> Option<[f64; 2]> {
    let mut grid = [0u32; SEARCH * SEARCH];
    let (mut best, mut best_at) = (u32::MAX, 0usize);
    let mut sum = 0u64;
    for (index, cell) in grid.iter_mut().enumerate() {
        let dx = (index % SEARCH) as i32 - RANGE;
        let dy = (index / SEARCH) as i32 - RANGE;
        let cost = sad(previous, current, width, block, dx, dy)?;
        *cell = cost;
        sum += u64::from(cost);
        if cost < best {
            best = cost;
            best_at = index;
        }
    }
    let mean = sum as f64 / grid.len() as f64;
    if mean < (BLOCK * BLOCK) as f64 * 2.0 || f64::from(best) > mean * 0.6 {
        return None;
    }
    let (ix, iy) = (best_at % SEARCH, best_at / SEARCH);
    if ix == 0 || iy == 0 || ix == SEARCH - 1 || iy == SEARCH - 1 {
        return None;
    }
    let cost = |x: usize, y: usize| grid.get(y * SEARCH + x).map_or(0.0, |v| f64::from(*v));
    let parabola = |minus: f64, centre: f64, plus: f64| {
        let curvature = minus - 2.0 * centre + plus;
        if curvature.abs() < 1e-9 {
            0.0
        } else {
            0.5 * (minus - plus) / curvature
        }
    };
    Some([
        ix as f64 - f64::from(RANGE) + parabola(cost(ix - 1, iy), cost(ix, iy), cost(ix + 1, iy)),
        iy as f64 - f64::from(RANGE) + parabola(cost(ix, iy - 1), cost(ix, iy), cost(ix, iy + 1)),
    ])
}

/// `p' = [[a, -b], [b, a]] p + t`.
struct Step {
    a: f64,
    b: f64,
    tx: f64,
    ty: f64,
}
impl Step {
    fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        [
            self.a * p[0] - self.b * p[1] + self.tx,
            self.b * p[0] + self.a * p[1] + self.ty,
        ]
    }
}

/// The weighted least-squares similarity taking each point to its partner.
fn solve(points: &[([f64; 2], [f64; 2])], weights: &[f64]) -> Option<Step> {
    let total: f64 = weights.iter().sum();
    if total < MIN_BLOCKS as f64 {
        return None;
    }
    let (mut from, mut to) = ([0.0f64; 2], [0.0f64; 2]);
    for ((p, q), w) in points.iter().zip(weights) {
        from[0] += w * p[0];
        from[1] += w * p[1];
        to[0] += w * q[0];
        to[1] += w * q[1];
    }
    from = from.map(|v| v / total);
    to = to.map(|v| v / total);
    let (mut dot, mut cross, mut norm) = (0.0f64, 0.0f64, 0.0f64);
    for ((p, q), w) in points.iter().zip(weights) {
        let (px, py) = (p[0] - from[0], p[1] - from[1]);
        let (qx, qy) = (q[0] - to[0], q[1] - to[1]);
        dot += w * (px * qx + py * qy);
        cross += w * (px * qy - py * qx);
        norm += w * (px * px + py * py);
    }
    if norm < 1e-9 {
        return None;
    }
    let (a, b) = (dot / norm, cross / norm);
    Some(Step {
        a,
        b,
        tx: to[0] - (a * from[0] - b * from[1]),
        ty: to[1] - (b * from[0] + a * from[1]),
    })
}

/// Robust similarity fit: three rounds of dropping points whose residual exceeds three times the
/// median (at least 0.15 px), then a final solve over the survivors.
fn fit(points: &[([f64; 2], [f64; 2])]) -> Option<Step> {
    if points.len() < MIN_BLOCKS {
        return None;
    }
    let mut weights = vec![1.0f64; points.len()];
    let mut result = None;
    for round in 0..4 {
        let step = solve(points, &weights)?;
        if round < 3 {
            let residuals: Vec<f64> = points
                .iter()
                .map(|(p, q)| {
                    let moved = step.apply(*p);
                    ((moved[0] - q[0]).powi(2) + (moved[1] - q[1]).powi(2)).sqrt()
                })
                .collect();
            let mut sorted = residuals.clone();
            sorted.sort_by(f64::total_cmp);
            let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
            let threshold = (median * 3.0).max(0.15);
            for (weight, residual) in weights.iter_mut().zip(&residuals) {
                *weight = if *residual > threshold { 0.0 } else { 1.0 };
            }
        }
        result = Some(step);
    }
    result
}
