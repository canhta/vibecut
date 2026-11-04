//! MSB-first bit reading for codec configuration records.
//!
//! Only the read side is needed: sample entries and audio-specific configs are
//! parsed, never generated bit by bit (the muxer writes whole bytes).

use std::fmt;

/// Failure from a bit read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitError {
    /// Reading past the end of the buffer.
    Eof,
    /// A field wider than 32 bits.
    InvalidWidth,
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Eof => "unexpected end of bitstream",
            Self::InvalidWidth => "bit field wider than 32 bits",
        })
    }
}

impl std::error::Error for BitError {}

pub type BitResult<T> = std::result::Result<T, BitError>;

/// MSB-first bit reader over a byte slice.
#[derive(Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Bits not yet consumed.
    pub fn bits_left(&self) -> usize {
        self.data.len().saturating_mul(8).saturating_sub(self.pos)
    }

    /// Read the next `n` bits as the low bits of a `u32`.
    pub fn read_bits(&mut self, n: u32) -> BitResult<u32> {
        if n > 32 {
            return Err(BitError::InvalidWidth);
        }
        if n as usize > self.bits_left() {
            return Err(BitError::Eof);
        }
        let mut value = 0u32;
        for _ in 0..n {
            let byte = self.pos / 8;
            let shift = 7 - self.pos % 8;
            let bit = self
                .data
                .get(byte)
                .map(|b| (b >> shift) & 1)
                .ok_or(BitError::Eof)?;
            value = (value << 1) | u32::from(bit);
            self.pos += 1;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields_msb_first() {
        let data = [0b1010_1100, 0b0101_0011];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_bits(1).unwrap(), 1);
        assert_eq!(r.read_bits(3).unwrap(), 0b010);
        assert_eq!(r.read_bits(8).unwrap(), 0b1100_0101);
        assert_eq!(r.bits_left(), 4);
        assert_eq!(r.read_bits(5), Err(BitError::Eof));
        assert_eq!(r.read_bits(0).unwrap(), 0);
        let mut r = BitReader::new(&[]);
        assert_eq!(r.read_bits(33), Err(BitError::InvalidWidth));
        assert_eq!(r.read_bits(1), Err(BitError::Eof));
    }
}
