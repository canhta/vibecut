//! Constant retime of media items: the single source/output time mapping.
//!
//! An item's source window (`source_range`) is authoritative. Its output
//! duration is derived with [`Speed::output_duration`] (floor), so trims and
//! splits choose a source span and place the output edge at or inside the
//! requested time instead of accumulating floating durations.

use crate::{Error, Item, Result};
use serde::{Deserialize, Serialize};
use time::{Rounding, SourceRange, Speed, Tick};

/// Playback speed and pitch policy of a video or audio item.
///
/// Every other item kind keeps the default (1x, pitch preserved).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Retime {
    pub speed: Speed,
    /// On time-stretches audio so pitch is preserved; off lets pitch follow speed.
    pub maintain_pitch: bool,
}

impl Default for Retime {
    /// 1x with pitch preserved: the reference editor's "change pitch" switch starts off.
    fn default() -> Self {
        Self {
            speed: Speed::UNITY,
            maintain_pitch: true,
        }
    }
}

/// One piece of an item after a trim or split: its source window and output start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub source_range: SourceRange,
    pub start: Tick,
}

fn overflow() -> Error {
    Error::Invalid("retime time overflow")
}

impl Item {
    /// Output duration: the source span played at the item's speed.
    ///
    /// # Errors
    ///
    /// Returns an error when the result does not fit `i64`.
    pub fn duration(&self) -> Result<Tick> {
        Ok(self
            .retime
            .speed
            .output_duration(self.source_range.duration())?)
    }

    /// Source tick shown at timeline tick `at`, floored: a frame is selected by
    /// `pts <= position`, so rounding down never shows a frame early or past the source window.
    ///
    /// # Errors
    ///
    /// Returns an error on arithmetic overflow.
    pub fn source_time_at(&self, at: Tick) -> Result<Tick> {
        let local = at.checked_sub(self.start).ok_or_else(overflow)?;
        let offset = self.retime.speed.source_offset(local, Rounding::Floor)?;
        self.source_range
            .start()
            .checked_add(offset)
            .ok_or_else(overflow)
    }

    /// First timeline tick whose source position reaches `source` (ceiling).
    ///
    /// # Errors
    ///
    /// Returns an error on arithmetic overflow.
    pub fn output_time_of(&self, source: Tick) -> Result<Tick> {
        let offset = source
            .checked_sub(self.source_range.start())
            .ok_or_else(overflow)?;
        let local = self.retime.speed.output_offset(offset, Rounding::Ceil)?;
        self.start.checked_add(local).ok_or_else(overflow)
    }

    /// Split at a timeline tick strictly inside the item. The left piece ends at
    /// or before `at`; the right piece starts exactly where the left ends and
    /// never ends after the original, so neighbours cannot be overlapped.
    ///
    /// # Errors
    ///
    /// Returns an error when `at` is outside the item or a piece would play for
    /// no output ticks.
    pub fn split_at(&self, at: Tick) -> Result<(Cut, Cut)> {
        if at <= self.start || at >= self.end()? {
            return Err(Error::Invalid("split must be inside item"));
        }
        let speed = self.retime.speed;
        let local = at.checked_sub(self.start).ok_or_else(overflow)?;
        let left_source = speed.source_within(local)?;
        let left_output = speed.output_duration(left_source)?;
        let cut = self
            .source_range
            .start()
            .checked_add(left_source)
            .ok_or_else(overflow)?;
        let right_source = SourceRange::new(cut, self.source_range.end());
        let left_source = SourceRange::new(self.source_range.start(), cut);
        let (Ok(left_source), Ok(right_source)) = (left_source, right_source) else {
            return Err(Error::Invalid("split leaves an empty piece"));
        };
        let right_start = self.start.checked_add(left_output).ok_or_else(overflow)?;
        if left_output == Tick::ZERO
            || speed.output_duration(right_source.duration())? == Tick::ZERO
        {
            return Err(Error::Invalid("split leaves an empty piece"));
        }
        Ok((
            Cut {
                source_range: left_source,
                start: self.start,
            },
            Cut {
                source_range: right_source,
                start: right_start,
            },
        ))
    }

    /// Move the start edge to timeline tick `at` (earlier extends the source).
    /// The end edge never moves; the start lands at or after `at`.
    ///
    /// # Errors
    ///
    /// Returns an error when the trimmed item would play for no output ticks.
    pub fn trim_start(&self, at: Tick) -> Result<Cut> {
        let end = self.end()?;
        let target = end.checked_sub(at).ok_or_else(overflow)?;
        if target <= Tick::ZERO {
            return Err(Error::Invalid("trim leaves an empty item"));
        }
        let speed = self.retime.speed;
        let source = speed.source_within(target)?;
        let output = speed.output_duration(source)?;
        let source_start = self
            .source_range
            .end()
            .checked_sub(source)
            .ok_or_else(overflow)?;
        let source_range = SourceRange::new(source_start, self.source_range.end())
            .map_err(|_| Error::Invalid("trim leaves an empty item"))?;
        if output == Tick::ZERO {
            return Err(Error::Invalid("trim leaves an empty item"));
        }
        Ok(Cut {
            source_range,
            start: end.checked_sub(output).ok_or_else(overflow)?,
        })
    }

    /// Move the end edge to timeline tick `at` (later extends the source). The
    /// end lands at or before `at`; the start never moves.
    ///
    /// # Errors
    ///
    /// Returns an error when the trimmed item would play for no output ticks.
    pub fn trim_end(&self, at: Tick) -> Result<Cut> {
        let target = at.checked_sub(self.start).ok_or_else(overflow)?;
        if target <= Tick::ZERO {
            return Err(Error::Invalid("trim leaves an empty item"));
        }
        let speed = self.retime.speed;
        let source = speed.source_within(target)?;
        let source_end = self
            .source_range
            .start()
            .checked_add(source)
            .ok_or_else(overflow)?;
        let source_range = SourceRange::new(self.source_range.start(), source_end)
            .map_err(|_| Error::Invalid("trim leaves an empty item"))?;
        if speed.output_duration(source)? == Tick::ZERO {
            return Err(Error::Invalid("trim leaves an empty item"));
        }
        Ok(Cut {
            source_range,
            start: self.start,
        })
    }
}
