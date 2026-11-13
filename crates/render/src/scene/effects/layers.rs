//! Device-free frame composition over already prepared immutable layers.
use super::EffectCompositor;
use crate::{Error, ExecutionHooks, Image, Result, blend, check};
use project::BlendMode;

/// The renderer's per-frame layer bound.
pub const MAX_FRAME_LAYERS: usize = 4096;

/// One borrowed, prepared full-canvas working image with its finite opacity and mode.
/// Layers run bottom to top; the borrow is the only retention of its pixels.
#[derive(Clone, Copy)]
pub struct FrameLayer<'a> {
    pub image: &'a Image,
    pub opacity: f32,
    pub mode: BlendMode,
}

/// Refuse before any allocation or device work: bounded count, finite opacity in
/// `0..=1` and every layer exactly `width` by `height`.
pub fn validate_layers(width: u32, height: u32, layers: &[FrameLayer<'_>]) -> Result<()> {
    Image::bytes(width, height)?;
    if layers.len() > MAX_FRAME_LAYERS {
        return Err(Error::Invalid("frame layer count"));
    }
    for layer in layers {
        if (layer.image.width(), layer.image.height()) != (width, height)
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
        {
            return Err(Error::Invalid("frame layer"));
        }
    }
    Ok(())
}

/// CPU reference: a transparent canvas, then each layer through [`blend::composite`].
/// The result is the only allocation; inputs are borrowed and unchanged.
pub fn reference_layers(
    width: u32,
    height: u32,
    layers: &[FrameLayer<'_>],
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    check(hooks)?;
    validate_layers(width, height, layers)?;
    let mut out = Image::new(width, height, [0.0; 4], hooks)?;
    for layer in layers {
        blend::composite(&mut out, layer.image, layer.opacity, layer.mode, hooks)?;
    }
    Ok(out)
}

/// Composite `layers` through `adapter`'s optional frame operation. Only adapter
/// `Unsupported` (including an unpublished runtime loss) discards its result and
/// recomputes from these exact borrowed layers on the CPU reference; invalid
/// input, budget, cancellation and other failures propagate unpublished.
pub fn composite_layers(
    width: u32,
    height: u32,
    layers: &[FrameLayer<'_>],
    adapter: &dyn EffectCompositor,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    check(hooks)?;
    validate_layers(width, height, layers)?;
    match adapter.composite_layers(width, height, layers, hooks) {
        Err(Error::Unsupported(_)) => {
            adapter.release_idle()?;
            reference_layers(width, height, layers, hooks)
        }
        result => result,
    }
}

/// Settle an adjustment layer onto its backdrop. `adjusted` is the backdrop after the layer's
/// effect stack; `coverage` is the layer's region (its alpha, after masks and transform). Normal
/// mode replaces the backdrop by `adjusted` in proportion to `coverage * opacity`, so a pixel
/// outside the region keeps its exact bits. Any other mode blends the coverage-scaled
/// `adjusted` over the backdrop like an ordinary layer, through the adapter's blend with the
/// CPU reference as the `Unsupported` fallback.
pub(crate) fn adjust_backdrop(
    below: &mut Image,
    mut adjusted: Image,
    coverage: &Image,
    opacity: f32,
    mode: BlendMode,
    adapter: &dyn EffectCompositor,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let (width, height) = (below.width(), below.height());
    if (adjusted.width(), adjusted.height()) != (width, height)
        || (coverage.width(), coverage.height()) != (width, height)
        || !opacity.is_finite()
        || !(0.0..=1.0).contains(&opacity)
    {
        return Err(Error::Invalid("adjustment layer inputs"));
    }
    let row = width as usize;
    if mode == BlendMode::Normal {
        for ((below, adjusted), coverage) in below
            .pixels
            .chunks_mut(row)
            .zip(adjusted.pixels.chunks(row))
            .zip(coverage.pixels.chunks(row))
        {
            check(hooks)?;
            for ((b, a), c) in below.iter_mut().zip(adjusted).zip(coverage) {
                let weight = c[3] * opacity;
                if weight > 0.0 {
                    for (target, adjusted) in b.iter_mut().zip(a) {
                        *target += (adjusted - *target) * weight;
                    }
                }
            }
        }
        return Ok(());
    }
    for (adjusted, coverage) in adjusted
        .pixels
        .chunks_mut(row)
        .zip(coverage.pixels.chunks(row))
    {
        check(hooks)?;
        for (a, c) in adjusted.iter_mut().zip(coverage) {
            for channel in a.iter_mut() {
                *channel *= c[3];
            }
        }
    }
    match adapter.composite(below, &adjusted, opacity, mode, hooks) {
        Err(Error::Unsupported(_)) => {
            adapter.release_idle()?;
            blend::composite(below, &adjusted, opacity, mode, hooks)
        }
        result => result,
    }
}
