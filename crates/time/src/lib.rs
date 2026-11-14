//! Exact integer media time: ticks, rational frame rates and source ranges, plus the
//! [`wake`] hook a worker thread uses to ask the UI loop to repaint.
//!
//! Time is an integer count of [`Tick`]s at [`TICKS_PER_SECOND`] = 254 016 000 000
//! per second. That rate divides evenly into every admitted broadcast frame
//! duration and common audio sample duration, so frame and sample conversions are
//! exact. Floating seconds appear only at display and provider boundaries.
//!
//! Fallible operations report [`TimeError`]; hostile input never panics.

mod clock;
mod range;
mod rate;
mod rounding;
mod speed;
mod tick;
mod timecode;
pub mod wake;

pub use clock::{format_clock, format_duration, format_short_clock};
pub use range::{SourceRange, TimeRange};
pub use rate::FrameRate;
pub use rounding::Rounding;
pub use speed::Speed;
pub use tick::{TICKS_PER_SECOND, Tick};
pub use timecode::{format_timecode, parse_timecode};

pub(crate) use rounding::{divide_rounded, gcd};

use std::fmt;

/// Failure from a time conversion or range constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// The frame rate or timecode mode is unsupported.
    Unsupported,
    /// A timecode label is malformed, out of bounds or skipped in DF counting.
    InvalidTimecode,
    /// Floating seconds were NaN or infinite.
    NonFiniteSeconds,
    /// A rate, denominator or timebase was zero or negative.
    InvalidRate,
    /// A range end was not greater than its start.
    EmptyRange,
    /// An intermediate value exceeded its storage width.
    Overflow,
    /// A representable intermediate result did not fit the return type.
    OutOfRange,
    /// A playback speed was not finite or fell outside 0.1x..=100x.
    InvalidSpeed,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unsupported => "unsupported timecode rate or mode",
            Self::InvalidTimecode => "invalid or skipped timecode label",
            Self::NonFiniteSeconds => "seconds must be finite",
            Self::InvalidRate => "rate must be positive",
            Self::EmptyRange => "range end must be greater than start",
            Self::Overflow => "time arithmetic overflowed",
            Self::OutOfRange => "time value is out of range",
            Self::InvalidSpeed => "speed must be between 0.1x and 100x",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TimeError {}
