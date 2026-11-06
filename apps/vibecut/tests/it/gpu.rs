//! One offscreen wgpu device for the whole test binary.
//!
//! `egui_kittest` otherwise builds an instance, adapter and device per harness. On Windows the
//! CPU adapter is WARP (`d3d10warp.dll`), and creating and destroying several WARP devices
//! concurrently under CPU load crashes the process (`STATUS_ACCESS_VIOLATION`, once
//! `STATUS_HEAP_CORRUPTION`). Every harness here shares this device; each still owns its renderer
//! and textures. The device is created on the first `render()`, so harnesses that never render
//! never touch wgpu.
use eframe::egui_wgpu::{RendererOptions, WgpuSetup, WgpuSetupExisting};
use egui_kittest::{
    Harness, HarnessBuilder, LazyRenderer,
    wgpu::{WgpuTestRenderer, create_render_state, default_wgpu_setup},
};
use std::sync::OnceLock;

static SHARED: OnceLock<WgpuSetupExisting> = OnceLock::new();

fn shared() -> WgpuSetupExisting {
    SHARED
        .get_or_init(|| {
            let state = create_render_state(default_wgpu_setup(), RendererOptions::PREDICTABLE);
            WgpuSetupExisting {
                instance: state.instance.clone(),
                adapter: state.adapter.clone(),
                device: state.device.clone(),
                queue: state.queue.clone(),
            }
        })
        .clone()
}

/// `Harness::builder()` rendering on the shared device.
pub fn builder<S>() -> HarnessBuilder<S> {
    Harness::builder().renderer(LazyRenderer::new(|| {
        WgpuTestRenderer::from_setup(WgpuSetup::Existing(shared()))
    }))
}
