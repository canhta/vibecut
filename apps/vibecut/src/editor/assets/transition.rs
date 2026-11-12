//! Names the catalogue for the browser and renders cached procedural thumbnails.
use super::{Text, egui};
use engine::{ByteBudget, ByteBudgetHooks};
use project::{TransitionDefinition, TransitionParams};

pub(super) fn name(definition: &TransitionDefinition) -> Text {
    match definition.kind {
        "cross_dissolve" => Text::TransitionCrossDissolve,
        "dip_to_black" => Text::TransitionDipBlack,
        "dip_to_white" => Text::TransitionDipWhite,
        "dip_to_color" => Text::TransitionDipColour,
        "light_fade" => Text::TransitionLightFade,
        "film_fade" => Text::TransitionFilmFade,
        "soft_focus_fade" => Text::TransitionSoftFocusFade,
        "burn_through" => Text::TransitionBurnThrough,
        "brightness_fade" => Text::TransitionBrightnessFade,
        "pixel_blocks" => Text::TransitionPixelBlocks,
        "straight_wipe" => Text::TransitionStraight,
        "soft_edge_wipe" => Text::TransitionSoftEdge,
        "clock_wipe" => Text::TransitionClock,
        "corner_sweep" => Text::TransitionCornerSweep,
        "star_wipe" => Text::TransitionStar,
        "panel_wipe" => Text::TransitionPanel,
        "stair_wipe" => Text::TransitionStair,
        "stretch_wipe" => Text::TransitionStretch,
        "neon_edge_wipe" => Text::TransitionNeonEdge,
        "circle_reveal" => Text::TransitionCircleReveal,
        "box_reveal" => Text::TransitionBoxReveal,
        "diamond_reveal" => Text::TransitionDiamondReveal,
        "cross_reveal" => Text::TransitionCrossReveal,
        "double_doors" => Text::TransitionDoubleDoors,
        "corner_inset" => Text::TransitionCornerInset,
        "zoom_cross" => Text::TransitionZoomCross,
        "zoom_smear" => Text::TransitionZoomSmear,
        "spin_smear" => Text::TransitionSpinSmear,
        "streak_smear" => Text::TransitionStreakSmear,
        "lens_defocus" => Text::TransitionLensDefocus,
        "bloom" => Text::TransitionBloom,
        "flash" => Text::TransitionFlash,
        "lens_flare" => Text::TransitionLensFlare,
        "light_leak" => Text::TransitionLightLeak,
        "colour_leak" => Text::TransitionColourLeak,
        "colour_burn" => Text::TransitionColourBurn,
        "light_sweep" => Text::TransitionLightSweep,
        "light_rays" => Text::TransitionLightRays,
        "phosphor_trail" => Text::TransitionPhosphorTrail,
        "solarise" => Text::TransitionSolarise,
        "stripes" => Text::TransitionStripes,
        "scramble" => Text::TransitionScramble,
        "digital_glitch" => Text::TransitionDigitalGlitch,
        "tape_damage" => Text::TransitionTapeDamage,
        "flicker_cut" => Text::TransitionFlickerCut,
        "quake" => Text::TransitionQuake,
        "glass_shards" => Text::TransitionGlassShards,
        "rough_edge" => Text::TransitionRoughEdge,
        "kaleidoscope" => Text::TransitionKaleidoscope,
        "liquid_warp" => Text::TransitionLiquidWarp,
        "screen_off" => Text::TransitionScreenOff,
        "push" => Text::TransitionPush,
        "slide" => Text::TransitionSlide,
        "roll" => Text::TransitionRoll,
        "split" => Text::TransitionSplitOpen,
        "stretch" => Text::TransitionStretchIn,
        "film_roll" => Text::TransitionFilmStrip,
        "spring_motion" => Text::TransitionSpring,
        "wave" => Text::TransitionWave,
        "slice" => Text::TransitionSlices,
        "louver" => Text::TransitionBlinds,
        "frame" => Text::TransitionPictureFrame,
        "roll_3d" => Text::TransitionCylinderRoll,
        "flip_motion" => Text::TransitionCardFlip,
        "fold_motion" => Text::TransitionFoldOpen,
        "spin_motion" => Text::TransitionSpin,
        "spin_3d" => Text::TransitionTurntable,
        "spinback_3d" => Text::TransitionTurntableBack,
        "page_peel" => Text::TransitionPagePeel,
        "mirror_transition" => Text::TransitionMirrorFlip,
        "block_motion" => Text::TransitionTileSlide,
        "whip" => Text::TransitionWhipPan,
        "motion_camera" => Text::TransitionCameraMove,
        "travel_motion" => Text::TransitionZoomTravel,
        "motion_tween" => Text::TransitionMatchScale,
        "pull_motion" => Text::TransitionPull,
        "pop_motion" => Text::TransitionPop,
        "shape_dissolve" => Text::TransitionShapeDissolve,
        "shape_flow" => Text::TransitionShapeFlow,
        _ => Text::Transitions,
    }
}

/// The shelf's left-rail label; `Basic` is shared with other panels.
pub(super) fn shelf_label(shelf: &str) -> Text {
    match shelf {
        "Basic" => Text::Basic,
        "Overlay" => Text::TransitionShelfOverlay,
        "Light" => Text::TransitionShelfLight,
        "Blur" => Text::TransitionShelfBlur,
        "Wipes" => Text::TransitionShelfWipes,
        "Glitch & Distort" => Text::TransitionShelfGlitchDistort,
        "Slides" => Text::TransitionShelfSlides,
        "3D" => Text::TransitionShelf3d,
        "Camera" => Text::TransitionShelfCamera,
        _ => Text::Transitions,
    }
}

/// Idle tiles retain a 176 px raster; hover/focus plays a 48 px, 12 fps loop.
pub(super) struct Preview {
    idle: Option<egui::TextureHandle>,
    animated: Option<egui::TextureHandle>,
    frame: Option<u32>,
    started: Option<f64>,
    pub(super) progress: f32,
    pub(super) failed: bool,
    pub(super) dark: bool,
}
impl Preview {
    pub(super) fn new(_kind: &str, dark: bool) -> crate::Result<Self> {
        Ok(Self {
            idle: None,
            animated: None,
            frame: None,
            started: None,
            progress: 0.5,
            failed: false,
            dark,
        })
    }
    pub(super) fn texture(&self) -> Option<&egui::TextureHandle> {
        if self.frame.is_some() {
            self.animated.as_ref()
        } else {
            self.idle.as_ref()
        }
    }
    pub(super) fn update(&mut self, ctx: &egui::Context, kind: &str, active: bool) {
        if self.failed {
            return;
        }
        let (size, progress) = if active {
            let now = ctx.input(|input| input.time);
            let start = *self.started.get_or_insert(now);
            let frame = (((now - start).max(0.) * 12.).floor() as u32) % 24;
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1. / 12.));
            if self.frame == Some(frame) {
                return;
            }
            self.frame = Some(frame);
            (48, frame as f32 / 23.)
        } else {
            self.started = None;
            self.frame = None;
            self.progress = 0.5;
            if self.idle.is_some() {
                return;
            }
            (176, 0.5)
        };
        let budget = match ByteBudget::new(64 << 20) {
            Ok(budget) => budget,
            Err(error) => {
                tracing::warn!(category = "transition_preview", %error, "preview budget");
                self.failed = true;
                return;
            }
        };
        let hooks = ByteBudgetHooks::new(&budget, None);
        let cache = render::transition_thumbnail::ThumbnailCache::new(1);
        let result = cache
            .thumbnail(
                kind,
                &TransitionParams::new(),
                (size, size),
                progress,
                &hooks,
            )
            .and_then(|image| image.rgba8(13, &hooks));
        match result {
            Ok(image) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [size as usize, size as usize],
                    &image.bytes,
                );
                let target = if active {
                    &mut self.animated
                } else {
                    &mut self.idle
                };
                if let Some(texture) = target {
                    texture.set(image, Default::default());
                } else {
                    *target = Some(ctx.load_texture(
                        format!(
                            "transition.{kind}.{}",
                            if active { "animated" } else { "idle" }
                        ),
                        image,
                        Default::default(),
                    ));
                }
                self.progress = progress;
            }
            Err(error) => {
                tracing::warn!(category = "transition_preview", %error, "preview failed");
                self.failed = true;
            }
        }
    }
}
