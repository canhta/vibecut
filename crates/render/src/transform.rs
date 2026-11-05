//! Apply item transforms in canvas space after raster/effect/mask evaluation.
use crate::{ExecutionHooks, Image, Result, check};
use project::{Item, TransformField as F};
use time::Tick;

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
