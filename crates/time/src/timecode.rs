//! Signed drop-frame and non-drop-frame editor timecode labels.

use crate::{FrameRate, Tick, TimeError};

/// Format the frame containing `time` as `[-]HH:MM:SS:FF` (NDF) or
/// `[-]HH:MM:SS;FF` (DF). Negative times use signed frame indices, floored
/// toward negative infinity. Hours extend beyond 23 without wrapping.
///
/// Supports 23.976, 24, 25, 29.97, 30, 48, 50, 59.94 and 60 fps. The rate
/// decides the mode: 29.97 and 59.94 are drop-frame, every other supported
/// rate is non-drop-frame. Returns `Unsupported` otherwise, and propagates
/// frame conversion errors. This is an editor label, not LTC.
pub fn format_timecode(time: Tick, rate: FrameRate) -> Result<String, TimeError> {
    let (base, drop) = parameters(rate)?;
    let frame = rate.frame_at(time)?;
    let mut count = i128::from(frame).abs();
    if drop != 0 {
        let short_minute = base * 60 - drop;
        let ten_minutes = base * 600 - drop * 9;
        let blocks = count / ten_minutes;
        let remainder = count % ten_minutes;
        count += drop * 9 * blocks + drop * ((remainder - drop).max(0) / short_minute);
    }
    let frames = count % base;
    let seconds = count / base % 60;
    let minutes = count / (base * 60) % 60;
    let hours = count / (base * 3600);
    let sign = if frame < 0 { "-" } else { "" };
    let separator = if drop != 0 { ';' } else { ':' };
    Ok(format!(
        "{sign}{hours:02}:{minutes:02}:{seconds:02}{separator}{frames:02}"
    ))
}

/// Parse an absolute signed editor frame label to its frame's start tick.
///
/// Accepts exactly the format produced by [`format_timecode`]: at least two
/// hour digits and two digits for all other fields, with no whitespace or `+`.
/// Hours do not wrap. Negative zero is accepted and normalizes to zero.
/// The rate decides the mode, with the same limits as the formatter. Returns
/// `InvalidTimecode` for malformed, out-of-bounds or skipped DF labels,
/// `OutOfRange` for numeric overflow, and propagates tick conversion errors. A
/// label whose frame begins outside the tick range is refused even if part of
/// that frame is representable.
pub fn parse_timecode(input: &str, rate: FrameRate) -> Result<Tick, TimeError> {
    let (base, drop) = parameters(rate)?;
    // The largest representable tick needs fewer than 20 hour digits.
    if input.len() > 32 {
        return Err(TimeError::OutOfRange);
    }
    let (negative, body) = input
        .strip_prefix('-')
        .map_or((false, input), |s| (true, s));
    let mut fields = body.split([':', ';']);
    let mut next = || fields.next().ok_or(TimeError::InvalidTimecode);
    let hours = next()?;
    let minutes = next()?;
    let seconds = next()?;
    let frames = next()?;
    if fields.next().is_some()
        || hours.len() < 2
        || minutes.len() != 2
        || seconds.len() != 2
        || frames.len() != 2
    {
        return Err(TimeError::InvalidTimecode);
    }
    let separator = if drop != 0 { ';' } else { ':' };
    if body != format!("{hours}:{minutes}:{seconds}{separator}{frames}") {
        return Err(TimeError::InvalidTimecode);
    }
    let number = |s: &str| -> Result<i128, TimeError> {
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(TimeError::InvalidTimecode);
        }
        s.parse().map_err(|_| TimeError::OutOfRange)
    };
    let (h, m, s, f) = (
        number(hours)?,
        number(minutes)?,
        number(seconds)?,
        number(frames)?,
    );
    if m >= 60 || s >= 60 || f >= base || (drop != 0 && m % 10 != 0 && s == 0 && f < drop) {
        return Err(TimeError::InvalidTimecode);
    }
    // Input length bounds h to at most 23 digits, safely inside i128 arithmetic.
    let total_minutes = h * 60 + m;
    let count = (total_minutes * 60 + s) * base + f - drop * (total_minutes - total_minutes / 10);
    let signed = if negative { -count } else { count };
    rate.tick_of(i64::try_from(signed).map_err(|_| TimeError::OutOfRange)?)
}

fn parameters(rate: FrameRate) -> Result<(i128, i128), TimeError> {
    match (rate.num(), rate.den()) {
        (30_000, 1_001) => Ok((30, 2)),
        (60_000, 1_001) => Ok((60, 4)),
        (24_000, 1_001) => Ok((24, 0)),
        (n @ (24 | 25 | 30 | 48 | 50 | 60), 1) => Ok((i128::from(n), 0)),
        _ => Err(TimeError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rollover_tables() {
        for (rate, scale) in [(FrameRate::FPS_29_97, 1), (FrameRate::FPS_59_94, 2)] {
            for (frame, suffix) in [
                (0, "00:00:00;00"),
                (1799, "00:00:59;29"),
                (1800, "00:01:00;02"),
                (17982, "00:10:00;00"),
                (107892, "01:00:00;00"),
                (2589408, "24:00:00;00"),
            ] {
                let label = if scale == 1 {
                    suffix.to_owned()
                } else {
                    let (prefix, field) = suffix.rsplit_once(';').unwrap();
                    format!("{prefix};{:02}", field.parse::<i64>().unwrap() * scale)
                };
                let tick = rate.tick_of(frame * scale).unwrap();
                assert_eq!(format_timecode(tick, rate).unwrap(), label);
                assert_eq!(parse_timecode(&label, rate).unwrap(), tick);
                if frame > 0 {
                    let negative = rate.tick_of(-frame * scale).unwrap();
                    assert_eq!(
                        parse_timecode(&format!("-{label}"), rate).unwrap(),
                        negative
                    );
                    assert_eq!(
                        format_timecode(negative, rate).unwrap(),
                        format!("-{label}")
                    );
                }
            }
        }
        for (rate, base) in [
            (FrameRate::FPS_23_976, 24),
            (FrameRate::FPS_24, 24),
            (FrameRate::FPS_25, 25),
            (FrameRate::FPS_30, 30),
            (FrameRate::FPS_60, 60),
        ] {
            for (frame, label) in [
                (base - 1, format!("00:00:00:{:02}", base - 1)),
                (base * 60, "00:01:00:00".to_owned()),
                (base * 3600, "01:00:00:00".to_owned()),
            ] {
                let tick = rate.tick_of(frame).unwrap();
                assert_eq!(format_timecode(tick, rate).unwrap(), label);
                assert_eq!(parse_timecode(&label, rate).unwrap(), tick);
            }
        }
    }

    #[test]
    fn successive_df_labels_match_independent_counter() {
        for (rate, base, drop) in [(FrameRate::FPS_29_97, 30, 2), (FrameRate::FPS_59_94, 60, 4)] {
            let (mut m, mut s, mut f) = (0, 0, 0);
            for frame in 0..(base * 600 - drop * 9 + 1) {
                let label = format!("00:{m:02}:{s:02};{f:02}");
                let tick = rate.tick_of(frame).unwrap();
                assert_eq!(format_timecode(tick, rate).unwrap(), label);
                assert_eq!(parse_timecode(&label, rate).unwrap(), tick);
                f += 1;
                if f == base {
                    f = 0;
                    s += 1;
                    if s == 60 {
                        s = 0;
                        m += 1;
                        if m % 10 != 0 {
                            f = drop;
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hostile_labels_and_limits() {
        for input in [
            "",
            "00",
            "00:00:00",
            "00:00:00:00:00",
            "00;00:00;00",
            "+00:00:00;00",
            " 00:00:00;00",
            "00:00:00;00\n",
            "00:60:00;00",
            "00:00:60;00",
            "00:00:00;30",
            "00:01:00;00",
            "00:01:00;01",
            "00:09:00;01",
            "00:01:00:02",
            "00:0:00;00",
            "00:00:00;０",
        ] {
            assert_eq!(
                parse_timecode(input, FrameRate::FPS_29_97),
                Err(TimeError::InvalidTimecode),
                "{input}"
            );
        }
        // The rate fixes the mode: only that rate's separator is accepted.
        assert_eq!(
            parse_timecode("00:00:00:00", FrameRate::FPS_29_97),
            Err(TimeError::InvalidTimecode)
        );
        assert_eq!(
            parse_timecode("00:00:00;00", FrameRate::FPS_24),
            Err(TimeError::InvalidTimecode)
        );
        for f in 0..4 {
            assert_eq!(
                parse_timecode(&format!("00:01:00;{f:02}"), FrameRate::FPS_59_94),
                Err(TimeError::InvalidTimecode)
            );
        }
        for input in ["99999999999999999999:00:00;00", &"9".repeat(1000)] {
            assert_eq!(
                parse_timecode(input, FrameRate::FPS_29_97),
                Err(TimeError::OutOfRange)
            );
        }
        assert_eq!(
            parse_timecode("99999:00:00:00", FrameRate::FPS_24),
            Err(TimeError::OutOfRange)
        );
        let unsupported = FrameRate::new(7, 3).unwrap();
        assert_eq!(
            format_timecode(Tick::ZERO, unsupported),
            Err(TimeError::Unsupported)
        );
        assert_eq!(
            parse_timecode("00:00:00:00", unsupported),
            Err(TimeError::Unsupported)
        );
        assert_eq!(
            format_timecode(Tick(-1), FrameRate::FPS_24).unwrap(),
            "-00:00:00:01"
        );
        for rate in [
            FrameRate::FPS_24,
            FrameRate::FPS_29_97,
            FrameRate::FPS_59_94,
        ] {
            for tick in [Tick(i64::MIN), Tick(i64::MAX)] {
                let label = format_timecode(tick, rate).unwrap();
                if tick.0 == i64::MIN {
                    assert_eq!(parse_timecode(&label, rate), Err(TimeError::OutOfRange));
                } else {
                    assert_eq!(
                        parse_timecode(&label, rate).unwrap(),
                        rate.snap(tick).unwrap()
                    );
                }
            }
        }
    }

    proptest! {
        #[test]
        fn frame_labels_round_trip(frame in -3_000_000_i64..3_000_000, choice in 0_usize..9) {
            let rate = [FrameRate::FPS_23_976, FrameRate::FPS_24, FrameRate::FPS_25,
                FrameRate::FPS_29_97, FrameRate::FPS_30, FrameRate::FPS_48,
                FrameRate::FPS_50, FrameRate::FPS_59_94, FrameRate::FPS_60][choice];
            let tick = rate.tick_of(frame).unwrap();
            let label = format_timecode(tick, rate).unwrap();
            prop_assert_eq!(parse_timecode(&label, rate).unwrap(), tick);
        }

        #[test]
        fn arbitrary_input_never_panics(input in ".{0,100}") {
            let _ = parse_timecode(&input, FrameRate::FPS_59_94);
        }
    }
}
