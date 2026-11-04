//! Bounded fresh-frame screenshots; captures are evidence of presentation only.
use crate::{Error, Result, egui};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

pub struct Capture {
    rx: mpsc::Receiver<Arc<egui::ColorImage>>,
    deadline: Instant,
    region: Option<egui::Rect>,
    scale: f32,
}
impl Capture {
    /// Must be invoked during a new UI frame after the command/inspection of interest.
    /// No minimized-window or headless fallback can return an old framebuffer.
    pub fn request(
        ctx: &egui::Context,
        region: Option<egui::Rect>,
        timeout: Duration,
    ) -> Result<Self> {
        if timeout.is_zero() || timeout > Duration::from_secs(5) {
            return Err(Error::Invalid("capture timeout"));
        }
        if region.is_some_and(|rect| {
            !rect.is_finite() || !rect.is_positive() || rect.min.x < 0.0 || rect.min.y < 0.0
        }) {
            return Err(Error::Invalid("capture region"));
        }
        let (tx, rx) = mpsc::sync_channel(1);
        ctx.request_screenshot(move |image| {
            let _ = tx.try_send(image);
        });
        ctx.request_repaint();
        Ok(Self {
            rx,
            deadline: Instant::now() + timeout,
            region,
            scale: ctx.pixels_per_point(),
        })
    }
    /// Poll from the UI; never wait on the render/device thread.
    pub fn poll(&self) -> Result<Option<egui::ColorImage>> {
        if Instant::now() >= self.deadline {
            return Err(Error::CaptureTimeout);
        }
        match self.rx.try_recv() {
            Ok(image) => Ok(Some(if let Some(rect) = self.region {
                if rect.max.x * self.scale > image.width() as f32
                    || rect.max.y * self.scale > image.height() as f32
                {
                    return Err(Error::CaptureUnavailable);
                }
                image.region(&rect, Some(self.scale))
            } else {
                (*image).clone()
            })),
            Err(mpsc::TryRecvError::Disconnected) => Err(Error::CaptureUnavailable),
            Err(mpsc::TryRecvError::Empty) if Instant::now() >= self.deadline => {
                Err(Error::CaptureTimeout)
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn resize_cannot_turn_panel_capture_into_an_out_of_bounds_panic() {
        let (tx, rx) = mpsc::sync_channel(1);
        let capture = Capture {
            rx,
            deadline: Instant::now() + Duration::from_secs(1),
            region: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(20.0, 20.0),
            )),
            scale: 1.0,
        };
        tx.send(Arc::new(egui::ColorImage::filled(
            [2, 2],
            egui::Color32::BLACK,
        )))
        .unwrap();
        assert!(matches!(capture.poll(), Err(Error::CaptureUnavailable)));
        assert!(matches!(
            Capture::request(
                &egui::Context::default(),
                Some(egui::Rect::from_min_max(
                    egui::pos2(10.0, 10.0),
                    egui::Pos2::ZERO
                )),
                Duration::from_secs(1)
            ),
            Err(Error::Invalid(_))
        ));
    }
}
