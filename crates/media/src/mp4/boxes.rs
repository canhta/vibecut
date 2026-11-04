//! Box headers, bounded payload readers, a box iterator and a box builder.

use super::FourCc;
use crate::{MediaError, Result};

/// Bounds-checked big-endian cursor over a byte slice.
///
/// A failed read leaves the position unchanged and returns
/// [`MediaError::Truncated`] naming the data.
#[derive(Clone, Debug)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    pub(crate) fn labeled(data: &'a [u8], what: &'static str) -> Self {
        Self { data, pos: 0, what }
    }

    /// Bytes left to read.
    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// The unread bytes without consuming them.
    pub(crate) fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }

    /// Read `n` bytes.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(MediaError::Truncated(self.what))?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(MediaError::Truncated(self.what))?;
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    /// Read `N` bytes into an array.
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let slice = self.bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        let [value] = self.array::<1>()?;
        Ok(value)
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(crate) fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    pub(crate) fn u24(&mut self) -> Result<u32> {
        let [a, b, c] = self.array::<3>()?;
        Ok(u32::from_be_bytes([0, a, b, c]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub(crate) fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    pub(crate) fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }

    pub(crate) fn fourcc(&mut self) -> Result<FourCc> {
        Ok(FourCc(self.array()?))
    }

    /// Full-box header: `(version, flags)`.
    pub(crate) fn full_header(&mut self) -> Result<(u8, u32)> {
        let value = self.u32()?;
        Ok(((value >> 24) as u8, value & 0x00FF_FFFF))
    }

    /// Read a 32-bit entry count after checking that `count * entry_size` bytes remain.
    pub(crate) fn count(&mut self, entry_size: usize) -> Result<usize> {
        let count = self.u32()? as usize;
        self.check_count(count, entry_size)?;
        Ok(count)
    }

    /// Check that `n * entry_size` bytes remain, so a hostile count cannot drive a
    /// large allocation.
    pub(crate) fn check_count(&self, n: usize, entry_size: usize) -> Result<()> {
        match n.checked_mul(entry_size) {
            Some(bytes) if bytes <= self.remaining() => Ok(()),
            _ => Err(MediaError::Truncated(self.what)),
        }
    }
}

/// A parsed top-level or nested box header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxHeader {
    pub(crate) kind: FourCc,
    /// Header length: 8, plus 8 for a 64-bit `largesize`, plus 16 for a `uuid` user type.
    pub(crate) header_len: u64,
    /// Total box size including the header; `None` means the box runs to the end.
    pub(crate) size: Option<u64>,
}

impl BoxHeader {
    /// Longest possible header.
    pub(crate) const MAX_LEN: usize = 32;

    /// Parse a header from the start of `buf`. Errors on a truncated header or a
    /// size smaller than the header it describes.
    pub(crate) fn parse(buf: &[u8]) -> Result<Self> {
        let mut cursor = Reader::labeled(buf, "box header");
        let size32 = cursor.u32()?;
        let kind = cursor.fourcc()?;
        let mut header_len = 8u64;
        let size = match size32 {
            0 => None,
            1 => {
                header_len += 8;
                Some(cursor.u64()?)
            }
            value => Some(u64::from(value)),
        };
        if kind == b"uuid" {
            header_len += 16;
            cursor.skip(16)?;
        }
        if let Some(size) = size
            && size < header_len
        {
            return Err(MediaError::Invalid(format!(
                "box '{kind}' size {size} smaller than its header"
            )));
        }
        Ok(Self {
            kind,
            header_len,
            size,
        })
    }
}

/// One box found by [`boxes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxRef<'a> {
    pub(crate) kind: FourCc,
    pub(crate) payload: &'a [u8],
}

/// Iterator over sibling boxes in a byte slice.
///
/// A malformed header or a box larger than the remaining bytes yields one error
/// and ends the iteration. A trailing run of fewer than eight zero bytes (the
/// QuickTime terminator) ends it silently.
pub(crate) struct BoxIter<'a> {
    data: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> BoxIter<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            done: false,
        }
    }

    fn step(&mut self) -> Option<Result<BoxRef<'a>>> {
        let rest = self.data.get(self.pos..).unwrap_or(&[]);
        if rest.is_empty() {
            return None;
        }
        if rest.len() < 8 {
            if rest.iter().all(|&b| b == 0) {
                return None;
            }
            return Some(Err(MediaError::Truncated("box header")));
        }
        let header = match BoxHeader::parse(rest) {
            Ok(header) => header,
            Err(error) => return Some(Err(error)),
        };
        let available = rest.len() as u64;
        let size = match header.size {
            None => available,
            Some(size) if size <= available => size,
            Some(size) => {
                return Some(Err(MediaError::Invalid(format!(
                    "box '{}' size {size} exceeds its container ({available} bytes left)",
                    header.kind
                ))));
            }
        };
        let payload = header.header_len;
        let payload = match (usize::try_from(payload), usize::try_from(size)) {
            (Ok(start), Ok(end)) => rest.get(start..end),
            _ => None,
        };
        let Some(payload) = payload else {
            return Some(Err(MediaError::Truncated("box payload")));
        };
        self.pos = self.pos.saturating_add(size as usize);
        Some(Ok(BoxRef {
            kind: header.kind,
            payload,
        }))
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = Result<BoxRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let item = self.step();
        if !matches!(item, Some(Ok(_))) {
            self.done = true;
        }
        item
    }
}

/// Iterate the boxes in `data`.
pub(crate) fn boxes(data: &[u8]) -> BoxIter<'_> {
    BoxIter::new(data)
}

/// Payload of the first box of type `kind`, if any, before a malformed box.
pub(crate) fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data)
        .map_while(|b| b.ok())
        .find(|b| b.kind == kind)
        .map(|b| b.payload)
}

/// Growable buffer for building boxes.
#[derive(Default, Clone, Debug)]
pub(crate) struct BoxBuf {
    pub(crate) buf: Vec<u8>,
}

impl BoxBuf {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Begin a box; the returned marker goes to [`end`](Self::end).
    pub(crate) fn start(&mut self, kind: &[u8; 4]) -> usize {
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.buf.extend_from_slice(kind);
        at
    }

    /// Begin a full box with a version and flags.
    pub(crate) fn start_full(&mut self, kind: &[u8; 4], version: u8, flags: u32) -> usize {
        let at = self.start(kind);
        self.u32((u32::from(version) << 24) | (flags & 0xFF_FFFF));
        at
    }

    /// Finish a box begun at `at`, writing its size.
    pub(crate) fn end(&mut self, at: usize) {
        let size = self.buf.len().saturating_sub(at) as u32;
        if let Some(slot) = self.buf.get_mut(at..at.saturating_add(4)) {
            slot.copy_from_slice(&size.to_be_bytes());
        }
    }

    /// Append a complete box with the given payload.
    pub(crate) fn leaf(&mut self, kind: &[u8; 4], payload: &[u8]) {
        let at = self.start(kind);
        self.bytes(payload);
        self.end(at);
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn i16(&mut self, value: i16) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn i32(&mut self, value: i32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn zeros(&mut self, n: usize) {
        self.buf.resize(self.buf.len().saturating_add(n), 0);
    }

    pub(crate) fn len(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_forms_and_hostile_sizes() {
        let h = BoxHeader::parse(&[0, 0, 0, 16, b'f', b'r', b'e', b'e']).unwrap();
        assert_eq!(
            (h.kind, h.header_len, h.size),
            (FourCc(*b"free"), 8, Some(16))
        );
        let h = BoxHeader::parse(&[0, 0, 0, 0, b'm', b'd', b'a', b't']).unwrap();
        assert_eq!((h.header_len, h.size), (8, None));
        let mut large = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        large.extend_from_slice(&100u64.to_be_bytes());
        assert_eq!(BoxHeader::parse(&large).unwrap().size, Some(100));
        assert!(BoxHeader::parse(&[0, 0, 0, 4, b'f', b'r', b'e', b'e']).is_err());
        assert!(BoxHeader::parse(&[0, 0, 0]).is_err());
        let mut uuid = vec![0, 0, 0, 26, b'u', b'u', b'i', b'd'];
        uuid.extend_from_slice(&[7u8; 16]);
        assert_eq!(BoxHeader::parse(&uuid).unwrap().header_len, 24);
        // A uuid box whose size does not cover its user type.
        let mut short_uuid = vec![0, 0, 0, 20, b'u', b'u', b'i', b'd'];
        short_uuid.extend_from_slice(&[7u8; 16]);
        assert!(BoxHeader::parse(&short_uuid).is_err());
        for hostile in [
            &[0xFF, 0xFF, 0xFF, 0xFF, b'a', b'b', b'c', b'd'][..],
            &[
                0, 0, 0, 1, b'a', b'b', b'c', b'd', 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            ],
            &[0, 0, 0, 7, b'a', b'b', b'c', b'd'],
        ] {
            let _ = BoxHeader::parse(hostile);
        }
    }

    #[test]
    fn iterates_children_and_stops_at_the_first_error() {
        let mut b = BoxBuf::new();
        b.leaf(b"aaaa", &[1, 2]);
        let outer = b.start(b"bbbb");
        b.leaf(b"cccc", &[]);
        b.end(outer);
        b.u32(0);
        let kids: Vec<_> = boxes(&b.buf).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].payload, &[1, 2]);
        assert_eq!(kids[1].payload.len(), 8);
        assert_eq!(find(&b.buf, b"bbbb").map(<[u8]>::len), Some(8));
        let bad = [0u8, 0, 0, 99, b'x', b'x', b'x', b'x', 0];
        let mut it = boxes(&bad);
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
        assert_eq!(boxes(&[0, 0, 0, 0]).count(), 0);
        assert_eq!(boxes(&[0, 1, 2]).count(), 1);
    }

    #[test]
    fn reader_bounds_and_counts() {
        let mut c = Reader::labeled(&[0, 0, 0, 5, 1], "t");
        assert!(c.count(1).is_err());
        assert_eq!(c.rest(), &[1]);
        assert!(c.u32().is_err());
        let mut c = Reader::labeled(&[0xFF, 0xFF, 0xFF, 0xFF], "count");
        assert!(c.count(usize::MAX).is_err());
        assert!(Reader::labeled(&[1, 2], "short").f64().is_err());
        assert!(matches!(
            Reader::labeled(&[], "x").u8(),
            Err(MediaError::Truncated("x"))
        ));
        assert_eq!(
            Reader::labeled(&[0, 0, 1, 0, 0, 0, 0x40, 1], "full")
                .full_header()
                .unwrap(),
            (0, 0x100)
        );
    }
}
