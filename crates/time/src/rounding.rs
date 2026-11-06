//! Explicit rounding for rational conversions.

use crate::TimeError;
use serde::{Deserialize, Serialize};

/// Rounding applied when a conversion falls between two ticks or units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rounding {
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceil,
    /// To the nearest value, with ties away from zero.
    Nearest,
}

/// Divide after normalizing a negative denominator, applying `rounding`.
pub(crate) fn divide_rounded(
    numerator: i128,
    denominator: i128,
    rounding: Rounding,
) -> Result<i128, TimeError> {
    if denominator == 0 {
        return Err(TimeError::InvalidRate);
    }
    let (numerator, denominator) = if denominator < 0 {
        (
            numerator.checked_neg().ok_or(TimeError::Overflow)?,
            denominator.checked_neg().ok_or(TimeError::Overflow)?,
        )
    } else {
        (numerator, denominator)
    };
    let quotient = numerator.div_euclid(denominator);
    let remainder = numerator.rem_euclid(denominator);
    match rounding {
        Rounding::Floor => Ok(quotient),
        Rounding::Ceil if remainder != 0 => quotient.checked_add(1).ok_or(TimeError::Overflow),
        Rounding::Ceil => Ok(quotient),
        Rounding::Nearest => {
            let doubled = remainder.checked_mul(2).ok_or(TimeError::Overflow)?;
            if doubled < denominator || (doubled == denominator && numerator < 0) {
                Ok(quotient)
            } else {
                quotient.checked_add(1).ok_or(TimeError::Overflow)
            }
        }
    }
}

pub(crate) fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}
