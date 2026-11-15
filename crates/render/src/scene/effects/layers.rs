//! Device-free frame composition over already prepared immutable layers.
use super::{CpuEffects, EffectCompositor, EffectPass, GAUSSIAN, apply_effect, gaussian_run};
use crate::{Error, ExecutionHooks, Image, Result, blend, check};
use project::BlendMode;

/// The renderer's per-frame layer bound.
pub const MAX_FRAME_LAYERS: usize = 4096;

/// The entries a device may keep resident inside one admitted frame.
pub const RESIDENT_ENTRIES: [&str; 4] = [GAUSSIAN, "grade", "chroma_key", "mirror"];

/// One borrowed, prepared full-canvas working image with its finite opacity and mode.
/// Layers run bottom to top; the borrows are the only retention of their pixels.
/// `effects` holds the layer's evaluated effect instances in stack order, one pass
/// list each (an empty list is skipped); each instance starts from the previous
/// one's output and its sample passes read that instance's own input. `coverage`
/// is the layer's prepared combined mask coverage, read from alpha, which scales
/// all four effected channels before the blend.
#[derive(Clone, Copy)]
pub struct FrameLayer<'a> {
    pub image: &'a Image,
    pub opacity: f32,
    pub mode: BlendMode,
    pub effects: &'a [Vec<EffectPass>],
    pub coverage: Option<&'a Image>,
}

/// Refuse before any allocation or device work: bounded count, finite opacity in
/// `0..=1`, every layer and coverage exactly `width` by `height` (Invalid), and
/// every evaluated pass admitted for that geometry (its own typed refusal).
pub fn validate_layers(width: u32, height: u32, layers: &[FrameLayer<'_>]) -> Result<()> {
    Image::bytes(width, height)?;
    if layers.len() > MAX_FRAME_LAYERS {
        return Err(Error::Invalid("frame layer count"));
    }
    for layer in layers {
        if (layer.image.width(), layer.image.height()) != (width, height)
            || layer
                .coverage
                .is_some_and(|c| (c.width(), c.height()) != (width, height))
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
        {
            return Err(Error::Invalid("frame layer"));
        }
    }
    for pass in layers
        .iter()
        .flat_map(|layer| layer.effects.iter().flatten())
    {
        super::validate_pass(pass, width, height)?;
    }
    Ok(())
}

/// Whether a device may run every instance of `effects` resident at this geometry:
/// only [`RESIDENT_ENTRIES`], and no gaussian run the CPU reference reduces (see
/// `gaussian_run`), so device and reference evaluate the same passes.
pub fn resident(effects: &[Vec<EffectPass>], width: u32, height: u32) -> bool {
    effects.iter().all(|run| {
        run.iter()
            .all(|pass| RESIDENT_ENTRIES.contains(&pass.entry_point()))
            && (0..run.len())
                .filter_map(|start| run.get(start..))
                .all(|rest| gaussian_run(rest, width, height).is_none())
    })
}

/// CPU reference: a transparent canvas, then each layer's effect instances through
/// [`apply_effect`] on [`CpuEffects`], its coverage, and [`blend::composite`].
/// Inputs are borrowed and unchanged; a layer without effects or coverage is
/// blended in place of a copy.
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
        let prepared = prepare(layer, &out, hooks)?;
        let image = prepared.as_ref().unwrap_or(layer.image);
        blend::composite(&mut out, image, layer.opacity, layer.mode, hooks)?;
    }
    Ok(out)
}
/// The layer's effected, coverage-scaled working copy; None when it has neither.
fn prepare(
    layer: &FrameLayer<'_>,
    below: &Image,
    hooks: &dyn ExecutionHooks,
) -> Result<Option<Image>> {
    let mut runs = layer
        .effects
        .iter()
        .filter(|run| !run.is_empty())
        .peekable();
    if runs.peek().is_none() && layer.coverage.is_none() {
        return Ok(None);
    }
    let mut image = layer.image.copy(hooks)?;
    for run in runs {
        image = apply_effect(image, run, below, &CpuEffects, hooks)?;
    }
    if let Some(coverage) = layer.coverage {
        crate::mask::scale_by(&mut image, coverage, hooks)?;
    }
    Ok(Some(image))
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
