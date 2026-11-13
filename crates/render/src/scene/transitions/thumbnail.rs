//! Tile thumbnails: a transition applied to two generated frames, cached.
use super::apply;
use crate::{Error, ExecutionHooks, Image, Result};
use project::TransitionParams;
use std::sync::{Arc, Mutex, PoisonError};

const MAX_EDGE: u32 = 512;

#[derive(Clone, PartialEq, Eq)]
struct Key {
    kind: String,
    params: String,
    size: (u32, u32),
    permille: u16,
}

/// Renders and keeps the most recent thumbnails, evicting the oldest past `capacity`.
pub struct ThumbnailCache {
    capacity: usize,
    entries: Mutex<Vec<(Key, Arc<Image>)>>,
}
impl ThumbnailCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Mutex::new(Vec::new()),
        }
    }
    /// The kind at `progress` (0..=1, kept to a thousandth) on the generated frames, `size` pixels
    /// per edge between 1 and 512. The same arguments return the same shared image.
    pub fn thumbnail(
        &self,
        kind: &str,
        params: &TransitionParams,
        size: (u32, u32),
        progress: f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Arc<Image>> {
        if !(1..=MAX_EDGE).contains(&size.0) || !(1..=MAX_EDGE).contains(&size.1) {
            return Err(Error::Invalid("thumbnail size"));
        }
        if !progress.is_finite() {
            return Err(Error::Invalid("transition progress"));
        }
        let permille = (progress.clamp(0., 1.) * 1000.).round() as u16;
        let key = Key {
            kind: kind.to_owned(),
            params: format!("{params:?}"),
            size,
            permille,
        };
        if let Some(hit) = self.find(&key) {
            return Ok(hit);
        }
        let [a, b] = frames(size.0, size.1, hooks)?;
        let image = Arc::new(apply(
            kind,
            params,
            &a,
            &b,
            f64::from(permille) / 1000.,
            1.,
            hooks,
        )?);
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if self.capacity > 0 {
            if entries.len() >= self.capacity {
                entries.remove(0);
            }
            entries.push((key, Arc::clone(&image)));
        }
        Ok(image)
    }
    fn find(&self, key: &Key) -> Option<Arc<Image>> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, image)| Arc::clone(image))
    }
}

/// Two opaque generated frames: a warm ramp under a checker, and a cool ramp with a bright disc.
fn frames(width: u32, height: u32, hooks: &dyn ExecutionHooks) -> Result<[Image; 2]> {
    let mut a = Image::new(width, height, [0.; 4], hooks)?;
    let mut b = Image::new(width, height, [0.; 4], hooks)?;
    let (w, h) = (width as f32, height as f32);
    for (frame, warm) in [(&mut a, true), (&mut b, false)] {
        for (n, px) in frame.pixels.iter_mut().enumerate() {
            let x = (n % width as usize) as f32 + 0.5;
            let y = (n / width as usize) as f32 + 0.5;
            *px = if warm {
                let cell = (w / 8.).max(1.);
                let checker = if ((x / cell).floor() + (y / cell).floor()) % 2. == 0. {
                    0.15
                } else {
                    0.
                };
                [
                    (0.2 + 0.7 * x / w + checker).min(1.),
                    0.1 + 0.3 * y / h,
                    0.05 + checker,
                    1.,
                ]
            } else {
                let disc = if (x - w * 0.62).hypot(y - h * 0.4) < h * 0.22 {
                    0.6
                } else {
                    0.
                };
                [
                    0.05 + disc,
                    (0.2 + 0.2 * y / h + disc).min(1.),
                    (0.4 + 0.5 * y / h).min(1.),
                    1.,
                ]
            };
        }
    }
    Ok([a, b])
}
