//! Applies an item's matte refinement strokes over a coverage plane.
use super::matte::{CELLS, SIDE};
use crate::{Error, Result};
use project::{MatteStroke, MatteStrokeMode};

/// Paints `strokes` in order over `plane`, the coverage `matte::feathered` returned for a frame of
/// `frame` pixels. An add stroke raises coverage to 1 under its tip, a remove stroke lowers it to
/// 0, and the tip edge is antialiased over one cell, so a later stroke paints over an earlier one.
/// Stroke points are in decoded-frame units, the units of the stretched plane, and the tip is a
/// circle in frame pixels whose radius is a fraction of the shorter side.
pub fn refine(plane: &mut [f32], frame: [u32; 2], strokes: &[&MatteStroke]) -> Result<()> {
    if plane.len() != CELLS || frame.contains(&0) {
        return Err(Error::Invalid("person matte refinement"));
    }
    let [width, height] = frame.map(|side| side as f32);
    // One cell's pixel size on each axis; the antialiased edge is one cell wide.
    let (cell_x, cell_y) = (width / SIDE as f32, height / SIDE as f32);
    let soft = cell_x.max(cell_y);
    let mut tip = Vec::new();
    for stroke in strokes {
        if !stroke.valid() {
            return Err(Error::Invalid("person matte refinement"));
        }
        let radius = stroke.radius * width.min(height);
        let reach = radius + soft;
        tip.clear();
        tip.resize(CELLS, 0.0_f32);
        let points: Vec<[f32; 2]> = stroke
            .points
            .iter()
            .map(|p| [p[0] * width, p[1] * height])
            .collect();
        // A lone point is a segment of length zero.
        let segments = points.len().max(2) - 1;
        for index in 0..segments {
            let a = points.get(index).copied().unwrap_or_default();
            let b = points.get(index + 1).copied().unwrap_or(a);
            let first_x = cells_below((a[0].min(b[0]) - reach) / cell_x);
            let last_x = cells_above((a[0].max(b[0]) + reach) / cell_x);
            let first_y = cells_below((a[1].min(b[1]) - reach) / cell_y);
            let last_y = cells_above((a[1].max(b[1]) + reach) / cell_y);
            for row in first_y..=last_y {
                for column in first_x..=last_x {
                    let centre = [(column as f32 + 0.5) * cell_x, (row as f32 + 0.5) * cell_y];
                    let distance = distance_to_segment(centre, a, b);
                    let weight = ((radius - distance) / soft + 0.5).clamp(0.0, 1.0);
                    if let Some(cell) = tip.get_mut(row * SIDE + column)
                        && weight > *cell
                    {
                        *cell = weight;
                    }
                }
            }
        }
        for (cell, weight) in plane.iter_mut().zip(&tip) {
            *cell = match stroke.mode {
                MatteStrokeMode::Add => *cell + (1.0 - *cell) * weight,
                MatteStrokeMode::Remove => *cell * (1.0 - weight),
            };
        }
    }
    Ok(())
}

fn cells_below(value: f32) -> usize {
    value.floor().clamp(0.0, (SIDE - 1) as f32) as usize
}
fn cells_above(value: f32) -> usize {
    value.ceil().clamp(0.0, (SIDE - 1) as f32) as usize
}

fn distance_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let along = if length > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let nearest = [a[0] + ab[0] * along, a[1] + ab[1] * along];
    ((p[0] - nearest[0]).powi(2) + (p[1] - nearest[1]).powi(2)).sqrt()
}
