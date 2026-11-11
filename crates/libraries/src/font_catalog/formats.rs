//! Bounds font containers before a decoder can allocate from their lengths.
use super::{EditError, FontError, Result};

pub(super) fn decode(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let bytes = if bytes.starts_with(b"wOF2") {
        woff2_bounds(&bytes)?;
        wuff::decompress_woff2(&bytes).map_err(|_| FontError::InvalidImport)?
    } else {
        bytes
    };
    if bytes.len() > text::MAX_FONT_BYTES {
        return Err(EditError::Control(engine::control::ControlError::Limit));
    }
    if !matches!(bytes.get(..4), Some(b"\0\x01\0\0" | b"OTTO")) {
        return Err(FontError::InvalidImport.into());
    }
    sfnt_bounds(&bytes)?;
    Ok(bytes)
}

fn sfnt_bounds(bytes: &[u8]) -> Result<()> {
    let count = u16_at(bytes, 4)? as usize;
    if count == 0 || count > 128 {
        return Err(EditError::Control(engine::control::ControlError::Limit));
    }
    let directory_end = 12 + count * 16;
    let directory = bytes
        .get(12..directory_end)
        .ok_or(FontError::InvalidImport)?;
    let mut tags = std::collections::BTreeSet::new();
    for table in directory.chunks_exact(16) {
        let tag: [u8; 4] = table
            .get(..4)
            .ok_or(FontError::InvalidImport)?
            .try_into()
            .map_err(|_| FontError::InvalidImport)?;
        let offset = u32_at(table, 8)? as usize;
        let length = u32_at(table, 12)? as usize;
        if !tags.insert(tag)
            || offset < directory_end
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(FontError::InvalidImport.into());
        }
    }
    if ![b"head", b"maxp", b"cmap", b"hhea", b"hmtx", b"name"]
        .iter()
        .all(|tag| tags.contains(*tag))
        || !(tags.contains(b"glyf") && tags.contains(b"loca")
            || tags.contains(b"CFF ")
            || tags.contains(b"CFF2"))
    {
        return Err(FontError::InvalidImport.into());
    }
    Ok(())
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(FontError::InvalidImport)?;
    Ok(u32::from_be_bytes(
        value.try_into().map_err(|_| FontError::InvalidImport)?,
    ))
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or(FontError::InvalidImport)?
            .try_into()
            .map_err(|_| FontError::InvalidImport)?,
    ))
}

fn base128(bytes: &[u8], offset: &mut usize) -> Result<u32> {
    let mut value = 0u32;
    for i in 0..5 {
        let byte = *bytes.get(*offset).ok_or(FontError::InvalidImport)?;
        *offset += 1;
        if (i == 0 && byte == 0x80) || value & 0xfe00_0000 != 0 {
            return Err(FontError::InvalidImport.into());
        }
        value = (value << 7) | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(FontError::InvalidImport.into())
}

fn woff2_bounds(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 48
        || !matches!(bytes.get(4..8), Some(b"\0\x01\0\0" | b"OTTO"))
        || u32_at(bytes, 8)? as usize != bytes.len()
    {
        return Err(FontError::InvalidImport.into());
    }
    let tables = u16_at(bytes, 12)? as usize;
    if tables == 0 || tables > 128 || u32_at(bytes, 16)? as usize > text::MAX_FONT_BYTES {
        return Err(EditError::Control(engine::control::ControlError::Limit));
    }
    let mut offset = 48usize;
    let mut expanded = 12 + tables * 16;
    let mut transformed = 0usize;
    for _ in 0..tables {
        let flags = *bytes.get(offset).ok_or(FontError::InvalidImport)?;
        offset += 1;
        let index = flags & 63;
        let version = flags >> 6;
        let explicit = if index == 63 {
            let tag = bytes
                .get(offset..offset + 4)
                .ok_or(FontError::InvalidImport)?;
            offset += 4;
            Some(tag)
        } else {
            None
        };
        let glyf = index == 10 || explicit == Some(b"glyf");
        let loca = index == 11 || explicit == Some(b"loca");
        let hmtx = index == 3 || explicit == Some(b"hmtx");
        let changed = if glyf || loca {
            if version != 0 && version != 3 {
                return Err(FontError::InvalidImport.into());
            }
            version == 0
        } else if hmtx && version == 1 {
            true
        } else {
            if version != 0 {
                return Err(FontError::InvalidImport.into());
            }
            false
        };
        let original = base128(bytes, &mut offset)? as usize;
        let compressed = if changed {
            base128(bytes, &mut offset)? as usize
        } else {
            original
        };
        expanded = expanded
            .checked_add(
                original
                    .checked_add(3)
                    .ok_or(EditError::Control(engine::control::ControlError::Limit))?
                    & !3,
            )
            .ok_or(EditError::Control(engine::control::ControlError::Limit))?;
        transformed = transformed
            .checked_add(compressed)
            .ok_or(EditError::Control(engine::control::ControlError::Limit))?;
        // This bounds Brotli's output and the glyph decoder's point scratch,
        // independently of the attacker-controlled totalSfntSize hint.
        if expanded > text::MAX_FONT_BYTES || transformed > 4 << 20 {
            return Err(EditError::Control(engine::control::ControlError::Limit));
        }
    }
    Ok(())
}
