//! Streams planar PCM blocks into one AAC audio file.

use crate::{AudioEncoder, AudioEncoderSettings, AudioFrame, Container, MediaError, Muxer, Result};
use std::path::Path;
use time::{Rounding, Tick};

/// Largest block `write` takes per channel, the limit of one PCM frame.
pub const AUDIO_FILE_BLOCK: usize = 65_536;

/// Sequential writer: the caller owns the temporary path and promotes it only
/// after `finish` succeeds; dropping an unfinished writer leaves no trailer.
pub struct AudioFileWriter {
    encoder: AudioEncoder,
    muxer: Muxer,
    track: usize,
    sample_rate: u32,
    channels: usize,
    samples: u64,
}

impl AudioFileWriter {
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        let encoder = AudioEncoder::new(AudioEncoderSettings::aac(sample_rate, channels))?;
        let mut muxer = Muxer::create(path, Container::Mp4)?;
        let track = muxer.add_parameters(&encoder.parameters(), encoder.time_base())?;
        Ok(Self {
            encoder,
            muxer,
            track,
            sample_rate,
            channels: usize::from(channels),
            samples: 0,
        })
    }

    /// Samples per channel written so far.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Append one block: one plane per channel, equal lengths, 1..=`AUDIO_FILE_BLOCK`.
    pub fn write(&mut self, planes: &[Vec<f32>]) -> Result<()> {
        if planes.len() != self.channels {
            return Err(MediaError::Invalid("audio channel count"));
        }
        let count = planes.first().map_or(0, Vec::len);
        let origin =
            i64::try_from(self.samples).map_err(|_| MediaError::TooLarge("audio sample count"))?;
        let pts = Tick::from_units(origin, i64::from(self.sample_rate), Rounding::Nearest)?;
        let frame = AudioFrame::from_planar_f32(pts, self.sample_rate, planes)?;
        for packet in self.encoder.encode(&frame)? {
            self.muxer.write_packet(self.track, &packet)?;
        }
        self.samples += count as u64;
        Ok(())
    }

    /// Flush the encoder and write the trailer; returns samples per channel.
    pub fn finish(mut self) -> Result<u64> {
        for packet in self.encoder.flush()? {
            self.muxer.write_packet(self.track, &packet)?;
        }
        self.muxer.finish()?;
        Ok(self.samples)
    }
}
