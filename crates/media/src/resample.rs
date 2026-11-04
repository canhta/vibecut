//! Explicit streaming sample-rate/layout conversion, with one source clock.
use crate::{AudioFrame, MediaError, Result, TimeBase, error, ffi};
use ffmpeg_next as ff;
use time::{Rounding, Tick};
#[derive(Clone, Copy, Debug)]
pub enum AudioSampleFormat {
    PlanarF32,
    PackedF32,
}
impl AudioSampleFormat {
    fn ff(self) -> ff::format::Sample {
        ff::format::Sample::F32(match self {
            Self::PlanarF32 => ff::format::sample::Type::Planar,
            Self::PackedF32 => ff::format::sample::Type::Packed,
        })
    }
}
/// One continuous source span. Recreate at a cut/discontinuity; drain flush at
/// its end. Rate/layout changes are explicit and samples use the original clock.
pub struct AudioResampler {
    context: ff::software::resampling::Context,
    origin: Option<Tick>,
    output_samples: i64,
    expected_input: Option<Tick>,
    finished: bool,
}
impl AudioResampler {
    pub fn new(
        first: &AudioFrame,
        output_rate: u32,
        channels: u16,
        format: AudioSampleFormat,
    ) -> Result<Self> {
        if !(8_000..=192_000).contains(&output_rate) || channels == 0 || channels > 8 {
            return Err(MediaError::Invalid("resampler output format"));
        }
        // Bound the total returned PCM as well as each individual frame.
        if u64::from(output_rate) > u64::from(first.sample_rate()) * 24 {
            return Err(MediaError::Unsupported("resampling expansion exceeds 24x"));
        }
        let context = ff::software::resampling::Context::get(
            first.inner.format(),
            first.inner.channel_layout(),
            first.sample_rate(),
            format.ff(),
            ff::ChannelLayout::default(i32::from(channels)),
            output_rate,
        )
        .map_err(error("create resampler"))?;
        Ok(Self {
            context,
            origin: None,
            output_samples: 0,
            expected_input: None,
            finished: false,
        })
    }
    /// Returns bounded output frames; retains only the filter delay between calls.
    pub fn convert(&mut self, input: &AudioFrame) -> Result<Vec<AudioFrame>> {
        if self.finished {
            return Err(MediaError::Invalid("resampler finished"));
        }
        let definition = self.context.input();
        if definition.rate != input.sample_rate()
            || definition.format != input.inner.format()
            || definition.channel_layout != input.inner.channel_layout()
        {
            return Err(MediaError::Unsupported("resampler input changed"));
        }
        let pts = input
            .pts()?
            .ok_or(MediaError::Unsupported("resampling requires source PTS"))?;
        let tolerance = i128::from(input.time_base.ticks(1)?.0).abs() + 1;
        if self
            .expected_input
            .is_some_and(|expected| (i128::from(pts.0) - i128::from(expected.0)).abs() > tolerance)
        {
            return Err(MediaError::Invalid("resampler source discontinuity"));
        }
        self.origin.get_or_insert(pts);
        self.expected_input = Some(
            self.expected_input
                .unwrap_or(pts)
                .checked_add(Tick::from_units(
                    input.samples() as i64,
                    i64::from(input.sample_rate()),
                    Rounding::Nearest,
                )?)
                .ok_or(MediaError::TooLarge("audio timestamp"))?,
        );
        ffi::resample_chunks(&mut self.context, &input.inner)?
            .into_iter()
            .map(|frame| self.output(frame))
            .collect()
    }

    pub fn flush(&mut self) -> Result<Option<AudioFrame>> {
        if self.finished {
            return Ok(None);
        }
        let output = self.context.output();
        let mut frame =
            ffi::audio_buffer(output.format, 65_536, output.channel_layout, output.rate)?;
        self.context
            .flush(&mut frame)
            .map_err(error("flush resampler"))?;
        if frame.samples() == 0 {
            self.finished = true;
            return Ok(None);
        }
        Ok(Some(self.output(frame)?))
    }
    fn output(&mut self, mut inner: ff::frame::Audio) -> Result<AudioFrame> {
        let time_base = TimeBase::new(1, inner.rate() as i32)?;
        let offset = time_base.ticks(self.output_samples)?;
        let presentation = self
            .origin
            .map(|origin| {
                origin
                    .checked_add(offset)
                    .ok_or(MediaError::TooLarge("resampled timestamp"))
            })
            .transpose()?;
        self.output_samples = self
            .output_samples
            .checked_add(inner.samples() as i64)
            .ok_or(MediaError::TooLarge("resampled sample count"))?;
        inner.set_pts(None);
        Ok(AudioFrame {
            inner,
            time_base,
            presentation,
        })
    }
}
