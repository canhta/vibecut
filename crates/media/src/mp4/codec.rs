//! Sample entries (`stsd` children) and codec configuration records.
//!
//! Read support covers the fields needed to describe a stream and to remux its
//! H.264/AAC configuration; write support is limited to an `avc1` entry with
//! `avcC` and an `mp4a` entry with `esds`.

use super::FourCc;
use super::boxes::{BoxBuf, Reader, boxes};
use crate::bitstream::BitReader;
use crate::{MediaError, Result};

/// `avcC` — AVCDecoderConfigurationRecord (ISO/IEC 14496-15 §5.3.3).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AvcConfig {
    pub profile: u8,
    pub compatibility: u8,
    pub level: u8,
    /// NAL length field size in bytes (1, 2 or 4).
    pub length_size: u8,
    /// Sequence parameter sets, without start codes.
    pub sps: Vec<Vec<u8>>,
    /// Picture parameter sets, without start codes.
    pub pps: Vec<Vec<u8>>,
}

impl AvcConfig {
    /// Build from parameter sets, taking profile/compatibility/level from the first SPS.
    pub fn new(sps: Vec<Vec<u8>>, pps: Vec<Vec<u8>>, length_size: u8) -> Self {
        let header = sps.first().map(|sps| {
            (
                sps.get(1).copied(),
                sps.get(2).copied(),
                sps.get(3).copied(),
            )
        });
        let (profile, compatibility, level) = match header {
            Some((Some(profile), Some(compatibility), Some(level))) => {
                (profile, compatibility, level)
            }
            _ => (0, 0, 0),
        };
        Self {
            profile,
            compatibility,
            level,
            length_size,
            sps,
            pps,
        }
    }

    fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Reader::labeled(data, "avcC");
        let _version = c.u8()?;
        let profile = c.u8()?;
        let compatibility = c.u8()?;
        let level = c.u8()?;
        let length_size = (c.u8()? & 3) + 1;
        let sps_count = (c.u8()? & 0x1F) as usize;
        c.check_count(sps_count, 2)?;
        let mut sps = Vec::with_capacity(sps_count);
        for _ in 0..sps_count {
            let len = c.u16()? as usize;
            sps.push(c.bytes(len)?.to_vec());
        }
        let pps_count = c.u8()? as usize;
        c.check_count(pps_count, 2)?;
        let mut pps = Vec::with_capacity(pps_count);
        for _ in 0..pps_count {
            let len = c.u16()? as usize;
            pps.push(c.bytes(len)?.to_vec());
        }
        Ok(Self {
            profile,
            compatibility,
            level,
            length_size,
            sps,
            pps,
        })
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u8(1);
        b.u8(self.profile);
        b.u8(self.compatibility);
        b.u8(self.level);
        b.u8(0xFC | (self.length_size.clamp(1, 4) - 1));
        b.u8(0xE0 | (self.sps.len().min(31) as u8));
        for sps in self.sps.iter().take(31) {
            b.u16(sps.len() as u16);
            b.bytes(sps);
        }
        b.u8(self.pps.len().min(255) as u8);
        for pps in self.pps.iter().take(255) {
            b.u16(pps.len() as u16);
            b.bytes(pps);
        }
        b.buf
    }
}

/// AAC configuration recovered from `esds`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AacConfig {
    /// AudioSpecificConfig bytes (the DecoderSpecificInfo descriptor).
    pub asc: Vec<u8>,
    /// Audio object type signalled first in the ASC (2 is AAC-LC).
    pub object_type: u8,
    /// Core sampling rate from the ASC; 0 when unknown.
    pub sample_rate: u32,
    /// Channel configuration index from the ASC.
    pub channel_config: u8,
}

impl AacConfig {
    /// Build from an AudioSpecificConfig, decoding its leading fields.
    pub fn from_asc(asc: Vec<u8>) -> Self {
        let (object_type, sample_rate, channel_config) = parse_asc(&asc).unwrap_or((0, 0, 0));
        Self {
            asc,
            object_type,
            sample_rate,
            channel_config,
        }
    }
}

/// Sampling rates addressed by an AudioSpecificConfig sampling-frequency index.
const AAC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// Decode `(audio object type, sampling frequency, channel configuration)`.
pub(crate) fn parse_asc(asc: &[u8]) -> Option<(u8, u32, u8)> {
    let mut r = BitReader::new(asc);
    let mut object_type = u8::try_from(r.read_bits(5).ok()?).ok()?;
    if object_type == 31 {
        object_type = 32u8.checked_add(u8::try_from(r.read_bits(6).ok()?).ok()?)?;
    }
    let index = r.read_bits(4).ok()? as usize;
    let sample_rate = if index == 15 {
        r.read_bits(24).ok()?
    } else {
        *AAC_RATES.get(index)?
    };
    let channel_config = u8::try_from(r.read_bits(4).ok()?).ok()?;
    Some((object_type, sample_rate, channel_config))
}

/// One parsed sample description.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct SampleEntry {
    pub(crate) format: FourCc,
    pub(crate) video: Option<super::VideoInfo>,
    pub(crate) audio: Option<super::AudioInfo>,
    pub(crate) avc: Option<AvcConfig>,
    pub(crate) aac: Option<AacConfig>,
}

fn is_video_fourcc(f: &[u8; 4]) -> bool {
    matches!(
        f,
        b"avc1"
            | b"avc3"
            | b"hvc1"
            | b"hev1"
            | b"av01"
            | b"vp09"
            | b"vp08"
            | b"apch"
            | b"apcn"
            | b"apcs"
            | b"apco"
            | b"ap4h"
            | b"ap4x"
            | b"apv1"
            | b"jpeg"
            | b"mjpa"
            | b"mjpb"
            | b"AVdn"
            | b"AVdh"
            | b"mp4v"
            | b"2vuy"
            | b"v210"
    )
}

fn is_audio_fourcc(f: &[u8; 4]) -> bool {
    matches!(
        f,
        b"mp4a"
            | b"lpcm"
            | b"sowt"
            | b"twos"
            | b"in24"
            | b"in32"
            | b"fl32"
            | b"fl64"
            | b"alac"
            | b"Opus"
            | b"fLaC"
            | b"ac-3"
            | b"ec-3"
            | b".mp3"
            | b".mp2"
            | b"ipcm"
            | b"fpcm"
            | b"NONE"
    )
}

/// Parse one `stsd` entry. `handler` is the track handler type and `is_qt`
/// selects QuickTime interpretation of the sound description version.
pub(crate) fn parse_sample_entry(
    format: FourCc,
    payload: &[u8],
    handler: FourCc,
    is_qt: bool,
) -> Result<SampleEntry> {
    let mut c = Reader::labeled(payload, "sample entry");
    c.skip(6)?;
    let _data_reference_index = c.u16()?;
    let f = &format.0;
    if f == b"tmcd" {
        return Ok(SampleEntry {
            format,
            ..SampleEntry::default()
        });
    }
    // `raw ` names both uncompressed video and 8-bit PCM; the handler decides.
    let raw_audio = f == b"raw " && handler == b"soun";
    let video_like =
        (is_video_fourcc(f) && !raw_audio) || (handler == b"vide" && !is_audio_fourcc(f));
    let audio_like = is_audio_fourcc(f) || (handler == b"soun" && !video_like);
    if video_like {
        let _version = c.u16()?;
        let _revision = c.u16()?;
        let _vendor = c.u32()?;
        let _temporal_quality = c.u32()?;
        let _spatial_quality = c.u32()?;
        let width = c.u16()?;
        let height = c.u16()?;
        c.skip(4)?;
        c.skip(4)?;
        c.skip(4)?;
        let _frame_count = c.u16()?;
        c.skip(32)?;
        let _depth = c.u16()?;
        let _color_table = c.i16()?;
        let mut avc = None;
        for b in boxes(c.rest()).map_while(|b| b.ok()) {
            if b.kind == b"avcC" && matches!(f, b"avc1" | b"avc3") {
                avc = Some(AvcConfig::parse(b.payload)?);
            }
        }
        return Ok(SampleEntry {
            format,
            video: Some(super::VideoInfo { width, height }),
            audio: None,
            avc,
            aac: None,
        });
    }
    if audio_like {
        let version = c.u16()?;
        let _revision = c.u16()?;
        let _vendor = c.u32()?;
        let mut channels = c.u16()?;
        let _sample_size = c.u16()?;
        let _compression_id = c.i16()?;
        let _packet_size = c.u16()?;
        let fixed_rate = c.u32()?;
        let mut sample_rate = (f64::from(fixed_rate) / 65_536.0).round() as u32;
        if is_qt && version == 1 {
            c.skip(16)?;
        } else if is_qt && version == 2 {
            c.skip(4)?;
            let rate = c.f64()?;
            if !rate.is_finite() || rate < 0.0 {
                return Err(MediaError::Invalid(
                    "QuickTime sound entry has an invalid rate".into(),
                ));
            }
            sample_rate = rate as u32;
            let wide_channels = c.u32()?;
            channels = u16::try_from(wide_channels).map_err(|_| {
                MediaError::Invalid("QuickTime sound entry has too many channels".into())
            })?;
            c.skip(20)?;
        }
        let mut aac = None;
        for b in boxes(c.rest()).map_while(|b| b.ok()) {
            if b.kind == b"esds"
                && f == b"mp4a"
                && let Some(asc) = esds_asc(b.payload)
            {
                let config = AacConfig::from_asc(asc);
                sample_rate = if sample_rate == 0 {
                    config.sample_rate
                } else {
                    sample_rate
                };
                aac = Some(config);
            }
        }
        return Ok(SampleEntry {
            format,
            video: None,
            audio: Some(super::AudioInfo {
                sample_rate,
                channels,
            }),
            avc: None,
            aac,
        });
    }
    Ok(SampleEntry {
        format,
        ..SampleEntry::default()
    })
}

/// Read a descriptor tag and length from an MPEG-4 descriptor stream.
fn descriptor(c: &mut Reader<'_>) -> Result<(u8, usize)> {
    let tag = c.u8()?;
    let mut len = 0usize;
    for _ in 0..4 {
        let b = c.u8()?;
        len = (len << 7) | usize::from(b & 0x7F);
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok((tag, len))
}

/// Extract the DecoderSpecificInfo (AudioSpecificConfig) from an `esds` payload.
fn esds_asc(payload: &[u8]) -> Option<Vec<u8>> {
    let mut c = Reader::labeled(payload, "esds");
    c.full_header().ok()?;
    let (tag, len) = descriptor(&mut c).ok()?;
    if tag != 3 {
        return None;
    }
    let es = c.bytes(len.min(c.remaining())).ok()?;
    let mut c = Reader::labeled(es, "ES_Descriptor");
    c.u16().ok()?;
    let flags = c.u8().ok()?;
    if flags & 0x80 != 0 {
        c.u16().ok()?;
    }
    if flags & 0x40 != 0 {
        let n = c.u8().ok()? as usize;
        c.skip(n).ok()?;
    }
    if flags & 0x20 != 0 {
        c.u16().ok()?;
    }
    while c.remaining() > 0 {
        let (tag, len) = descriptor(&mut c).ok()?;
        let body = c.bytes(len.min(c.remaining())).ok()?;
        if tag != 4 {
            continue;
        }
        let mut d = Reader::labeled(body, "DecoderConfigDescriptor");
        d.u8().ok()?;
        d.u8().ok()?;
        d.u24().ok()?;
        d.u32().ok()?;
        d.u32().ok()?;
        while d.remaining() > 0 {
            let (tag, len) = descriptor(&mut d).ok()?;
            let value = d.bytes(len.min(d.remaining())).ok()?;
            if tag == 5 {
                return Some(value.to_vec());
            }
        }
        return None;
    }
    None
}

fn put_descriptor(b: &mut BoxBuf, tag: u8, body: &[u8]) {
    b.u8(tag);
    let n = body.len() as u32;
    b.u8(0x80 | ((n >> 21) & 0x7F) as u8);
    b.u8(0x80 | ((n >> 14) & 0x7F) as u8);
    b.u8(0x80 | ((n >> 7) & 0x7F) as u8);
    b.u8((n & 0x7F) as u8);
    b.bytes(body);
}

/// `esds` payload for an AAC AudioSpecificConfig.
fn esds_payload(asc: &[u8]) -> Vec<u8> {
    let mut decoder_config = BoxBuf::new();
    decoder_config.u8(0x40);
    decoder_config.u8(0x15);
    decoder_config.u8(0);
    decoder_config.u16(0);
    decoder_config.u32(0);
    decoder_config.u32(0);
    if !asc.is_empty() {
        put_descriptor(&mut decoder_config, 5, asc);
    }
    let mut es = BoxBuf::new();
    es.u16(1);
    es.u8(0);
    put_descriptor(&mut es, 4, &decoder_config.buf);
    put_descriptor(&mut es, 6, &[2]);
    let mut b = BoxBuf::new();
    b.u32(0);
    put_descriptor(&mut b, 3, &es.buf);
    b.buf
}

/// Write an `avc1` sample entry with its `avcC`.
pub(crate) fn write_video_entry(b: &mut BoxBuf, width: u16, height: u16, avc: &AvcConfig) {
    let entry = b.start(b"avc1");
    b.zeros(6);
    b.u16(1);
    b.u16(0);
    b.u16(0);
    b.u32(0);
    b.u32(0);
    b.u32(0);
    b.u16(width);
    b.u16(height);
    b.u32(0x0048_0000);
    b.u32(0x0048_0000);
    b.u32(0);
    b.u16(1);
    let name = b"VibeCut H.264";
    let mut compressor = [0u8; 32];
    if let Some(length) = compressor.first_mut() {
        *length = name.len() as u8;
    }
    if let Some(target) = compressor.get_mut(1..=name.len()) {
        target.copy_from_slice(name);
    }
    b.bytes(&compressor);
    b.u16(0x18);
    b.i16(-1);
    b.leaf(b"avcC", &avc.to_bytes());
    b.end(entry);
}

/// Write an `mp4a` sample entry with its `esds`.
pub(crate) fn write_audio_entry(b: &mut BoxBuf, sample_rate: u32, channels: u16, aac: &AacConfig) {
    let entry = b.start(b"mp4a");
    b.zeros(6);
    b.u16(1);
    b.u16(0);
    b.u16(0);
    b.u32(0);
    b.u16(channels.max(1));
    b.u16(16);
    b.i16(0);
    b.u16(0);
    // The 16.16 field saturates below 65 536; the ASC carries the true rate.
    let fixed = if sample_rate < 65_536 {
        sample_rate << 16
    } else {
        0
    };
    b.u32(fixed);
    b.leaf(b"esds", &esds_payload(&aac.asc));
    b.end(entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avcc_roundtrip_and_hostile_lengths() {
        let config = AvcConfig::new(
            vec![vec![0x67, 0x64, 0x00, 0x1F, 0xAC]],
            vec![vec![0x68, 0xEB]],
            4,
        );
        assert_eq!((config.profile, config.level), (0x64, 0x1F));
        assert_eq!(AvcConfig::parse(&config.to_bytes()).unwrap(), config);
        assert!(AvcConfig::parse(&[]).is_err());
        // SPS count says 31 but no length bytes follow.
        assert!(AvcConfig::parse(&[1, 0x64, 0, 0x1F, 0xFF, 0xE0 | 31]).is_err());
    }

    #[test]
    fn asc_roundtrip() {
        assert_eq!(parse_asc(&[0x11, 0x90]), Some((2, 48_000, 2)));
        assert_eq!(parse_asc(&[0x12, 0x10]), Some((2, 44_100, 2)));
        assert_eq!(parse_asc(&[]), None);
        let config = AacConfig::from_asc(vec![0x11, 0x90]);
        assert_eq!(
            (
                config.object_type,
                config.sample_rate,
                config.channel_config
            ),
            (2, 48_000, 2)
        );
    }

    #[test]
    fn written_entries_parse_back() {
        let avc = AvcConfig::new(vec![vec![0x67, 0x42, 0, 0x1E]], vec![vec![0x68, 0xCE]], 4);
        let mut b = BoxBuf::new();
        write_video_entry(&mut b, 1920, 1080, &avc);
        let entry = boxes(&b.buf).next().unwrap().unwrap();
        let parsed =
            parse_sample_entry(entry.kind, entry.payload, FourCc(*b"vide"), false).unwrap();
        assert_eq!(
            parsed.video,
            Some(super::super::VideoInfo {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(parsed.avc, Some(avc));

        let aac = AacConfig::from_asc(vec![0x11, 0x90]);
        let mut b = BoxBuf::new();
        write_audio_entry(&mut b, 48_000, 2, &aac);
        let entry = boxes(&b.buf).next().unwrap().unwrap();
        let parsed =
            parse_sample_entry(entry.kind, entry.payload, FourCc(*b"soun"), false).unwrap();
        assert_eq!(
            parsed.audio,
            Some(super::super::AudioInfo {
                sample_rate: 48_000,
                channels: 2
            })
        );
        assert_eq!(parsed.aac, Some(aac));
    }

    #[test]
    fn hostiles_do_not_panic() {
        for data in [
            &[][..],
            &[0xFF][..],
            &[0, 0, 0, 0, 0, 0, 1][..],
            &[0, 0, 0, 0, 0, 0, 1, 0, 0xFF, 0xFF][..],
        ] {
            let _ = parse_sample_entry(FourCc(*b"avc1"), data, FourCc(*b"vide"), false);
            let _ = parse_sample_entry(FourCc(*b"mp4a"), data, FourCc(*b"soun"), false);
            let _ = esds_asc(data);
        }
    }
}
