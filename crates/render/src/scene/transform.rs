//! Apply item transforms in canvas space after raster/effect/mask evaluation.
use crate::{ExecutionHooks, Image, Result, check};
use project::{FlipAxis, Item, TransformField as F};
use time::Tick;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Transform {
    x: f64,
    y: f64,
    sx: f64,
    sy: f64,
    radians: f64,
    /// `-1` reflects that local axis about the placement centre; `1` leaves it.
    flip_x: f64,
    flip_y: f64,
}
impl Transform {
    pub(crate) fn at(item: &Item, at: Tick) -> Result<Self> {
        let axis = |axis: FlipAxis| -> Result<f64> {
            Ok(if axis.value_at(&item.animations, at)? {
                -1.0
            } else {
                1.0
            })
        };
        Ok(Self {
            x: F::PositionX.value_at(&item.animations, at)?,
            y: F::PositionY.value_at(&item.animations, at)?,
            sx: F::ScaleX.value_at(&item.animations, at)?,
            sy: F::ScaleY.value_at(&item.animations, at)?,
            radians: F::Rotate.value_at(&item.animations, at)?.to_radians(),
            flip_x: axis(FlipAxis::Horizontal)?,
            flip_y: axis(FlipAxis::Vertical)?,
        })
    }
    pub(crate) fn active(&self) -> bool {
        self.x != 0.0
            || self.y != 0.0
            || self.sx != 1.0
            || self.sy != 1.0
            || self.radians != 0.0
            || self.flip_x != 1.0
            || self.flip_y != 1.0
    }
    /// Forward mapping in project canvas pixels, around the placement centre:
    /// `p' = c + t + R F S (p - c)`, where `F` reflects the flipped axes.
    pub(super) fn point(&self, point: [f64; 2], center: [f64; 2]) -> [f64; 2] {
        let (sin, cos) = self.radians.sin_cos();
        let dx = (point[0] - center[0]) * self.sx * self.flip_x;
        let dy = (point[1] - center[1]) * self.sy * self.flip_y;
        [
            center[0] + self.x + dx * cos - dy * sin,
            center[1] + self.y + dx * sin + dy * cos,
        ]
    }
    /// `F R^-1 (p' - c - t)`: the rotated inverse with the reflection removed and
    /// scale still applied, which is the local axis a resize handle reads.
    pub(super) fn local_axis(&self, point: [f64; 2], center: [f64; 2]) -> [f64; 2] {
        let (sin, cos) = self.radians.sin_cos();
        let dx = point[0] - center[0] - self.x;
        let dy = point[1] - center[1] - self.y;
        [
            (dx * cos + dy * sin) * self.flip_x,
            (-dx * sin + dy * cos) * self.flip_y,
        ]
    }
    /// `p = c + S^-1 F R^-1 (p' - c - t)`.
    pub(super) fn inverse(&self, point: [f64; 2], center: [f64; 2]) -> [f64; 2] {
        let [x, y] = self.local_axis(point, center);
        [x / self.sx + center[0], y / self.sy + center[1]]
    }
    pub(super) fn scale(&self) -> [f64; 2] {
        [self.sx, self.sy]
    }
    pub(crate) fn apply(
        &self,
        image: &Image,
        canvas: &project::Canvas,
        center: [f64; 2],
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        let mut out = Image::new(image.width, image.height, [0.0; 4], hooks)?;
        // Output pixel gains, so unequal output aspect composes in project space.
        let gain_x = f64::from(image.width) / f64::from(canvas.width);
        let gain_y = f64::from(image.height) / f64::from(canvas.height);
        let cx = center[0] * f64::from(canvas.width);
        let cy = center[1] * f64::from(canvas.height);
        let out_x = cx * gain_x;
        let out_y = cy * gain_y;
        let tx = self.x * gain_x;
        let ty = self.y * gain_y;
        let (sin, cos) = self.radians.sin_cos();
        for (y, row) in out.pixels.chunks_mut(image.width as usize).enumerate() {
            check(hooks)?;
            for (x, pixel) in row.iter_mut().enumerate() {
                let dx = (x as f64 + 0.5 - out_x - tx) / gain_x;
                let dy = (y as f64 + 0.5 - out_y - ty) / gain_y;
                let rx = (dx * cos + dy * sin) * self.flip_x / self.sx;
                let ry = (-dx * sin + dy * cos) * self.flip_y / self.sy;
                *pixel = crate::image::sample(
                    image,
                    ((cx + rx) * gain_x - 0.5) as f32,
                    ((cy + ry) * gain_y - 0.5) as f32,
                );
            }
        }
        Ok(out)
    }
}
