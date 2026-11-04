//! Narrow gaps in the safe FFmpeg wrapper: codec metadata and hardware frames.
use crate::{AudioInfo, ColorInfo, MediaError, Result, VideoInfo, error};
use ffmpeg_next::{
    self as ff,
    ffi::{
        AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX, AV_PIX_FMT_FLAG_HWACCEL, AVCodecContext,
        AVHWDeviceContext, AVHWDeviceType, AVMediaType, AVPixelFormat, FF_THREAD_FRAME,
        FF_THREAD_SLICE, av_buffer_ref, av_frame_copy_props, av_frame_get_buffer, av_frame_ref,
        av_hwdevice_ctx_create, av_hwframe_transfer_data, av_opt_set_int, av_packet_ref,
        av_pix_fmt_desc_get, avcodec_find_decoder, avcodec_get_hw_config,
    },
};

pub(super) fn parameter_info(
    p: &ff::codec::Parameters,
) -> Result<(Option<VideoInfo>, Option<AudioInfo>)> {
    // SAFETY: Parameters owns a live AVCodecParameters; this read does not retain
    // pointers or mutate the object. Null pixel descriptors mean an unknown format.
    unsafe {
        let p = &*p.as_ptr();
        if p.extradata_size < 0 || p.extradata_size > 1 << 20 {
            return Err(MediaError::TooLarge("codec extradata"));
        }
        if p.codec_type == AVMediaType::AVMEDIA_TYPE_VIDEO {
            if p.width <= 0 || p.height <= 0 || i64::from(p.width) * i64::from(p.height) > 1 << 26 {
                return Err(MediaError::TooLarge("video dimensions"));
            }
            let depth = u8::try_from(p.bits_per_raw_sample).ok().filter(|n| *n > 0);
            return Ok((
                Some(VideoInfo {
                    width: p.width as u32,
                    height: p.height as u32,
                    color: ColorInfo {
                        primaries: p.color_primaries as u32,
                        transfer: p.color_trc as u32,
                        matrix: p.color_space as u32,
                        range: p.color_range as u32,
                        chroma_location: p.chroma_location as u32,
                    },
                    bit_depth: depth,
                }),
                None,
            ));
        }
        if p.codec_type == AVMediaType::AVMEDIA_TYPE_AUDIO {
            if p.sample_rate == 0 || p.ch_layout.nb_channels == 0 {
                return Ok((None, None));
            }
            if p.sample_rate < 0
                || p.sample_rate > 384_000
                || p.ch_layout.nb_channels <= 0
                || p.ch_layout.nb_channels > 32
            {
                return Err(MediaError::TooLarge("audio format"));
            }
            return Ok((
                None,
                Some(AudioInfo {
                    sample_rate: p.sample_rate as u32,
                    channels: p.ch_layout.nb_channels as u16,
                    initial_padding: p.initial_padding.max(0) as u32,
                    trailing_padding: p.trailing_padding.max(0) as u32,
                }),
            ));
        }
        Ok((None, None))
    }
}

pub(super) fn thread_and_limits(ctx: &mut ff::codec::context::Context) {
    // SAFETY: exclusive access to the not-yet-open codec context; FFmpeg accepts
    // both threading flags and validates the requested maximum pixel count.
    unsafe {
        let ptr = ctx.as_mut_ptr();
        (*ptr).thread_type = FF_THREAD_FRAME | FF_THREAD_SLICE;
        (*ptr).thread_count = 0;
        (*ptr).max_pixels = 1 << 26;
    }
}

pub(super) fn decoder_bounds(
    context: &mut ff::codec::context::Context,
    pixels: Option<u64>,
    samples: Option<u64>,
) {
    // SAFETY: exclusively borrowed unopened codec context. Callers validate these
    // small positive bounds; FFmpeg checks them before allocating decoded storage.
    unsafe {
        if let Some(pixels) = pixels {
            (*context.as_mut_ptr()).max_pixels = pixels as i64;
        }
        if let Some(samples) = samples {
            (*context.as_mut_ptr()).max_samples = samples as i64;
        }
    }
}

pub(super) fn frame_color(f: &ff::frame::Video) -> ColorInfo {
    // SAFETY: the owned frame is live; only initialized scalar metadata is read.
    unsafe {
        let f = &*f.as_ptr();
        ColorInfo {
            primaries: f.color_primaries as u32,
            transfer: f.color_trc as u32,
            matrix: f.colorspace as u32,
            range: f.color_range as u32,
            chroma_location: f.chroma_location as u32,
        }
    }
}

pub(super) fn premultiplied_alpha(frame: &ff::frame::Video) -> bool {
    // SAFETY: owned live frame; read initialized scalar alpha metadata only.
    unsafe { (*frame.as_ptr()).alpha_mode == ff::ffi::AVAlphaMode::AVALPHA_MODE_PREMULTIPLIED }
}
pub(super) fn is_hardware(f: &ff::frame::Video) -> bool {
    // SAFETY: the frame owns its buffer references. No GPU plane is dereferenced.
    unsafe { !(*f.as_ptr()).hw_frames_ctx.is_null() }
}

pub(super) fn download(f: &ff::frame::Video) -> Result<ff::frame::Video> {
    let mut output = ff::frame::Video::empty();
    // SAFETY: FFmpeg transfers from a valid reference-counted hardware frame into
    // an empty owned frame, allocating CPU buffers. Properties are copied once.
    unsafe {
        let status = av_hwframe_transfer_data(output.as_mut_ptr(), f.as_ptr(), 0);
        if status < 0 {
            return Err(error("download GPU frame")(ff::Error::from(status)));
        }
        let status = av_frame_copy_props(output.as_mut_ptr(), f.as_ptr());
        if status < 0 {
            return Err(error("copy frame metadata")(ff::Error::from(status)));
        }
    }
    Ok(output)
}

unsafe extern "C" fn choose_format(
    ctx: *mut AVCodecContext,
    formats: *const AVPixelFormat,
) -> AVPixelFormat {
    // SAFETY: libavcodec supplies a live codec context and a sentinel-terminated
    // candidate array for the duration of this callback. Only scalar fields and
    // immutable static codec configuration records are read.
    unsafe {
        if ctx.is_null() || formats.is_null() {
            return AVPixelFormat::AV_PIX_FMT_NONE;
        }
        let device = (*ctx).hw_device_ctx;
        if !device.is_null() {
            let kind = (*((*device).data.cast::<AVHWDeviceContext>())).type_;
            for i in 0..64 {
                let config = avcodec_get_hw_config((*ctx).codec, i);
                if config.is_null() {
                    break;
                }
                if (*config).device_type == kind {
                    for j in 0..128 {
                        let f = *formats.add(j);
                        if f == AVPixelFormat::AV_PIX_FMT_NONE {
                            break;
                        }
                        if f == (*config).pix_fmt {
                            return f;
                        }
                    }
                }
            }
        }
        for i in 0..128 {
            let f = *formats.add(i);
            if f == AVPixelFormat::AV_PIX_FMT_NONE {
                break;
            }
            let desc = av_pix_fmt_desc_get(f);
            if !desc.is_null() && (*desc).flags & AV_PIX_FMT_FLAG_HWACCEL as u64 == 0 {
                return f;
            }
        }
        AVPixelFormat::AV_PIX_FMT_NONE
    }
}

pub(super) fn hardware_decoder(ctx: &mut ff::codec::context::Context) {
    // SAFETY: exclusive access before avcodec_open2. Device references are owned
    // by AVCodecContext and released by its existing ffmpeg-next RAII owner.
    unsafe {
        let ptr = ctx.as_mut_ptr();
        let codec = avcodec_find_decoder((*ptr).codec_id);
        if codec.is_null() {
            return;
        }
        let kinds: &[AVHWDeviceType] = if cfg!(target_os = "macos") {
            &[AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX]
        } else if cfg!(windows) {
            &[
                AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
                AVHWDeviceType::AV_HWDEVICE_TYPE_QSV,
            ]
        } else {
            &[]
        };
        for &kind in kinds {
            let mut supported = false;
            for i in 0..64 {
                let config = avcodec_get_hw_config(codec, i);
                if config.is_null() {
                    break;
                }
                if (*config).device_type == kind
                    && (*config).methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
                {
                    supported = true;
                    break;
                }
            }
            if !supported {
                continue;
            }
            let mut device = std::ptr::null_mut();
            if av_hwdevice_ctx_create(&mut device, kind, std::ptr::null(), std::ptr::null_mut(), 0)
                >= 0
            {
                (*ptr).hw_device_ctx = device;
                (*ptr).get_format = Some(choose_format);
                return;
            }
        }
    }
}

pub(super) fn copy_props(
    source: &ff::frame::Video,
    destination: &mut ff::frame::Video,
) -> Result<()> {
    // SAFETY: distinct owned live frames; the function retains any property
    // references it copies and does not alias their pixel buffers.
    unsafe {
        let status = av_frame_copy_props(destination.as_mut_ptr(), source.as_ptr());
        if status < 0 {
            return Err(error("copy video properties")(ff::Error::from(status)));
        }
    }
    Ok(())
}

pub(super) fn share_hardware_frames(
    ctx: &mut ff::codec::context::Context,
    frame: &ff::frame::Video,
) -> Result<()> {
    // SAFETY: both wrappers own live FFmpeg objects. av_buffer_ref retains the
    // pool independently; the codec context releases it after the encoder closes.
    unsafe {
        let source = (*frame.as_ptr()).hw_frames_ctx;
        if !source.is_null() {
            let retained = av_buffer_ref(source);
            if retained.is_null() {
                return Err(MediaError::TooLarge("hardware frame reference"));
            }
            (*ctx.as_mut_ptr()).hw_frames_ctx = retained;
        }
    }
    Ok(())
}

pub(super) fn reference_video(source: &ff::frame::Video) -> Result<ff::frame::Video> {
    let mut destination = ff::frame::Video::empty();
    // SAFETY: owned empty destination and live reference-counted source. Unlike
    // ffmpeg-next Video::clone, av_frame_ref also works for opaque GPU planes.
    unsafe {
        if destination.as_ptr().is_null() {
            return Err(MediaError::TooLarge("video frame reference"));
        }
        let status = av_frame_ref(destination.as_mut_ptr(), source.as_ptr());
        if status < 0 {
            return Err(error("retain video frame")(ff::Error::from(status)));
        }
    }
    Ok(destination)
}
pub(super) fn reference_audio(source: &ff::frame::Audio) -> Result<ff::frame::Audio> {
    let mut destination = ff::frame::Audio::empty();
    // SAFETY: live owned wrappers; the destination obtains independent buffer
    // references, and both RAII owners release only their own references.
    unsafe {
        if destination.as_ptr().is_null() {
            return Err(MediaError::TooLarge("audio frame reference"));
        }
        let status = av_frame_ref(destination.as_mut_ptr(), source.as_ptr());
        if status < 0 {
            return Err(error("retain audio frame")(ff::Error::from(status)));
        }
    }
    Ok(destination)
}
pub(super) fn reference_packet(source: &ff::Packet) -> Result<ff::Packet> {
    use ff::codec::packet::{Mut, Ref};
    let mut destination = ff::Packet::empty();
    // SAFETY: distinct live packet structs. av_packet_ref retains payload and
    // side-data ownership without making the buffer writable (and copying it).
    unsafe {
        let status = av_packet_ref(destination.as_mut_ptr(), source.as_ptr());
        if status < 0 {
            return Err(error("retain packet")(ff::Error::from(status)));
        }
    }
    Ok(destination)
}

pub(super) fn encoder_color(ctx: &mut ff::codec::context::Context, color: ColorInfo) -> Result<()> {
    // SAFETY: exclusive unopened codec context. AVOptions validates integer enum
    // values instead of constructing a Rust C enum from untrusted numeric tags.
    unsafe {
        for (name, value) in [
            (c"color_primaries", color.primaries),
            (c"color_trc", color.transfer),
            (c"colorspace", color.matrix),
            (c"color_range", color.range),
            (c"chroma_sample_location", color.chroma_location),
        ] {
            let status =
                av_opt_set_int(ctx.as_mut_ptr().cast(), name.as_ptr(), i64::from(value), 0);
            if status < 0 {
                return Err(error("set encoder color")(ff::Error::from(status)));
            }
        }
    }
    Ok(())
}

pub(super) fn encoder_frame_metadata(
    ctx: &ff::codec::context::Context,
    frame: &mut ff::frame::Video,
) {
    // SAFETY: live owned context and exclusive frame; metadata scalar values were
    // validated by AVOptions. CFR encoder input duration is one encoder time unit.
    unsafe {
        let ctx = &*ctx.as_ptr();
        let frame = &mut *frame.as_mut_ptr();
        frame.color_primaries = ctx.color_primaries;
        frame.color_trc = ctx.color_trc;
        frame.colorspace = ctx.colorspace;
        frame.color_range = ctx.color_range;
        frame.chroma_location = ctx.chroma_sample_location;
        frame.time_base = ctx.time_base;
        frame.duration = 1;
    }
}

pub(super) fn audio_buffer(
    format: ff::format::Sample,
    samples: usize,
    layout: ff::ChannelLayout,
    rate: u32,
) -> Result<ff::frame::Audio> {
    if samples == 0 || samples > 65_536 || layout.channels() == 0 || layout.channels() > 32 {
        return Err(MediaError::TooLarge("audio allocation"));
    }
    let mut frame = ff::frame::Audio::empty();
    frame.set_format(format);
    frame.set_samples(samples);
    frame.set_channel_layout(layout);
    frame.set_rate(rate);
    // SAFETY: bounded validated dimensions/layout on a new owned frame. Allocate
    // checked buffers before safe resampling APIs, whose alloc helper ignores OOM.
    unsafe {
        if frame.as_ptr().is_null() {
            return Err(MediaError::TooLarge("audio frame"));
        }
        let status = av_frame_get_buffer(frame.as_mut_ptr(), 0);
        if status < 0 {
            return Err(error("allocate audio frame")(ff::Error::from(status)));
        }
    }
    Ok(frame)
}

/// Aligned integer window on supported 8-bit 4:2:0 CPU layouts. The caller
/// retains a wide halo for Lanczos chroma reconstruction, then samples only its
/// interior. Other layouts and existing crop metadata keep the full-frame path.
pub(super) fn horizontal_window(
    source: &ff::frame::Video,
    left: u32,
    right: u32,
) -> Result<Option<ff::frame::Video>> {
    if !matches!(
        source.format(),
        ff::format::Pixel::NV12 | ff::format::Pixel::YUV420P
    ) || !source.width().is_multiple_of(2)
    {
        return Ok(None);
    }
    if left >= right || right > source.width() || !left.is_multiple_of(128) {
        return Err(MediaError::Invalid("conversion window"));
    }
    let mut cropped = reference_video(source)?;
    // SAFETY: reference_video retains the admitted source allocation. Bounds
    // above keep a nonempty window inside it; flags=0 preserves FFmpeg's plane
    // alignment (never AV_FRAME_CROP_UNALIGNED). Only CPU layouts are admitted.
    // The original frame and its data pointers remain untouched.
    unsafe {
        let frame = &mut *cropped.as_mut_ptr();
        if frame.crop_left != 0
            || frame.crop_right != 0
            || frame.crop_top != 0
            || frame.crop_bottom != 0
        {
            return Ok(None);
        }
        frame.crop_left = left as usize;
        frame.crop_right = (source.width() - right) as usize;
        let status = ff::ffi::av_frame_apply_cropping(cropped.as_mut_ptr(), 0);
        if status < 0 {
            return Err(error("apply aligned conversion window")(ff::Error::from(
                status,
            )));
        }
    }
    if cropped.width() != right - left || cropped.height() != source.height() {
        return Ok(None);
    }
    Ok(Some(cropped))
}

/// Frame-aware swscale path: the safe wrapper's legacy sws_scale call ignores
/// frame color/chroma metadata and its conversion return code.
pub(super) struct Scaler(std::ptr::NonNull<ff::ffi::SwsContext>);
impl Scaler {
    pub fn with_threads(threads: u8) -> Result<Self> {
        if !(1..=8).contains(&threads) {
            return Err(MediaError::Invalid("conversion thread count"));
        }
        // SAFETY: new context has no initialized legacy source/destination state;
        // sws_scale_frame dynamically configures it from each pair of frames.
        unsafe {
            let context = std::ptr::NonNull::new(ff::ffi::sws_alloc_context())
                .ok_or(MediaError::TooLarge("scaler context"))?;
            let result = Self(context);
            let status = av_opt_set_int(
                context.as_ptr().cast(),
                c"sws_flags".as_ptr(),
                i64::from(
                    (ff::software::scaling::Flags::LANCZOS
                        | ff::software::scaling::Flags::ACCURATE_RND)
                        .bits(),
                ),
                0,
            );
            if status < 0 {
                return Err(error("configure quality scaler")(ff::Error::from(status)));
            }
            let status = av_opt_set_int(
                context.as_ptr().cast(),
                c"threads".as_ptr(),
                i64::from(threads),
                0,
            );
            if status < 0 {
                return Err(error("configure conversion threads")(ff::Error::from(
                    status,
                )));
            }
            Ok(result)
        }
    }
    pub fn rgba(&mut self, source: &ff::frame::Video) -> Result<ff::frame::Video> {
        let mut output = ff::frame::Video::empty();
        output.set_width(source.width());
        output.set_height(source.height());
        output.set_format(ff::format::Pixel::RGBA);
        copy_props(source, &mut output)?;
        // SAFETY: distinct live frames, source dimensions were admitted by decode;
        // swscale allocates checked output and reads source color/range/chroma tags.
        unsafe {
            (*output.as_mut_ptr()).colorspace = ff::ffi::AVColorSpace::AVCOL_SPC_RGB;
            (*output.as_mut_ptr()).color_range = ff::ffi::AVColorRange::AVCOL_RANGE_JPEG;
            let result =
                ff::ffi::sws_scale_frame(self.0.as_ptr(), output.as_mut_ptr(), source.as_ptr());
            if result < 0 {
                return Err(error("convert straight RGBA")(ff::Error::from(result)));
            }
        }
        Ok(output)
    }
    pub fn scale(
        &mut self,
        source: &ff::frame::Video,
        width: u32,
        height: u32,
        format: ff::format::Pixel,
        encoder: &ff::codec::context::Context,
    ) -> Result<ff::frame::Video> {
        let mut source = reference_video(source)?;
        encoder_frame_metadata(encoder, &mut source);
        let mut output = ff::frame::Video::empty();
        output.set_width(width);
        output.set_height(height);
        output.set_format(format);
        copy_props(&source, &mut output)?;
        // SAFETY: live distinct owned frames, bounded dimensions validated by the
        // encoder, and CPU source memory. FFmpeg allocates the output planes and
        // reads chroma siting/range/matrix from AVFrame metadata.
        unsafe {
            let status =
                ff::ffi::sws_scale_frame(self.0.as_ptr(), output.as_mut_ptr(), source.as_ptr());
            if status < 0 {
                return Err(error("scale frame with color metadata")(ff::Error::from(
                    status,
                )));
            }
        }
        Ok(output)
    }
}
impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: this unique RAII owner frees its context exactly once after calls finish.
        unsafe {
            ff::ffi::sws_freeContext(self.0.as_ptr());
        }
    }
}

pub(super) fn frame_bit_depth(frame: &ff::frame::Video) -> Option<u8> {
    // SAFETY: owned FFmpeg frames and retained hardware-pool references are live;
    // pixel format values and descriptor records originate in FFmpeg itself.
    unsafe {
        let pixel: ff::ffi::AVPixelFormat = frame.format().into();
        let frame = &*frame.as_ptr();
        let format = if frame.hw_frames_ctx.is_null() {
            pixel
        } else {
            (*((*frame.hw_frames_ctx)
                .data
                .cast::<ff::ffi::AVHWFramesContext>()))
            .sw_format
        };
        let descriptor = av_pix_fmt_desc_get(format);
        if descriptor.is_null() {
            None
        } else {
            (*descriptor)
                .comp
                .first()
                .and_then(|component| u8::try_from(component.depth).ok())
        }
    }
}

/// FFmpeg's bounded PCM FIFO, used once to bridge decoder/resampler and encoder
/// frame sizes. It stores samples, never a second timing or priming model.
pub(super) struct AudioFifo {
    pointer: std::ptr::NonNull<ff::ffi::AVAudioFifo>,
    format: ff::format::Sample,
    layout: ff::ChannelLayout,
    rate: u32,
}
impl AudioFifo {
    pub fn new(format: ff::format::Sample, layout: ff::ChannelLayout, rate: u32) -> Result<Self> {
        // SAFETY: validated encoder format/layout/rate, and a small initial
        // allocation. Every later growth is bounded before calling FFmpeg.
        let pointer =
            unsafe { ff::ffi::av_audio_fifo_alloc(format.into(), layout.channels(), 2048) };
        Ok(Self {
            pointer: std::ptr::NonNull::new(pointer).ok_or(MediaError::TooLarge("audio FIFO"))?,
            format,
            layout,
            rate,
        })
    }
    pub fn samples(&self) -> usize {
        // SAFETY: this live FIFO has a unique owner and nonnegative sample count.
        unsafe { ff::ffi::av_audio_fifo_size(self.pointer.as_ptr()).max(0) as usize }
    }
    pub fn push(&mut self, frame: &ff::frame::Audio) -> Result<()> {
        if frame.format() != self.format
            || frame.channel_layout() != self.layout
            || frame.rate() != self.rate
        {
            return Err(MediaError::Invalid("audio FIFO format"));
        }
        if frame.samples() > 65_536
            || self
                .samples()
                .checked_add(frame.samples())
                .is_none_or(|n| n > 131_072)
        {
            return Err(MediaError::TooLarge("audio FIFO samples"));
        }
        if frame.samples() == 0 {
            return Ok(());
        }
        // SAFETY: frame owns initialized planes in the FIFO's sample format;
        // FFmpeg copies samples without retaining or modifying input planes.
        unsafe {
            let status = ff::ffi::av_audio_fifo_write(
                self.pointer.as_ptr(),
                (*frame.as_ptr()).extended_data.cast(),
                frame.samples() as i32,
            );
            if status < 0 {
                return Err(error("buffer audio samples")(ff::Error::from(status)));
            }
            if status as usize != frame.samples() {
                return Err(MediaError::Invalid("short audio FIFO write"));
            }
        }
        Ok(())
    }
    pub fn pop(&mut self, samples: usize) -> Result<ff::frame::Audio> {
        if samples == 0 || samples > self.samples() || samples > 65_536 {
            return Err(MediaError::Invalid("audio FIFO read size"));
        }
        let mut frame = audio_buffer(self.format, samples, self.layout, self.rate)?;
        // SAFETY: allocated output planes match the FIFO format and count, and
        // samples does not exceed either the output capacity or queued samples.
        unsafe {
            let status = ff::ffi::av_audio_fifo_read(
                self.pointer.as_ptr(),
                (*frame.as_mut_ptr()).extended_data.cast(),
                samples as i32,
            );
            if status < 0 {
                return Err(error("read audio FIFO")(ff::Error::from(status)));
            }
            if status as usize != samples {
                return Err(MediaError::Invalid("short audio FIFO read"));
            }
        }
        Ok(frame)
    }
}
impl Drop for AudioFifo {
    fn drop(&mut self) {
        // SAFETY: unique owner, no borrowed planes or callbacks survive a FIFO call.
        unsafe {
            ff::ffi::av_audio_fifo_free(self.pointer.as_ptr());
        }
    }
}

pub(super) fn cpu_stride(frame: &ff::frame::Video, index: usize) -> Result<usize> {
    // SAFETY: live decoded frame; checked indexing reads scalar plane metadata.
    // ffmpeg-next's slice helpers cast negative strides to usize, so reject those
    // layouts before creating a Rust slice. Dynamic swscale can normalize them.
    unsafe {
        let frame = &*frame.as_ptr();
        let stride = *frame
            .linesize
            .get(index)
            .ok_or(MediaError::Invalid("plane index"))?;
        let data = *frame
            .data
            .get(index)
            .ok_or(MediaError::Invalid("plane index"))?;
        if stride < 0 {
            return Err(MediaError::Unsupported(
                "negative stride requires explicit normalization",
            ));
        }
        if data.is_null() {
            return Err(MediaError::Invalid("missing CPU plane"));
        }
        Ok(stride as usize)
    }
}

pub(super) fn allocation_limit() {
    // SAFETY: called once before any media operation, while initialization is
    // serialized. media is the sole FFmpeg owner; the global allocator limit
    // rejects hostile single allocations before a packet/frame reaches Rust.
    unsafe {
        ff::ffi::av_max_alloc(256 << 20);
    }
}

pub(super) fn clear_codec_tag(stream: &mut ff::StreamMut<'_>) {
    // SAFETY: exclusive output stream with owned codec parameters. A source
    // container's fourcc is not portable; let the destination muxer choose it.
    unsafe {
        (*(*stream.as_mut_ptr()).codecpar).codec_tag = 0;
    }
}

pub(super) fn resample_chunks(
    context: &mut ff::software::resampling::Context,
    input: &ff::frame::Audio,
) -> Result<Vec<ff::frame::Audio>> {
    let output = *context.output();
    let mut frames = Vec::new();
    let mut count = input.samples();
    loop {
        // SAFETY: live initialized context, bounded input sample count. This is
        // an upper bound, not a request to grow FFmpeg's internal input buffer.
        let bound = unsafe { ff::ffi::swr_get_out_samples(context.as_mut_ptr(), count as i32) };
        if bound < 0 {
            return Err(error("resampling capacity")(ff::Error::from(bound)));
        }
        let capacity = (bound as usize).clamp(1, 65_536);
        let mut frame = audio_buffer(output.format, capacity, output.channel_layout, output.rate)?;
        // SAFETY: live uniquely owned resampler and bounded initialized frames.
        // A non-null input array with zero samples drains buffered output without
        // signaling end-of-stream; only flush passes a null input array.
        let produced = unsafe {
            ff::ffi::swr_convert(
                context.as_mut_ptr(),
                (*frame.as_mut_ptr()).extended_data,
                capacity as i32,
                (*input.as_ptr()).extended_data.cast(),
                count as i32,
            )
        };
        if produced < 0 {
            return Err(error("resample audio")(ff::Error::from(produced)));
        }
        frame.set_samples(produced as usize);
        if produced > 0 {
            frames.push(frame);
        }
        if (produced as usize) < capacity {
            break;
        }
        count = 0;
    }
    Ok(frames)
}

pub(super) fn rgba_buffer(width: u32, height: u32, color: ColorInfo) -> Result<ff::frame::Video> {
    let mut frame = ff::frame::Video::empty();
    frame.set_width(width);
    frame.set_height(height);
    frame.set_format(ff::format::Pixel::RGBA);
    let mut context = ff::codec::context::Context::new();
    encoder_color(&mut context, color)?;
    encoder_frame_metadata(&context, &mut frame);
    // SAFETY: positive bounded dimensions checked by caller, new uniquely owned
    // frame. Check allocation before using safe slice helpers that assume success.
    unsafe {
        if frame.as_ptr().is_null() {
            return Err(MediaError::TooLarge("RGBA frame"));
        }
        (*frame.as_mut_ptr()).alpha_mode = ff::ffi::AVAlphaMode::AVALPHA_MODE_STRAIGHT;
        let status = av_frame_get_buffer(frame.as_mut_ptr(), 32);
        if status < 0 {
            return Err(error("allocate RGBA frame")(ff::Error::from(status)));
        }
    }
    Ok(frame)
}

pub(super) fn display_transform_required(parameters: &ff::codec::Parameters) -> Result<bool> {
    // SAFETY: Parameters retains its codec metadata and side-data buffers for this
    // borrow. Validate counts, pointers and the fixed 9*i32 matrix size before
    // reading, and never expose an FFI pointer to the compositor.
    unsafe {
        let pointer = parameters.as_ptr();
        if pointer.is_null() {
            return Err(MediaError::Invalid("codec parameters"));
        }
        let p = &*pointer;
        let ratio = p.sample_aspect_ratio;
        if ratio.num < 0
            || ratio.den < 0
            || (ratio.num > 0 && (ratio.den == 0 || ratio.num != ratio.den))
        {
            return Ok(true);
        }
        if p.nb_coded_side_data < 0
            || p.nb_coded_side_data > 256
            || (p.nb_coded_side_data > 0 && p.coded_side_data.is_null())
        {
            return Err(MediaError::TooLarge("codec side data"));
        }
        let data = ff::ffi::av_packet_side_data_get(
            p.coded_side_data,
            p.nb_coded_side_data,
            ff::ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
        );
        if data.is_null() {
            return Ok(false);
        }
        if (*data).data.is_null() || (*data).size < 36 {
            return Err(MediaError::Invalid("display matrix"));
        }
        let identity = [65536i32, 0, 0, 0, 65536, 0, 0, 0, 1073741824];
        for (index, expected) in identity.into_iter().enumerate() {
            if (*data).data.add(index * 4).cast::<i32>().read_unaligned() != expected {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
