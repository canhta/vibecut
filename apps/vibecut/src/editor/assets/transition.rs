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
        _ => Text::Transitions,
    }
}

/// A tile's cached thumbnail, rendered once from the shared procedural frames.
pub(super) struct Preview {
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) failed: bool,
    pub(super) dark: bool,
}
impl Preview {
    pub(super) fn new(_kind: &str, dark: bool) -> crate::Result<Self> {
        Ok(Self {
            texture: None,
            failed: false,
            dark,
        })
    }
    pub(super) fn update(&mut self, ctx: &egui::Context, kind: &str, _active: bool) {
        if self.texture.is_some() || self.failed {
            return;
        }
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
            .thumbnail(kind, &TransitionParams::new(), (176, 176), 0.5, &hooks)
            .and_then(|image| image.rgba8(13, &hooks));
        match result {
            Ok(image) => {
                let image = egui::ColorImage::from_rgba_unmultiplied([176, 176], &image.bytes);
                self.texture =
                    Some(ctx.load_texture(format!("transition.{kind}"), image, Default::default()));
            }
            Err(error) => {
                tracing::warn!(category = "transition_preview", %error, "preview failed");
                self.failed = true;
            }
        }
    }
}
