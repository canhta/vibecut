//! Maps preview input onto render-owned item geometry.
use super::{Renderer, transform::Transform};
use crate::{Error, Rect, Result};
use project::{ItemId, ItemParams, TransformField};
use time::Tick;

/// Letterboxed canvas in logical screen pixels, with zoom about its centre.
/// Pan is a screen-pixel offset. Conversion remains defined outside the canvas
/// so a captured drag can continue through a letterbox or beyond the viewport.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    screen: Rect,
    canvas: [f64; 2],
    origin: [f64; 2],
    scale: f64,
}
impl Viewport {
    /// Refuses non-finite, empty or numerically unrepresentable geometry.
    pub fn new(screen: Rect, canvas: [u32; 2], zoom: f64, pan: [f64; 2]) -> Result<Self> {
        finite(&[
            screen.x,
            screen.y,
            screen.width,
            screen.height,
            zoom,
            pan[0],
            pan[1],
        ])?;
        if screen.width <= 0.0 || screen.height <= 0.0 || canvas.contains(&0) || zoom <= 0.0 {
            return Err(Error::Invalid("viewport dimensions"));
        }
        let canvas = canvas.map(f64::from);
        let scale = (screen.width / canvas[0]).min(screen.height / canvas[1]) * zoom;
        let origin = [
            screen.x + (screen.width - canvas[0] * scale) / 2.0 + pan[0],
            screen.y + (screen.height - canvas[1] * scale) / 2.0 + pan[1],
        ];
        finite(&[
            scale,
            origin[0],
            origin[1],
            screen.x + screen.width,
            screen.y + screen.height,
        ])?;
        let far = [origin[0] + canvas[0] * scale, origin[1] + canvas[1] * scale];
        finite(&far)?;
        if scale <= 0.0 || !scale.recip().is_finite() || far[0] <= origin[0] || far[1] <= origin[1]
        {
            return Err(Error::Invalid("viewport scale"));
        }
        Ok(Self {
            screen,
            canvas,
            origin,
            scale,
        })
    }
    pub fn screen_to_canvas(&self, point: [f64; 2]) -> Result<[f64; 2]> {
        let result = [
            (point[0] - self.origin[0]) / self.scale,
            (point[1] - self.origin[1]) / self.scale,
        ];
        finite(&result)?;
        Ok(result)
    }
    pub fn canvas_to_screen(&self, point: [f64; 2]) -> Result<[f64; 2]> {
        let result = [
            self.origin[0] + point[0] * self.scale,
            self.origin[1] + point[1] * self.scale,
        ];
        finite(&result)?;
        Ok(result)
    }
    /// Logical screen distance converted to canvas distance (snapping/handle tolerance).
    pub fn canvas_distance(&self, screen_pixels: f64) -> Result<f64> {
        let value = screen_pixels / self.scale;
        finite(&[value])?;
        if screen_pixels < 0.0 {
            return Err(Error::Invalid("viewport distance"));
        }
        Ok(value)
    }
    fn contains(&self, screen: [f64; 2], canvas: [f64; 2]) -> bool {
        screen[0] >= self.screen.x
            && screen[0] < self.screen.x + self.screen.width
            && screen[1] >= self.screen.y
            && screen[1] < self.screen.y + self.screen.height
            && canvas[0] >= 0.0
            && canvas[0] < self.canvas[0]
            && canvas[1] >= 0.0
            && canvas[1] < self.canvas[1]
    }
}

/// Clockwise corners/edge midpoints, beginning at the unrotated top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}
impl Handle {
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];
    fn direction(self) -> [f64; 2] {
        match self {
            Self::TopLeft => [-1., -1.],
            Self::Top => [0., -1.],
            Self::TopRight => [1., -1.],
            Self::Right => [1., 0.],
            Self::BottomRight => [1., 1.],
            Self::Bottom => [0., 1.],
            Self::BottomLeft => [-1., 1.],
            Self::Left => [-1., 0.],
        }
    }
}

/// Selection box in canvas pixels; masks/transparent raster pixels do not cut holes
/// in this box. Uses the same evaluated placement and transform as the compositor.
#[derive(Clone, Copy, Debug)]
pub struct ItemGeometry {
    bounds: Rect,
    transform: Transform,
}
impl ItemGeometry {
    /// Not directly constructible: geometry comes from [`Renderer::item_geometry`]
    /// or [`super::arrange::bounds`].
    pub(super) fn from_parts(bounds: Rect, transform: Transform) -> Self {
        Self { bounds, transform }
    }
    fn center(&self) -> [f64; 2] {
        [
            self.bounds.x + self.bounds.width / 2.0,
            self.bounds.y + self.bounds.height / 2.0,
        ]
    }
    /// Untransformed item box in project canvas pixels.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    /// Canvas-axis-aligned envelope after scale, reflection, rotation and
    /// translation, in project canvas pixels. It is the geometric selection box,
    /// never alpha, glyph or effect coverage.
    pub fn transformed_bounds(&self) -> Rect {
        self.transform.bounds(self.bounds)
    }
    /// Screen point for an untransformed canvas point inside the item box, with the
    /// same placement transform the compositor applies.
    pub fn point(&self, viewport: &Viewport, canvas: [f64; 2]) -> Result<[f64; 2]> {
        finite(&canvas)?;
        viewport.canvas_to_screen(self.transform.point(canvas, self.center()))
    }
    /// Untransformed canvas point for a screen point, the inverse of [`Self::point`].
    pub fn inverse(&self, viewport: &Viewport, screen: [f64; 2]) -> Result<[f64; 2]> {
        finite(&screen)?;
        Ok(self
            .transform
            .inverse(viewport.screen_to_canvas(screen)?, self.center()))
    }
    pub fn handle_position(&self, viewport: &Viewport, handle: Handle) -> Result<[f64; 2]> {
        let center = self.center();
        let [x, y] = handle.direction();
        viewport.canvas_to_screen(self.transform.point(
            [
                center[0] + x * self.bounds.width / 2.0,
                center[1] + y * self.bounds.height / 2.0,
            ],
            center,
        ))
    }
    /// Rotation grip beyond the bottom midpoint, with a constant logical-screen gap.
    pub fn rotation_handle_position(&self, viewport: &Viewport, gap: f64) -> Result<[f64; 2]> {
        finite(&[gap])?;
        if gap < 0.0 {
            return Err(Error::Invalid("rotation handle gap"));
        }
        let bottom = self.handle_position(viewport, Handle::Bottom)?;
        let center =
            viewport.canvas_to_screen(self.transform.point(self.center(), self.center()))?;
        let dx = bottom[0] - center[0];
        let dy = bottom[1] - center[1];
        let distance = dx.hypot(dy);
        if distance <= 0.0 {
            return Err(Error::Invalid("rotation handle geometry"));
        }
        let point = [
            bottom[0] + dx / distance * gap,
            bottom[1] + dy / distance * gap,
        ];
        finite(&point)?;
        Ok(point)
    }
    /// Nearest handle inside a caller-chosen logical screen-pixel radius.
    /// Equal-distance ties use `Handle::ALL` order; negative/non-finite radii refuse.
    pub fn hit_handle(
        &self,
        viewport: &Viewport,
        screen: [f64; 2],
        radius: f64,
    ) -> Result<Option<Handle>> {
        finite(&[screen[0], screen[1], radius])?;
        if radius < 0.0 {
            return Err(Error::Invalid("handle radius"));
        }
        let mut hit = None;
        let mut nearest = radius;
        for handle in Handle::ALL {
            let [x, y] = self.handle_position(viewport, handle)?;
            let distance = (screen[0] - x).hypot(screen[1] - y);
            if distance <= radius && (hit.is_none() || distance < nearest) {
                hit = Some(handle);
                nearest = distance;
            }
        }
        Ok(hit)
    }
    /// Centre-anchored per-axis scaling from an absolute canvas pointer. Edge
    /// handles retain the other axis; crossing the centre clamps rather than flips.
    /// Feed the resulting ratios through the existing transform parameter commands.
    pub fn scale_at(&self, handle: Handle, canvas: [f64; 2]) -> Result<[f64; 2]> {
        finite(&canvas)?;
        let [x, y] = self.transform.local_axis(canvas, self.center());
        let [dx, dy] = handle.direction();
        let [sx, sy] = self.transform.scale();
        Ok([
            TransformField::ScaleX.clamp(if dx == 0. {
                sx
            } else {
                x * dx * 2. / self.bounds.width
            })?,
            TransformField::ScaleY.clamp(if dy == 0. {
                sy
            } else {
                y * dy * 2. / self.bounds.height
            })?,
        ])
    }
    fn contains(&self, canvas: [f64; 2]) -> bool {
        let [x, y] = self.transform.inverse(canvas, self.center());
        x >= self.bounds.x
            && x < self.bounds.x + self.bounds.width
            && y >= self.bounds.y
            && y < self.bounds.y + self.bounds.height
    }
}

/// A mask's normalized cubic coordinates mapped through the evaluated mask
/// box, the item transform and the preview viewport. The same affine the
/// compositor uses, so point handles stay on the rendered contour.
#[derive(Clone, Copy, Debug)]
pub struct MaskGeometry {
    /// Mask-box centre in project canvas pixels.
    centre: [f64; 2],
    /// Evaluated mask-box size in project canvas pixels.
    size: [f64; 2],
    /// Clockwise radians.
    rotation: f64,
    item: ItemGeometry,
}
impl MaskGeometry {
    /// Normalized mask-local point (0..1 spans the evaluated mask box) to
    /// logical screen pixels.
    pub fn point(&self, viewport: &Viewport, local: [f64; 2]) -> Result<[f64; 2]> {
        finite(&local)?;
        let (sin, cos) = self.rotation.sin_cos();
        let x = (local[0] - 0.5) * self.size[0];
        let y = (local[1] - 0.5) * self.size[1];
        self.item.point(
            viewport,
            [
                self.centre[0] + cos * x - sin * y,
                self.centre[1] + sin * x + cos * y,
            ],
        )
    }
    /// Logical screen point to normalized mask-local coordinates; refuses a
    /// degenerate mask box or an unrepresentable result.
    pub fn inverse(&self, viewport: &Viewport, screen: [f64; 2]) -> Result<[f64; 2]> {
        if self.size.iter().any(|v| *v == 0.0 || !v.is_finite()) {
            return Err(Error::Invalid("mask geometry"));
        }
        let canvas = self.item.inverse(viewport, screen)?;
        let (sin, cos) = self.rotation.sin_cos();
        let dx = canvas[0] - self.centre[0];
        let dy = canvas[1] - self.centre[1];
        let local = [
            (dx * cos + dy * sin) / self.size[0] + 0.5,
            (-dx * sin + dy * cos) / self.size[1] + 0.5,
        ];
        finite(&local)?;
        Ok(local)
    }
}

impl Renderer<'_> {
    /// Active, selectable item bounds at output `at`. Hidden/locked tracks,
    /// audio, empty text, zero opacity and inactive caption cues are excluded.
    pub fn item_geometry(&self, item: ItemId, at: Tick) -> Result<Option<ItemGeometry>> {
        if at < Tick::ZERO {
            return Err(Error::Invalid("negative frame time"));
        }
        for track in self
            .project
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|t| !t.hidden && !t.locked)
        {
            let Some(item) = self.selectable_items(track, at)?.find(|i| i.id == item) else {
                continue;
            };
            return self.geometry_at(item, at);
        }
        Ok(None)
    }
    /// One active mask's normalized-to-screen affine at output `at`. Returns
    /// `None` when the item or mask is inactive, hidden or locked.
    pub fn mask_geometry(
        &self,
        item: ItemId,
        mask: &str,
        at: Tick,
    ) -> Result<Option<MaskGeometry>> {
        if at < Tick::ZERO {
            return Err(Error::Invalid("negative frame time"));
        }
        let Some(base) = self.item_geometry(item, at)? else {
            return Ok(None);
        };
        let Some(source) = self
            .project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item)
        else {
            return Ok(None);
        };
        let local = at
            .checked_sub(source.start)
            .ok_or(Error::Invalid("item time"))?;
        let Some(mask) = source
            .masks
            .iter()
            .find(|candidate| candidate.id == mask && candidate.active())
        else {
            return Ok(None);
        };
        let evaluated = mask.at(&source.animations, local)?;
        let placement = self.placement_at(source, local)?;
        let canvas = &self.project.data().canvas;
        let r = placement.destination;
        let dw = f64::from(canvas.width) * r.width;
        let dh = f64::from(canvas.height) * r.height;
        let centre = [
            f64::from(canvas.width) * r.x + dw * (0.5 + evaluated.center_x),
            f64::from(canvas.height) * r.y + dh * (0.5 + evaluated.center_y),
        ];
        let size = [
            dw * evaluated.width * evaluated.scale,
            dh * evaluated.height * evaluated.scale,
        ];
        if size.iter().any(|v| !v.is_finite() || *v == 0.0)
            || centre.iter().any(|v| !v.is_finite())
            || !evaluated.rotation.is_finite()
        {
            return Err(Error::Invalid("mask geometry"));
        }
        Ok(Some(MaskGeometry {
            centre,
            size,
            rotation: evaluated.rotation.to_radians(),
            item: base,
        }))
    }
    fn geometry_at(&self, item: &project::Item, at: Tick) -> Result<Option<ItemGeometry>> {
        let local = at
            .checked_sub(item.start)
            .ok_or(Error::Invalid("item time"))?;
        match &item.content {
            ItemParams::Audio(_) => return Ok(None),
            ItemParams::Graphic(_) => {
                let appearance = super::draw::graphic_appearance(item, local)?;
                if appearance.fill[3] == 0.
                    && (appearance.stroke[3] == 0. || appearance.width == 0.)
                {
                    return Ok(None);
                }
            }
            ItemParams::Overlay(p) if p.color.0[3] == 0 => {
                let stroke = crate::color::at(
                    &item.animations,
                    &project::AnimationPath::new("params.strokeColor")?,
                    local,
                    project::Color([0; 4]),
                )?;
                if stroke.0[3] == 0 {
                    return Ok(None);
                }
            }
            ItemParams::Text(p) if p.content.as_str().is_empty() => return Ok(None),
            ItemParams::Caption(p) => {
                let source = item.source_time_at(at)?;
                if !p.cues.cues.iter().any(|c| c.range.contains(source)) {
                    return Ok(None);
                }
            }
            _ => {}
        }
        let placement = self.placement_at(item, local)?;
        if placement.opacity == 0.0 {
            return Ok(None);
        }
        let canvas = &self.project.data().canvas;
        let r = placement.destination;
        Ok(Some(ItemGeometry::from_parts(
            Rect {
                x: r.x * f64::from(canvas.width),
                y: r.y * f64::from(canvas.height),
                width: r.width * f64::from(canvas.width),
                height: r.height * f64::from(canvas.height),
            },
            Transform::at(item, local)?,
        )))
    }
    /// Topmost selection box under a screen point, returning the timeline's ItemId.
    /// Letterboxes, clipped canvas and viewport edges are never selectable.
    pub fn hit_test(
        &self,
        viewport: &Viewport,
        screen: [f64; 2],
        at: Tick,
    ) -> Result<Option<ItemId>> {
        if at < Tick::ZERO {
            return Err(Error::Invalid("negative frame time"));
        }
        let dimensions = &self.project.data().canvas;
        if viewport.canvas != [f64::from(dimensions.width), f64::from(dimensions.height)] {
            return Err(Error::Invalid("viewport canvas mismatch"));
        }
        let canvas = viewport.screen_to_canvas(screen)?;
        if !viewport.contains(screen, canvas) {
            return Ok(None);
        }
        // Timeline::iter is bottom-to-top, including its typed track groups.
        let mut hit = None;
        for track in self
            .project
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|t| !t.hidden && !t.locked)
        {
            for item in self.selectable_items(track, at)? {
                if let Some(geometry) = self.geometry_at(item, at)?
                    && geometry.contains(canvas)
                {
                    hit = Some(item.id);
                }
            }
        }
        Ok(hit)
    }
    fn selectable_items<'a>(
        &'a self,
        track: &'a project::Track,
        at: Tick,
    ) -> Result<impl Iterator<Item = &'a project::Item>> {
        let timeline = &self.project.data().scene().timeline;
        let transition = timeline
            .transitions
            .iter()
            .find(|t| t.track == track.id && t.contains(at));
        let mut pair = [None, None];
        if let Some(tr) = transition {
            let (a, b) = tr.contributors(timeline, at)?;
            let progress = at
                .checked_sub(tr.start)
                .ok_or(Error::Invalid("transition time"))?
                .0 as f64
                / tr.duration.0 as f64;
            let (outgoing, incoming) =
                super::transitions::exposed_sides(&tr.kind, &tr.params, progress)?;
            pair = [outgoing.then_some(a), incoming.then_some(b)];
        }
        Ok(pair
            .into_iter()
            .flatten()
            .chain(track.items.iter().filter(move |i| {
                transition.is_none() && i.start <= at && i.end().is_ok_and(|end| at < end)
            }))
            .filter(|i| !i.hidden))
    }
}

fn finite(values: &[f64]) -> Result<()> {
    if values.iter().any(|v| !v.is_finite()) {
        Err(Error::Invalid("viewport coordinate"))
    } else {
        Ok(())
    }
}
