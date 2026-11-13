//! Persisted imported look identities.
use crate::{Error, Result};
use captions::NfcText;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

/// Canonical lowercase BLAKE3 of the normalized cube file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LutId([u8; 32]);
impl schemars::JsonSchema for LutId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LutId".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"string","minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"})
    }
}
impl LutId {
    pub const fn new(hash: media::ContentHash) -> Self {
        Self(hash.0)
    }
    pub fn hex(self) -> String {
        self.0.iter().map(|v| format!("{v:02x}")).collect()
    }
}
impl fmt::Debug for LutId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[LUT identity]")
    }
}
impl Serialize for LutId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}
impl<'de> Deserialize<'de> for LutId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() != 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(serde::de::Error::custom("invalid LUT identity"));
        }
        let (pairs, _) = text.as_bytes().as_chunks::<2>();
        let mut bytes = [0; 32];
        for (out, pair) in bytes.iter_mut().zip(pairs) {
            let digit = |b: u8| {
                if b.is_ascii_digit() {
                    b - b'0'
                } else {
                    b - b'a' + 10
                }
            };
            *out = digit(pair.first().copied().unwrap_or_default()) * 16
                + digit(pair.get(1).copied().unwrap_or_default());
        }
        Ok(Self(bytes))
    }
}

/// A copied immutable bundle. Paths and provenance are private user data.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LutRecord {
    pub id: LutId,
    pub name: NfcText,
    pub licence: Option<NfcText>,
    pub source_name: NfcText,
    pub source_hash: media::ContentHash,
    pub directory: PathBuf,
}
impl fmt::Debug for LutRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LutRecord { [private] }")
    }
}
impl LutRecord {
    /// The three immutable files a bundle directory holds: the canonical cube
    /// (whose BLAKE3 is the id), the copied original and the provenance record.
    pub const FILES: [&'static str; 3] = ["canonical.cube", "original", "provenance.json"];
    /// Structural validation only; render/import owners verify actual bytes.
    pub fn validate(&self) -> Result<()> {
        if self.name.as_str().trim().is_empty()
            || self.name.as_str().chars().any(char::is_control)
            || self.name.as_str().len() > 256
            || self.source_name.as_str().is_empty()
            || self.source_name.as_str().len() > 256
            || self
                .licence
                .as_ref()
                .is_some_and(|s| s.as_str().len() > 1024)
            || !self.directory.is_absolute()
            || self.directory.to_str().is_none()
            || self.directory.as_os_str().len() > 4096
        {
            return Err(Error::Invalid("LUT record"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LutReference {
    #[serde(deserialize_with = "required_lut")]
    #[schemars(required)]
    pub lut: Option<LutId>,
}
fn required_lut<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<LutId>, D::Error> {
    Option::<LutId>::deserialize(d)
}
