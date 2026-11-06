//! Apply item transforms in canvas space after raster/effect/mask evaluation.
use crate::{ExecutionHooks, Image, Result, check};
use project::{Item, TransformField as F};
use time::Tick;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Transform {
    x: f64,
    y: f64,
    sx: f64,
    sy: f64,
    radians: f64,
}
impl Transform {
    pub(crate) fn at(item: &Item, at: Tick) -> Result<Self> {
        Ok(Self {
            x: F::PositionX.value_at(&item.animations, at)?,
            y: F::PositionY.value_at(&item.animations, at)?,
            sx: F::ScaleX.value_at(&item.animations, at)?,
            sy: F::ScaleY.value_at(&item.animations, at)?,
            radians: F::Rotate.value_at(&item.animations, at)?.to_radians(),
        })
    }
    pub(crate) fn active(&self) -> bool {
        self.x != 0.0 || self.y != 0.0 || self.sx != 1.0 || self.sy != 1.0 || self.radians != 0.0
    }
    /// Forward mapping in project canvas pixels, around the placement centre.
    pub(super) fn point(&self, point: [f64; 2], center: [f64; 2]) -> [f64; 2] {
        let (sin, cos) = self.radians.sin_cos();
        let dx = (point[0] - center[0]) * self.sx;
        let dy = (point[1] - center[1]) * self.sy;
        [
            center[0] + self.x + dx * cos - dy * sin,
            center[1] + self.y + dx * sin + dy * cos,
        ]
    }
    pub(super) fn unrotate(&self, point: [f64; 2], center: [f64; 2]) -> [f64; 2] {
        let (sin, cos) = self.radians.sin_cos();
        let dx = point[0] - center[0] - self.x;
        let dy = point[1] - center[1] - self.y;
        [dx * cos + dy * sin, -dx * sin + dy * cos]
    }
    pub(super) fn inverse(&self, point: [f64; 2], center: [f64; 2]) -> [f64; 2] {
        let [x, y] = self.unrotate(point, center);
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
        let [cx, cy] = center;
        let cx = f64::from(image.width) * cx;
        let cy = f64::from(image.height) * cy;
        let tx = self.x * f64::from(image.width) / f64::from(canvas.width);
        let ty = self.y * f64::from(image.height) / f64::from(canvas.height);
        let (sin, cos) = self.radians.sin_cos();
        for (y, row) in out.pixels.chunks_mut(image.width as usize).enumerate() {
            check(hooks)?;
            for (x, pixel) in row.iter_mut().enumerate() {
                let dx = x as f64 + 0.5 - cx - tx;
                let dy = y as f64 + 0.5 - cy - ty;
                let sx = (dx * cos + dy * sin) / self.sx + cx - 0.5;
                let sy = (-dx * sin + dy * cos) / self.sy + cy - 0.5;
                *pixel = crate::image::sample(image, sx as f32, sy as f32);
            }
        }
        Ok(out)
    }
}
