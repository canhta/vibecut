//! Native logical-pixel window geometry, outside project content and undo.

use serde::{Deserialize, Serialize};

/// Native logical-pixel geometry, outside project content and undo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowGeometry {
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
}
impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            size: [1440.0, 900.0],
            position: None,
            maximized: false,
        }
    }
}
impl WindowGeometry {
    pub(crate) fn valid(&self) -> bool {
        self.size
            .iter()
            .all(|v| v.is_finite() && (320.0..=16384.0).contains(v))
            && self
                .position
                .is_none_or(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 65536.0))
    }
    pub fn builder(&self) -> crate::egui::ViewportBuilder {
        let mut builder = crate::egui::ViewportBuilder::default()
            .with_inner_size(self.size)
            .with_min_inner_size([1280.0, 800.0])
            .with_maximized(self.maximized);
        if let Some(position) = self.position {
            builder = builder.with_position(position);
        }
        builder
    }
    pub fn read(ui: &crate::egui::Ui) -> Option<Self> {
        ui.input(|i| {
            let viewport = i.viewport();
            let inner = viewport.inner_rect?;
            let geometry = Self {
                size: [inner.width(), inner.height()],
                position: viewport.outer_rect.map(|r| [r.min.x, r.min.y]),
                maximized: viewport.maximized.unwrap_or(false),
            };
            geometry.valid().then_some(geometry)
        })
    }
}
