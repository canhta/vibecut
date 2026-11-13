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
    /// Canvas-axis-aligned envelope of the transformed box's four corners.
    pub(super) fn bounds(&self, bounds: super::Rect) -> super::Rect {
        let center = [
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ];
        let corners = [
            [bounds.x, bounds.y],
            [bounds.x + bounds.width, bounds.y],
            [bounds.x + bounds.width, bounds.y + bounds.height],
            [bounds.x, bounds.y + bounds.height],
        ]
        .map(|corner| self.point(corner, center));
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in corners {
            min = [min[0].min(point[0]), min[1].min(point[1])];
            max = [max[0].max(point[0]), max[1].max(point[1])];
        }
        super::Rect {
            x: min[0],
            y: min[1],
            width: max[0] - min[0],
            height: max[1] - min[1],
        }
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

/// Edit-time canvas envelopes for item alignment or even spacing.
pub mod arrange {
    use super::super::Rect;
    use super::super::viewport::ItemGeometry;
    use super::Transform;
    use crate::{Error, Result};
    use project::{Item, ProjectData};
    use time::Tick;

    /// The saved geometric box of one item at output `at`, as an axis-aligned
    /// envelope in project canvas pixels.
    ///
    /// The transform is sampled at `clamp(at - start, 0, duration)` so a selection
    /// of items that do not overlap in time still shares one clock. The box is the
    /// canonical placement saved in the project: the fit-mode box of a video or image
    /// item (`project::fit_box`), the full canvas for overlay items, the text box
    /// around its saved position, and the centred
    /// graphic author square. Transient render placements are not consulted, and
    /// alpha, glyph, effect and mask coverage never tighten the box.
    ///
    /// # Errors
    ///
    /// Refuses a negative time, audio or caption items, a non-finite or empty
    /// destination, an unrepresentable transform and a non-finite envelope.
    pub fn bounds(data: &ProjectData, item: &Item, at: Tick) -> Result<Rect> {
        if at < Tick::ZERO {
            return Err(Error::Invalid("negative arrangement time"));
        }
        let local = at.saturating_sub(item.start).min(
            item.duration()
                .map_err(|_| Error::Invalid("item duration"))?,
        );
        let placement = super::super::evaluation::saved_placement(data, item)
            .ok_or(Error::Unsupported("arrangement on unsupported item"))?;
        let destination = placement.destination;
        if [
            destination.x,
            destination.y,
            destination.width,
            destination.height,
        ]
        .iter()
        .any(|value| !value.is_finite())
            || destination.width <= 0.0
            || destination.height <= 0.0
        {
            return Err(Error::Invalid("item destination"));
        }
        let bounds = Rect {
            x: destination.x * f64::from(data.canvas.width),
            y: destination.y * f64::from(data.canvas.height),
            width: destination.width * f64::from(data.canvas.width),
            height: destination.height * f64::from(data.canvas.height),
        };
        let envelope =
            ItemGeometry::from_parts(bounds, Transform::at(item, local)?).transformed_bounds();
        if [envelope.x, envelope.y, envelope.width, envelope.height]
            .iter()
            .any(|value| !value.is_finite())
            || envelope.width <= 0.0
            || envelope.height <= 0.0
        {
            return Err(Error::Invalid("item arrangement geometry"));
        }
        Ok(envelope)
    }

    /// Whether a video clip, transformed at output time `at`, covers every canvas
    /// pixel. The axis-aligned envelope must reach all four canvas edges and the
    /// rotation must be a whole number of quarter turns, so no corner is left open.
    ///
    /// # Errors
    ///
    /// Refuses what [`bounds`] refuses.
    pub fn covers_canvas(data: &ProjectData, item: &Item, at: Tick) -> Result<bool> {
        let local = at.saturating_sub(item.start).min(
            item.duration()
                .map_err(|_| Error::Invalid("item duration"))?,
        );
        let turns = Transform::at(item, local)?.radians / std::f64::consts::FRAC_PI_2;
        if (turns - turns.round()).abs() > 1e-9 {
            return Ok(false);
        }
        let envelope = bounds(data, item, at)?;
        let width = f64::from(data.canvas.width);
        let height = f64::from(data.canvas.height);
        const EPS: f64 = 1e-6;
        Ok(envelope.x <= EPS
            && envelope.y <= EPS
            && envelope.x + envelope.width >= width - EPS
            && envelope.y + envelope.height >= height - EPS)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use project::{
            Animations, BuiltinPath, Color, ItemId, ItemParams, OverlayParams, Param, ParamValue,
            TextParams,
        };
        use time::SourceRange;

        fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
            Rect {
                x,
                y,
                width,
                height,
            }
        }

        fn item(content: ItemParams, animations: Animations) -> Item {
            Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations,
                id: ItemId(1),
                source_asset: None,
                source_range: SourceRange::new(Tick::ZERO, Tick(1)).unwrap(),
                start: Tick::ZERO,
                content,
            }
        }

        fn transformed(animations: &[(BuiltinPath, f64)]) -> Animations {
            let mut params = Animations::default();
            for (path, value) in animations {
                let value = if path.is_flip() {
                    ParamValue::Bool(true)
                } else {
                    ParamValue::Scalar(*value)
                };
                params
                    .set((*path).into(), Param::new(value).unwrap())
                    .unwrap();
            }
            params
        }

        fn close(actual: Rect, expected: Rect) -> bool {
            (actual.x - expected.x).abs() < 1e-9
                && (actual.y - expected.y).abs() < 1e-9
                && (actual.width - expected.width).abs() < 1e-9
                && (actual.height - expected.height).abs() < 1e-9
        }

        #[test]
        fn envelope_matches_the_placed_box_after_scale_reflection_and_quarter_turn() {
            // Reference example: destination (40,30,80,20), translate (10,-5),
            // scale (.5,2), 90 degrees gives the envelope (70,15,40,40).
            let boxed = item(
                ItemParams::Text(TextParams::new("Box")),
                transformed(&[
                    (BuiltinPath::TransformPositionX, 10.0),
                    (BuiltinPath::TransformPositionY, -5.0),
                    (BuiltinPath::TransformScaleX, 0.5),
                    (BuiltinPath::TransformScaleY, 2.0),
                    (BuiltinPath::TransformRotate, 90.0),
                ]),
            );
            let envelope = Transform::at(&boxed, Tick::ZERO)
                .unwrap()
                .bounds(rect(40.0, 30.0, 80.0, 20.0));
            assert!(
                close(envelope, rect(70.0, 15.0, 40.0, 40.0)),
                "{envelope:?}"
            );

            let reflected = item(
                ItemParams::Overlay(OverlayParams {
                    color: Color([255; 4]),
                }),
                transformed(&[
                    (BuiltinPath::TransformFlipX, 1.0),
                    (BuiltinPath::TransformFlipY, 1.0),
                ]),
            );
            let plain = rect(10.0, 20.0, 40.0, 30.0);
            assert!(
                close(
                    Transform::at(&reflected, Tick::ZERO).unwrap().bounds(plain),
                    plain
                ),
                "a centre reflection permutes corners without changing the envelope"
            );
        }

        #[test]
        fn bounds_use_the_saved_placement_and_clamp_the_clock() {
            let mut data = project::Project::empty(project::ProjectId(1), "Arrange".into())
                .unwrap()
                .into_data();
            data.canvas.width = 200;
            data.canvas.height = 100;
            let mut text = item(
                ItemParams::Text(TextParams::new("Box")),
                transformed(&[
                    (BuiltinPath::TransformPositionX, 10.0),
                    (BuiltinPath::TransformPositionY, -5.0),
                    (BuiltinPath::TransformScaleX, 0.5),
                    (BuiltinPath::TransformScaleY, 2.0),
                    (BuiltinPath::TransformRotate, 90.0),
                ]),
            );
            text.start = Tick(100);
            text.content = ItemParams::Text(TextParams::new("Box"));
            assert!(
                close(
                    bounds(&data, &text, Tick::ZERO).unwrap(),
                    rect(85.0, 5.0, 50.0, 80.0)
                ),
                "before the item starts, its first transform sample is used"
            );
            // Transform (0, 0) is the canvas centre for text as for every kind: the 160 x 25
            // box is centred on the 200 x 100 canvas.
            let centred = item(
                ItemParams::Text(TextParams::new("Box")),
                transformed(&[
                    (BuiltinPath::TransformPositionX, 0.0),
                    (BuiltinPath::TransformPositionY, 0.0),
                ]),
            );
            assert!(
                close(
                    bounds(&data, &centred, Tick::ZERO).unwrap(),
                    rect(20.0, 37.5, 160.0, 25.0)
                ),
                "text with a zero Transform sits at the canvas centre"
            );
            let audio = item(
                ItemParams::Audio(project::AudioParams {
                    volume: 0.0,
                    muted: false,
                    source: Default::default(),
                    fades: Default::default(),
                    cleanup: Default::default(),
                }),
                Animations::default(),
            );
            assert!(matches!(
                bounds(&data, &audio, Tick::ZERO),
                Err(Error::Unsupported("arrangement on unsupported item"))
            ));
        }
    }
}
