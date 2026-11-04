use crate::{Error, ExecutionHooks, Stage, contracts::checkpoint};
const MAX_ROWS: usize = 444;
const MAX_FRAMES: usize = 1500;
/// Dense bounded negative-attention DTW; no missing attention becomes fake time.
pub(super) fn boundaries(
    cost: &[f32],
    rows: usize,
    frames: usize,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<usize>, Error> {
    if !(2..=MAX_ROWS).contains(&rows)
        || frames == 0
        || frames > MAX_FRAMES
        || cost.len() != rows * frames
        || cost.iter().any(|v| !v.is_finite())
    {
        return Err(Error::Invalid("DTW matrix"));
    }
    let width = frames + 1;
    let mut acc = vec![f32::INFINITY; (rows + 1) * width];
    let mut trace = vec![0u8; acc.len()];
    *acc.first_mut().ok_or(Error::Inference)? = 0.;
    for i in 1..=rows {
        checkpoint(hooks, Stage::Align)?;
        for j in 1..=frames {
            let at = i * width + j;
            let a = *acc.get(at - width - 1).ok_or(Error::Inference)?;
            let b = *acc.get(at - width).ok_or(Error::Inference)?;
            let c = *acc.get(at - 1).ok_or(Error::Inference)?;
            let (v, t) = if a < b && a < c {
                (a, 0)
            } else if b < a && b < c {
                (b, 1)
            } else {
                (c, 2)
            };
            *acc.get_mut(at).ok_or(Error::Inference)? =
                cost.get((i - 1) * frames + j - 1).ok_or(Error::Inference)? + v;
            *trace.get_mut(at).ok_or(Error::Inference)? = t;
        }
    }
    if !acc.last().is_some_and(|v| v.is_finite()) {
        return Err(Error::TimingUnavailable);
    }
    let (mut i, mut j) = (rows, frames);
    let mut jumps = vec![usize::MAX; rows];
    while i > 0 && j > 0 {
        *jumps.get_mut(i - 1).ok_or(Error::Inference)? = j - 1;
        match trace.get(i * width + j).ok_or(Error::Inference)? {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
    }
    if jumps.contains(&usize::MAX) || jumps.windows(2).any(|w| w.first() > w.get(1)) {
        return Err(Error::TimingUnavailable);
    }
    Ok(jumps)
}
/// Standardize over query tokens, median filter each selected row, then average heads.
pub(super) fn costs(
    heads: &[Vec<Vec<f32>>],
    first: usize,
    rows: usize,
    frames: usize,
) -> Result<Vec<f32>, Error> {
    if heads.is_empty()
        || heads.len() > 64
        || !(2..=MAX_ROWS).contains(&rows)
        || frames == 0
        || frames > MAX_FRAMES
    {
        return Err(Error::AttentionUnavailable);
    }
    let tokens = heads.first().ok_or(Error::AttentionUnavailable)?.len();
    if tokens == 0 || tokens > 448 || first.checked_add(rows).is_none_or(|n| n > tokens) {
        return Err(Error::AttentionUnavailable);
    }
    let mut out = vec![0.; rows * frames];
    for head in heads {
        if head.len() != tokens
            || head
                .iter()
                .any(|r| r.len() != frames || r.iter().any(|v| !v.is_finite()))
        {
            return Err(Error::AttentionUnavailable);
        }
        let mut normalized = vec![0.; rows * frames];
        for f in 0..frames {
            let values = head
                .iter()
                .map(|r| r.get(f).copied().ok_or(Error::AttentionUnavailable))
                .collect::<Result<Vec<_>, _>>()?;
            let mean = values.iter().sum::<f32>() / tokens as f32;
            let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / tokens as f32)
                .sqrt()
                .max(1e-8);
            for r in 0..rows {
                *normalized.get_mut(r * frames + f).ok_or(Error::Inference)? =
                    (*values.get(first + r).ok_or(Error::Inference)? - mean) / sd;
            }
        }
        for r in 0..rows {
            for f in 0..frames {
                let mut window = [0.; 7];
                for (k, v) in window.iter_mut().enumerate() {
                    let index = reflect(f as isize + k as isize - 3, frames);
                    *v = *normalized.get(r * frames + index).ok_or(Error::Inference)?;
                }
                window.sort_by(f32::total_cmp);
                *out.get_mut(r * frames + f).ok_or(Error::Inference)? -=
                    window.get(3).ok_or(Error::Inference)? / heads.len() as f32;
            }
        }
    }
    if out.iter().any(|v| !v.is_finite()) || out.iter().all(|v| v.abs() < 1e-6) {
        return Err(Error::AttentionUnavailable);
    }
    Ok(out)
}
pub(super) fn reflect(index: isize, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n as isize - 1);
    let i = index.rem_euclid(period);
    if i >= n as isize {
        (period - i) as usize
    } else {
        i as usize
    }
}
