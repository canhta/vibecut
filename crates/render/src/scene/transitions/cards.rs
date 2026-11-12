//! Textured 3D cards under a pinhole camera for perspective transitions.
//!
//! World space is pixel units: the screen is the plane z = 0 spanning (0..w, 0..h), +z points
//! away from the viewer and the eye sits at (w/2, h/2, -focal). A card at z = 0 with the identity
//! transform therefore covers the frame pixel-for-pixel.

/// A 3-vector.
pub(super) type V3 = [f32; 3];

#[inline]
fn sub(a: V3, b: V3) -> V3 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    [ax - bx, ay - by, az - bz]
}
#[inline]
fn add(a: V3, b: V3) -> V3 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    [ax + bx, ay + by, az + bz]
}
#[inline]
pub(super) fn cross(a: V3, b: V3) -> V3 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}
#[inline]
fn dot(a: V3, b: V3) -> f32 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    ax * bx + ay * by + az * bz
}

/// An affine placement of a card: world = origin + lx*ex + ly*ey for image pixel (lx, ly).
#[derive(Clone, Copy, Debug)]
pub(super) struct Card {
    pub(super) origin: V3,
    pub(super) ex: V3,
    pub(super) ey: V3,
}

impl Card {
    /// The identity card (covers the frame at z = 0).
    pub(super) const fn flat() -> Self {
        Card {
            origin: [0.; 3],
            ex: [1., 0., 0.],
            ey: [0., 1., 0.],
        }
    }
    fn map(self, point: impl Fn(V3) -> V3, linear: impl Fn(V3) -> V3) -> Self {
        Card {
            origin: point(self.origin),
            ex: linear(self.ex),
            ey: linear(self.ey),
        }
    }
    pub(super) fn translate(self, d: V3) -> Self {
        let [dx, dy, dz] = d;
        self.map(
            |p| {
                let [x, y, z] = p;
                [x + dx, y + dy, z + dz]
            },
            |v| v,
        )
    }
    /// Uniform scale about `c`.
    pub(super) fn scale_about(self, c: V3, s: f32) -> Self {
        let [cx, cy, cz] = c;
        self.map(
            |p| {
                let [x, y, z] = p;
                [cx + (x - cx) * s, cy + (y - cy) * s, cz + (z - cz) * s]
            },
            |v| {
                let [x, y, z] = v;
                [x * s, y * s, z * s]
            },
        )
    }
    /// Rotate about the vertical (y) axis through `c` by `a` radians (positive turns the right
    /// edge away).
    pub(super) fn rot_y(self, c: V3, a: f32) -> Self {
        let (s, co) = a.sin_cos();
        let rotate = move |v: V3| {
            let [x, y, z] = v;
            [x * co - z * s, y, x * s + z * co]
        };
        self.map(move |p| add(c, rotate(sub(p, c))), rotate)
    }
    /// Rotate about the horizontal (x) axis through `c` (positive turns the bottom edge away).
    pub(super) fn rot_x(self, c: V3, a: f32) -> Self {
        let (s, co) = a.sin_cos();
        let rotate = move |v: V3| {
            let [x, y, z] = v;
            [x, y * co - z * s, y * s + z * co]
        };
        self.map(move |p| add(c, rotate(sub(p, c))), rotate)
    }
    /// Rotate in the screen plane about `c` (positive = clockwise on screen).
    pub(super) fn rot_z(self, c: V3, a: f32) -> Self {
        let (s, co) = a.sin_cos();
        let rotate = move |v: V3| {
            let [x, y, z] = v;
            [x * co - y * s, x * s + y * co, z]
        };
        self.map(move |p| add(c, rotate(sub(p, c))), rotate)
    }

    /// Intersect the eye ray through screen point (x, y): local image coordinates, depth along
    /// the ray, and whether the front face (the side with the normal towards the eye) is seen.
    pub(super) fn hit(&self, eye: V3, x: f32, y: f32) -> Option<(f32, f32, f32, bool)> {
        let d = sub([x, y, 0.], eye);
        // eye + t*d = origin + lx*ex + ly*ey  ->  lx*ex + ly*ey - t*d = eye - origin
        let rhs = sub(eye, self.origin);
        let [dx, dy, dz] = d;
        let nd = [-dx, -dy, -dz];
        let det = dot(self.ex, cross(self.ey, nd));
        if det.abs() < 1e-9 {
            return None;
        }
        let lx = dot(rhs, cross(self.ey, nd)) / det;
        let ly = dot(self.ex, cross(rhs, nd)) / det;
        let t = dot(self.ex, cross(self.ey, rhs)) / det;
        if t <= 1e-4 {
            return None;
        }
        let n = cross(self.ex, self.ey);
        let front = dot(n, d) > 0.;
        Some((lx, ly, t, front))
    }
}

/// The camera eye for a frame of `w`x`h` with a focal distance of `focal` pixels.
pub(super) fn eye(w: f32, h: f32, focal: f32) -> V3 {
    [w / 2., h / 2., -focal.max(1.)]
}

/// Focal distance for a "Perspective" percentage (0 % = nearly orthographic, 100 % = wide angle).
pub(super) fn focal_for(w: f32, h: f32, perspective: f32) -> f32 {
    w.max(h) * (6. - 5. * perspective.clamp(0., 1.)).max(0.6)
}
