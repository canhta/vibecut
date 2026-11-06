//! Constant retime of media items: the single source/output time mapping.
//!
//! An item's source window (`source_range`) is authoritative. Its output
//! duration starts with [`Speed::output_duration`] (floor). A split retains that
//! mapping and slices its output window, including fractional source-tick phase.

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
    /// Retained mapping and audio processing window after a cut. New speed
    /// edits establish a new window; splitting only advances its output offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<RetimeWindow>,
}

/// Original source processing window and a half-open output slice of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetimeWindow {
    pub source_range: SourceRange,
    pub offset: Tick,
    pub duration: Tick,
}

impl Default for Retime {
    /// 1x with pitch preserved: the reference editor's "change pitch" switch starts off.
    fn default() -> Self {
        Self {
            speed: Speed::UNITY,
            maintain_pitch: true,
            window: None,
        }
    }
}

/// One piece of an item after a trim or split: its source window and output start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub source_range: SourceRange,
    pub start: Tick,
    pub retime: Retime,
}

fn overflow() -> Error {
    Error::Invalid("retime time overflow")
}

impl Item {
    /// Source processing window and output slice used by both video and audio.
    pub fn retime_window(&self) -> Result<RetimeWindow> {
        Ok(self.retime.window.unwrap_or(RetimeWindow {
            source_range: self.source_range,
            offset: Tick::ZERO,
            duration: self
                .retime
                .speed
                .output_duration(self.source_range.duration())?,
        }))
    }

    pub(crate) fn validate_retime_window(&self) -> Result<()> {
        if let Some(w) = self.retime.window {
            let end = w.offset.checked_add(w.duration).ok_or_else(overflow)?;
            let total = self
                .retime
                .speed
                .output_duration(w.source_range.duration())?;
            if w.offset < Tick::ZERO
                || w.duration <= Tick::ZERO
                || end > total
                || w.source_range.start() < Tick::ZERO
                || self.source_range.start() < w.source_range.start()
                || self.source_range.end() > w.source_range.end()
                || self.source_time_at(self.start)? != self.source_range.start()
                || self.source_range.end()
                    != if end == total {
                        w.source_range.end()
                    } else {
                        self.source_time_at(self.end()?)?
                    }
            {
                return Err(Error::Invalid("retime continuation window"));
            }
        }
        Ok(())
    }
    /// Output duration: the retained slice, or the source span played at speed.
    ///
    /// # Errors
    ///
    /// Returns an error when the result does not fit `i64`.
    pub fn duration(&self) -> Result<Tick> {
        if let Some(w) = self.retime.window {
            return Ok(w.duration);
        }
        Ok(self
            .retime
            .speed
            .output_duration(self.source_range.duration())?)
    }

    /// Source tick shown at timeline tick `at`, floored through the retained
    /// mapping: a frame is selected by `pts <= position`.
    ///
    /// # Errors
    ///
    /// Returns an error on arithmetic overflow.
    pub fn source_time_at(&self, at: Tick) -> Result<Tick> {
        let local = at.checked_sub(self.start).ok_or_else(overflow)?;
        let window = self.retime_window()?;
        let local = local.checked_add(window.offset).ok_or_else(overflow)?;
        let offset = self.retime.speed.source_offset(local, Rounding::Floor)?;
        window
            .source_range
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
        let window = self.retime_window()?;
        let offset = source
            .checked_sub(window.source_range.start())
            .ok_or_else(overflow)?;
        let local = self.retime.speed.output_offset(offset, Rounding::Ceil)?;
        local
            .checked_sub(window.offset)
            .and_then(|t| self.start.checked_add(t))
            .ok_or_else(overflow)
    }

    /// Split at a timeline tick strictly inside the item. Both pieces retain
    /// the original mapping and exactly partition its output and source windows.
    ///
    /// # Errors
    ///
    /// Returns an error when `at` is outside the item or a piece would play for
    /// no output ticks.
    pub fn split_at(&self, at: Tick) -> Result<(Cut, Cut)> {
        if at <= self.start || at >= self.end()? {
            return Err(Error::Invalid("split must be inside item"));
        }
        let left_output = at.checked_sub(self.start).ok_or_else(overflow)?;
        let cut = self.source_time_at(at)?;
        let right_source = SourceRange::new(cut, self.source_range.end());
        let left_source = SourceRange::new(self.source_range.start(), cut);
        let (Ok(left_source), Ok(right_source)) = (left_source, right_source) else {
            return Err(Error::Invalid("split leaves an empty piece"));
        };
        Ok((
            Cut {
                source_range: left_source,
                start: self.start,
                retime: self.slice_retime(Tick::ZERO, left_output)?,
            },
            Cut {
                source_range: right_source,
                start: at,
                retime: self.slice_retime(
                    left_output,
                    self.duration()?
                        .checked_sub(left_output)
                        .ok_or_else(overflow)?,
                )?,
            },
        ))
    }

    fn slice_retime(&self, offset: Tick, duration: Tick) -> Result<Retime> {
        if !matches!(
            self.content,
            crate::ItemParams::Video(_) | crate::ItemParams::Audio(_)
        ) {
            return Ok(self.retime);
        }
        let w = self.retime_window()?;
        Ok(Retime {
            window: Some(RetimeWindow {
                source_range: w.source_range,
                offset: w.offset.checked_add(offset).ok_or_else(overflow)?,
                duration,
            }),
            ..self.retime
        })
    }

    /// Move the start edge to timeline tick `at` (earlier extends the source).
    /// The end edge never moves; the start lands at or after `at`.
    ///
    /// # Errors
    ///
    /// Returns an error when the trimmed item would play for no output ticks.
    pub fn trim_start(&self, at: Tick) -> Result<Cut> {
        if at == self.start {
            return Ok(Cut {
                start: self.start,
                source_range: self.source_range,
                retime: self.retime,
            });
        }
        if self.retime.window.is_some() && at > self.start && at < self.end()? {
            let offset = at.checked_sub(self.start).ok_or_else(overflow)?;
            let duration = self.end()?.checked_sub(at).ok_or_else(overflow)?;
            return Ok(Cut {
                start: at,
                source_range: SourceRange::new(self.source_time_at(at)?, self.source_range.end())?,
                retime: self.slice_retime(offset, duration)?,
            });
        }
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
            retime: Retime {
                window: None,
                ..self.retime
            },
        })
    }

    /// Move the end edge to timeline tick `at` (later extends the source). The
    /// end lands at or before `at`; the start never moves.
    ///
    /// # Errors
    ///
    /// Returns an error when the trimmed item would play for no output ticks.
    pub fn trim_end(&self, at: Tick) -> Result<Cut> {
        if at == self.end()? {
            return Ok(Cut {
                start: self.start,
                source_range: self.source_range,
                retime: self.retime,
            });
        }
        if self.retime.window.is_some() && at > self.start && at < self.end()? {
            let duration = at.checked_sub(self.start).ok_or_else(overflow)?;
            return Ok(Cut {
                start: self.start,
                source_range: SourceRange::new(
                    self.source_range.start(),
                    self.source_time_at(at)?,
                )?,
                retime: self.slice_retime(Tick::ZERO, duration)?,
            });
        }
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
            retime: Retime {
                window: None,
                ..self.retime
            },
        })
    }
}
