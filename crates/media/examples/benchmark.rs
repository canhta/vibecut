//! Observed throughput through the same public decode/encode/mux interfaces.
use media::{
    Container, DecodePreference, Muxer, PacketReader, StreamKind, VideoDecoder, VideoEncoder,
    VideoEncoderSettings, VideoFrame,
};
use std::path::Path;
use std::time::{Duration, Instant};

fn run(input: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = PacketReader::open(input)?;
    let video = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .ok_or("no video")?
        .clone();
    let start = Instant::now();
    let mut decoder = VideoDecoder::new(&video, DecodePreference::Hardware)?;
    let mut decoded = 0usize;
    let mut hardware = 0usize;
    while let Some(packet) = reader.next_packet()? {
        if packet.stream() == video.index {
            for frame in decoder.decode(&packet)? {
                decoded += 1;
                hardware += usize::from(frame.is_hardware());
            }
        }
    }
    for frame in decoder.flush()? {
        decoded += 1;
        hardware += usize::from(frame.is_hardware());
    }
    let decode_elapsed = start.elapsed();
    println!(
        "decode_frames={decoded} hardware_frames={hardware} decode_seconds={:.6} decode_fps={:.3}",
        decode_elapsed.as_secs_f64(),
        decoded as f64 / decode_elapsed.as_secs_f64()
    );
    let start = Instant::now();
    let mut reader = PacketReader::open(input)?;
    if reader
        .probe()
        .streams
        .iter()
        .any(|s| s.kind == StreamKind::Audio)
    {
        return Err("benchmark requires a synthetic video-only fixture".into());
    }
    let mut decoder = VideoDecoder::new(&video, DecodePreference::Hardware)?;
    let mut encoder: Option<VideoEncoder> = None;
    let mut mux = Muxer::create(output, Container::Mp4)?;
    let mut count = 0usize;
    let mut encode_time = Duration::ZERO;
    let mut write = |frame: VideoFrame,
                     mux: &mut Muxer,
                     encoder: &mut Option<VideoEncoder>|
     -> Result<(), Box<dyn std::error::Error>> {
        if encoder.is_none() {
            let settings = VideoEncoderSettings::h264(
                frame.width(),
                frame.height(),
                video.frame_rate.ok_or("unknown rate")?,
                frame.color(),
            );
            let enc = VideoEncoder::new(settings, &frame)?;
            mux.add_parameters(&enc.parameters(), enc.time_base())?;
            *encoder = Some(enc);
        }
        let enc = encoder.as_mut().ok_or("encoder missing")?;
        let mark = Instant::now();
        let packets = enc.encode(&frame)?;
        encode_time += mark.elapsed();
        for packet in packets {
            mux.write_packet(0, &packet)?;
        }
        count += 1;
        Ok(())
    };
    while let Some(packet) = reader.next_packet()? {
        if packet.stream() == video.index {
            for frame in decoder.decode(&packet)? {
                write(frame, &mut mux, &mut encoder)?;
            }
        }
    }
    for frame in decoder.flush()? {
        write(frame, &mut mux, &mut encoder)?;
    }
    drop(write);
    let enc = encoder.as_mut().ok_or("no decoded video")?;
    let mark = Instant::now();
    let tail = enc.flush()?;
    encode_time += mark.elapsed();
    for packet in tail {
        mux.write_packet(0, &packet)?;
    }
    mux.finish()?;
    let elapsed = start.elapsed();
    println!(
        "encoder={} encoded_frames={count} encode_seconds={:.6} encode_fps={:.3} export_seconds={:.6}",
        enc.name(),
        encode_time.as_secs_f64(),
        count as f64 / encode_time.as_secs_f64(),
        elapsed.as_secs_f64()
    );
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [input, output] => run(Path::new(input), Path::new(output)),
        _ => Err("usage: benchmark INPUT OUTPUT".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
