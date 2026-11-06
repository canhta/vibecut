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
    fn center(&self) -> [f64; 2] {
        [
            self.bounds.x + self.bounds.width / 2.0,
            self.bounds.y + self.bounds.height / 2.0,
        ]
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
    /// Rotation grip beyond the top midpoint, with a constant logical-screen gap.
    pub fn rotation_handle_position(&self, viewport: &Viewport, gap: f64) -> Result<[f64; 2]> {
        finite(&[gap])?;
        if gap < 0.0 {
            return Err(Error::Invalid("rotation handle gap"));
        }
        let top = self.handle_position(viewport, Handle::Top)?;
        let center =
            viewport.canvas_to_screen(self.transform.point(self.center(), self.center()))?;
        let dx = top[0] - center[0];
        let dy = top[1] - center[1];
        let distance = dx.hypot(dy);
        if distance <= 0.0 {
            return Err(Error::Invalid("rotation handle geometry"));
        }
        let point = [top[0] + dx / distance * gap, top[1] + dy / distance * gap];
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
        let [x, y] = self.transform.unrotate(canvas, self.center());
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
    fn geometry_at(&self, item: &project::Item, at: Tick) -> Result<Option<ItemGeometry>> {
        let local = at
            .checked_sub(item.start)
            .ok_or(Error::Invalid("item time"))?;
        match &item.content {
            ItemParams::Audio(_) => return Ok(None),
            ItemParams::Overlay(p) if p.color.0[3] == 0 => {
                let stroke = crate::color::at(
                    &item.animations,
                    "params.strokeColor",
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
        Ok(Some(ItemGeometry {
            bounds: Rect {
                x: r.x * f64::from(canvas.width),
                y: r.y * f64::from(canvas.height),
                width: r.width * f64::from(canvas.width),
                height: r.height * f64::from(canvas.height),
            },
            transform: Transform::at(item, local)?,
        }))
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
            let dip = project::transition_definition(&tr.kind)?.dip.is_some();
            pair = [
                (progress < 1.0 && (!dip || progress < 0.5)).then_some(a),
                (progress > 0.0 && (!dip || progress > 0.5)).then_some(b),
            ];
        }
        Ok(pair
            .into_iter()
            .flatten()
            .chain(track.items.iter().filter(move |i| {
                transition.is_none() && i.start <= at && i.end().is_ok_and(|end| at < end)
            })))
    }
}

fn finite(values: &[f64]) -> Result<()> {
    if values.iter().any(|v| !v.is_finite()) {
        Err(Error::Invalid("viewport coordinate"))
    } else {
        Ok(())
    }
}
