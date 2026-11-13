//! Dubbing edits: the duration-fit tiers, the overflow rule and the ducking automation.
use super::super::{EditError, ItemId, ItemParams, ProjectData, Result, Tick, item};
use project::{
    AnimationPath, AudioSource, BuiltinPath, Item, Keyframe, NarrationDucking, Param, ParamValue,
};

/// Slowest stretch a take may take before a rephrase is asked for, in thousandths.
pub const STRETCH_MIN_THOUSANDTHS: u16 = 900;
/// Fastest stretch a take may take, in thousandths.
pub const STRETCH_MAX_THOUSANDTHS: u16 = 1100;
/// A take may run over its cue by at most half of the cue (3/2) into a silent gap.
pub const OVERFLOW_NUMERATOR: i128 = 3;
pub const OVERFLOW_DENOMINATOR: i128 = 2;
/// ElevenLabs `voice_settings.speed` ceiling, in thousandths.
pub const PROVIDER_SPEED_MAX_THOUSANDTHS: u16 = 1200;

/// What the duration of a take asks of the cue it was generated for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FitInput {
    /// Natural output duration of the generated audio at speed 1.
    pub natural: Tick,
    /// The output time the cue allows.
    pub cue: Tick,
    /// Silent timeline after the cue end, where an overflow may run.
    pub silent_after: Tick,
    /// Extended grapheme clusters of the text that was synthesized.
    pub characters: u64,
    /// The provider offers a speed setting that this request has not used yet.
    pub provider_speed: bool,
    /// A rephrase has already been requested for this cue.
    pub rephrased: bool,
}

/// The one way to place a take. Never cuts audio and never exceeds the limits silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The take fills the cue as generated.
    Exact,
    /// Pitch-preserving stretch within 0.90 to 1.10.
    Stretch { thousandths: u16 },
    /// The take is shorter than the cue by more than the stretch allows; it plays at speed 1
    /// and leaves a gap.
    Gap,
    /// Regenerate with the provider speed setting (in thousandths), then fit again.
    ProviderSpeed { thousandths: u16 },
    /// Ask for one shorter text of at most this many characters.
    Rephrase { max_characters: u64 },
    /// After a rephrase: the take runs `over` past the cue end into a silent gap at the
    /// fastest stretch.
    Overflow { thousandths: u16, over: Tick },
}

/// The take cannot be placed within the fit limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurationFit;

/// Decide how a take of `natural` duration fits a cue. Pure integer arithmetic.
///
/// # Errors
///
/// Returns [`DurationFit`] for a non-positive duration, or when a rephrased take is still
/// longer than the overflow limit or the silent gap after the cue.
pub fn fit(input: FitInput) -> std::result::Result<Fit, DurationFit> {
    let natural = i128::from(input.natural.0);
    let cue = i128::from(input.cue.0);
    if natural <= 0 || cue <= 0 {
        return Err(DurationFit);
    }
    // Speed that makes the take exactly as long as the cue, rounded up so a stretch
    // never leaves the take longer than the cue.
    let needed = (natural * 1000 + cue - 1) / cue;
    let min = i128::from(STRETCH_MIN_THOUSANDTHS);
    let max = i128::from(STRETCH_MAX_THOUSANDTHS);
    if needed == 1000 {
        return Ok(Fit::Exact);
    }
    if needed < min {
        return Ok(Fit::Gap);
    }
    if needed <= max {
        return Ok(Fit::Stretch {
            thousandths: u16::try_from(needed).map_err(|_| DurationFit)?,
        });
    }
    if input.provider_speed {
        // Speed up the voice so the remaining stretch stays within the limit.
        let speed = (needed * 1000 + max - 1) / max;
        if speed <= i128::from(PROVIDER_SPEED_MAX_THOUSANDTHS) {
            return Ok(Fit::ProviderSpeed {
                thousandths: u16::try_from(speed.max(1001)).map_err(|_| DurationFit)?,
            });
        }
    }
    if !input.rephrased {
        let budget = i128::from(input.characters) * cue * max / (natural * 1000);
        return Ok(Fit::Rephrase {
            max_characters: u64::try_from(budget.max(1)).map_err(|_| DurationFit)?,
        });
    }
    let stretched = natural * 1000 / max;
    let over = stretched - cue;
    if stretched * OVERFLOW_DENOMINATOR > cue * OVERFLOW_NUMERATOR
        || over > i128::from(input.silent_after.0)
    {
        return Err(DurationFit);
    }
    Ok(Fit::Overflow {
        thousandths: STRETCH_MAX_THOUSANDTHS,
        over: Tick(i64::try_from(over).map_err(|_| DurationFit)?),
    })
}

fn scalar(value: &ParamValue) -> Result<f64> {
    match value {
        ParamValue::Scalar(value) => Ok(*value),
        _ => Err(EditError::Invalid("volume animation")),
    }
}
fn ms(value: u32) -> Tick {
    Tick(i64::from(value) * (time::TICKS_PER_SECOND / 1_000))
}
fn volume_path() -> AnimationPath {
    AnimationPath::from(BuiltinPath::Volume)
}

/// The narration record of an item, when it is one.
fn narration(item: &Item) -> Option<&project::Narration> {
    match &item.content {
        ItemParams::Audio(params) => match &params.source {
            AudioSource::Narration { narration } => Some(narration),
            _ => None,
        },
        _ => None,
    }
}
fn narration_mut(item: &mut Item) -> Option<&mut project::Narration> {
    match &mut item.content {
        ItemParams::Audio(params) => match &mut params.source {
            AudioSource::Narration { narration } => Some(narration),
            _ => None,
        },
        _ => None,
    }
}

/// The four ducking breakpoints in output time: attack start, plateau start, plateau end
/// and release end.
fn breakpoints(start: Tick, end: Tick, ducking: &NarrationDucking) -> Result<[Tick; 4]> {
    let overflow = || EditError::Invalid("ducking time");
    Ok([
        start
            .checked_sub(ms(ducking.attack_ms))
            .ok_or_else(overflow)?,
        start,
        end,
        end.checked_add(ms(ducking.release_ms))
            .ok_or_else(overflow)?,
    ])
}

/// Whether the audio of an item belongs to the original programme that ducking lowers.
fn is_original(item: &Item) -> bool {
    match &item.content {
        ItemParams::Audio(params) => !matches!(params.source, AudioSource::Narration { .. }),
        ItemParams::Video(params) => params.source_audio_enabled,
        _ => false,
    }
}
fn static_volume(item: &Item) -> f64 {
    match &item.content {
        ItemParams::Audio(params) => f64::from(params.volume),
        ItemParams::Video(params) => f64::from(params.volume),
        _ => 0.0,
    }
}

/// Remove the keys a ducking wrote at its breakpoints from every original item.
fn clear_ducking(data: &mut ProjectData, points: [Tick; 4]) -> Result<()> {
    let path = volume_path();
    for track in data
        .scene_mut()
        .timeline
        .iter_mut()
        .filter(|track| !track.locked)
    {
        for target in track.items.iter_mut().filter(|item| is_original(item)) {
            let base = static_volume(target);
            let origin = target.start;
            let Some(param) = target.animations.get_mut(&path).ok() else {
                continue;
            };
            let Param::Value(channel) = param else {
                continue;
            };
            let mut removed = false;
            for point in points {
                let local = point
                    .checked_sub(origin)
                    .ok_or(EditError::Invalid("ducking time"))?;
                if channel.keys().iter().any(|key| key.time == local) {
                    channel.remove(local)?;
                    removed = true;
                }
            }
            let bare = channel.keys().is_empty()
                && scalar(channel.base()).is_ok_and(|value| value == base);
            if removed && bare {
                target.animations.remove(&path);
            }
        }
    }
    Ok(())
}

/// Write the ducking envelope of one take onto every overlapping original item. Where an
/// envelope is already present the lower gain wins at each breakpoint.
fn write_ducking(
    data: &mut ProjectData,
    points: [Tick; 4],
    ducking: &NarrationDucking,
) -> Result<()> {
    let path = volume_path();
    let depth = f64::from(ducking.depth_tenths_db) / 10.0;
    let (window_start, window_end) = (points[0], points[3]);
    for track in data
        .scene_mut()
        .timeline
        .iter_mut()
        .filter(|track| !track.locked)
    {
        for target in track.items.iter_mut().filter(|item| is_original(item)) {
            let origin = target.start;
            let end = target.end()?;
            if end <= window_start || origin >= window_end {
                continue;
            }
            let base = static_volume(target);
            let mut param = match target.animations.get(&path) {
                Some(param) => param.clone(),
                None => Param::new(ParamValue::Scalar(base))?,
            };
            let Param::Value(channel) = &mut param else {
                return Err(EditError::Invalid("volume animation"));
            };
            let [
                Some(fade_in),
                Some(low_start),
                Some(low_end),
                Some(fade_out),
            ] = points.map(|point| point.checked_sub(origin))
            else {
                return Err(EditError::Invalid("ducking time"));
            };
            let local = [fade_in, low_start, low_end, fade_out];
            let at = |channel: &project::Channel, time: Tick| -> Result<f64> {
                scalar(&channel.value_at(time))
            };
            // Each take lowers the item's own level by its depth; the lowest gain wins
            // where takes overlap, and a quieter user curve is never raised.
            let floor = base - depth;
            let targets = [
                at(channel, fade_in)?,
                at(channel, low_start)?.min(floor),
                at(channel, low_end)?.min(floor),
                at(channel, fade_out)?,
            ];
            // Keys of another take that fall inside this plateau drop to its floor, so the
            // envelope stays the lowest of both.
            let inside: Vec<Keyframe> = channel
                .keys()
                .iter()
                .filter(|key| key.time > low_start && key.time < low_end)
                .cloned()
                .collect();
            for (time, value) in local.iter().zip(targets) {
                channel.put(Keyframe::new(*time, ParamValue::Scalar(value)))?;
            }
            for mut key in inside {
                if scalar(&key.value)? > floor {
                    key.value = ParamValue::Scalar(floor);
                    channel.put(key)?;
                }
            }
            target.animations.set(path.clone(), param)?;
        }
    }
    Ok(())
}

fn locate(data: &ProjectData, id: ItemId) -> Result<&Item> {
    data.scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .filter(|item| narration(item).is_some())
        .ok_or(EditError::Invalid("item is not a narration"))
}

/// One enabled narration's ducking window in the active scene.
struct Duck {
    id: ItemId,
    start: Tick,
    end: Tick,
    ducking: NarrationDucking,
}
fn ducks(data: &ProjectData) -> Result<Vec<Duck>> {
    let mut ducks = Vec::new();
    for item in data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
    {
        if let Some(record) = narration(item).filter(|record| record.ducking.enabled) {
            ducks.push(Duck {
                id: item.id,
                start: item.start,
                end: item.end()?,
                ducking: record.ducking,
            });
        }
    }
    ducks.sort_by_key(|duck| (duck.start, duck.id));
    Ok(ducks)
}
/// Remove every key any narration's current ducking wrote.
fn clear_all(data: &mut ProjectData) -> Result<()> {
    for duck in ducks(data)? {
        clear_ducking(data, breakpoints(duck.start, duck.end, &duck.ducking)?)?;
    }
    Ok(())
}
/// Write every narration's ducking again, in timeline order, so overlaps resolve to the
/// lowest gain whatever order the edits happened in.
fn write_all(data: &mut ProjectData) -> Result<()> {
    for duck in ducks(data)? {
        write_ducking(
            data,
            breakpoints(duck.start, duck.end, &duck.ducking)?,
            &duck.ducking,
        )?;
    }
    Ok(())
}

/// Replace one narration's ducking and rewrite the automation every narration owns.
pub(in crate::session::edit) fn execute_set_ducking(
    mut data: ProjectData,
    id: ItemId,
    ducking: NarrationDucking,
) -> Result<ProjectData> {
    locate(&data, id)?;
    clear_all(&mut data)?;
    let record =
        narration_mut(item(&mut data, id)?).ok_or(EditError::Invalid("item is not a narration"))?;
    record.ducking = ducking;
    record.validate()?;
    write_all(&mut data)?;
    Ok(data)
}

/// Whether the timeline is silent over `[from, to)` apart from `ignore`: no unmuted audio
/// item and no video item with its own audio overlaps it.
pub(super) fn silent_between(data: &ProjectData, from: Tick, to: Tick, ignore: ItemId) -> bool {
    !data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .filter(|other| other.id != ignore)
        .any(|other| {
            let audible = match &other.content {
                ItemParams::Audio(params) => !params.muted,
                ItemParams::Video(params) => params.source_audio_enabled && !params.muted,
                _ => false,
            };
            audible && other.start < to && other.end().is_ok_and(|end| end > from)
        })
}

/// The take must end within the overflow limit of its cue and, past the cue, over silence.
fn check_overflow(data: &ProjectData, id: ItemId) -> Result<()> {
    let target = locate(data, id)?;
    let cue = narration(target)
        .map(|narration| narration.cue_duration)
        .ok_or(EditError::Invalid("item is not a narration"))?;
    let duration = target.duration()?;
    if duration <= cue {
        return Ok(());
    }
    let allowed = i128::from(cue.0) * OVERFLOW_NUMERATOR / OVERFLOW_DENOMINATOR;
    let cue_end = target
        .start
        .checked_add(cue)
        .ok_or(EditError::Invalid("dub time"))?;
    if i128::from(duration.0) > allowed || !silent_between(data, cue_end, target.end()?, id) {
        return Err(EditError::DurationFit);
    }
    Ok(())
}

/// Change a narration's stretch within the fit limits and move its ducking with it.
pub(in crate::session::edit) fn execute_set_fit(
    mut data: ProjectData,
    id: ItemId,
    speed: time::Speed,
) -> Result<ProjectData> {
    let thousandths = speed.thousandths();
    if thousandths != 1000
        && !(STRETCH_MIN_THOUSANDTHS..=STRETCH_MAX_THOUSANDTHS).contains(&thousandths)
    {
        return Err(EditError::DurationFit);
    }
    locate(&data, id)?;
    clear_all(&mut data)?;
    super::super::retime::set_speed(&mut data, id, speed, true)?;
    check_overflow(&data, id)?;
    write_all(&mut data)?;
    Ok(data)
}
