use crate::arguments::one_path;
use crate::exit_codes::CliError;
use crate::output::{Output, tick_json};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use time::Tick;

pub(crate) fn probe(args: Vec<OsString>) -> Result<Output, CliError> {
    let path = one_path(args, "usage: vibecut-cli probe <file>")?;
    let reader = media::PacketReader::open(&path)?;
    let probe = reader.probe().clone();
    let capabilities = media::ProbeCapabilities::from_probe(&probe);
    drop(reader);
    let metadata = media::MediaMetadata::from_probe(&media::SourceProbe::from(&probe))?;
    let streams: Vec<Value> = probe.streams.iter().map(stream_json).collect();
    let capabilities: Vec<Value> = capabilities
        .streams
        .iter()
        .map(|stream| {
            json!({
                "index": stream.index,
                "kind": stream_kind(stream.kind),
                "codec": stream.codec,
                "decodable": stream.decodability.is_decodable(),
                "reason": match stream.decodability {
                    media::Decodability::Decodable => Value::Null,
                    media::Decodability::Unsupported(reason) => json!(reason.label()),
                },
            })
        })
        .collect();
    let value = json!({
        "format": probe.format,
        "duration": optional_tick_json(probe.duration),
        "bytes_read": probe.bytes_read,
        "metadata": {
            "kind": media_kind(metadata.kind),
            "duration": optional_tick_json(metadata.duration),
            "width": metadata.width,
            "height": metadata.height,
            "frame_rate": metadata.display_frame_rate.map(|rate| json!({
                "num": rate.exact.num(),
                "den": rate.exact.den(),
                "fps": rate.exact.as_f64(),
                "display_rounded": rate.rounded,
            })),
            "has_audio": metadata.has_audio,
        },
        "capabilities": capabilities,
        "streams": streams,
    });
    let mut human = format!("format: {}\n", probe.format);
    match probe.duration {
        Some(duration) => {
            let _ = writeln!(
                human,
                "duration: {:.3} s ({} ticks)",
                duration.seconds(),
                duration.0
            );
        }
        None => human.push_str("duration: unknown\n"),
    }
    let _ = writeln!(human, "kind: {}", media_kind(metadata.kind));
    if let (Some(width), Some(height)) = (metadata.width, metadata.height) {
        let _ = writeln!(human, "size: {width}x{height}");
    }
    if let Some(rate) = metadata.display_frame_rate {
        let _ = writeln!(
            human,
            "frame rate: {} fps (exact {}/{}, display {})",
            rate.exact.label(),
            rate.exact.num(),
            rate.exact.den(),
            rate.rounded
        );
    }
    let _ = writeln!(human, "has_audio: {}", metadata.has_audio);
    for stream in &probe.streams {
        let _ = writeln!(human, "{}", stream_human(stream));
    }
    for stream in &capabilities {
        let _ = writeln!(
            human,
            "capability: stream {} {} {}",
            stream["index"],
            stream["kind"],
            if stream["decodable"].as_bool() == Some(true) {
                "decodable".to_owned()
            } else {
                format!("unsupported ({})", stream["reason"])
            }
        );
    }
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
}

fn media_kind(kind: media::MediaKind) -> &'static str {
    match kind {
        media::MediaKind::Video => "video",
        media::MediaKind::Image => "image",
        media::MediaKind::Audio => "audio",
    }
}

fn stream_json(stream: &media::StreamInfo) -> Value {
    json!({
        "index": stream.index,
        "kind": stream_kind(stream.kind),
        "codec": stream.codec,
        "time_base": {
            "numerator": stream.time_base.numerator(),
            "denominator": stream.time_base.denominator(),
        },
        "duration": optional_tick_json(stream.duration),
        "start": optional_tick_json(stream.start),
        "frame_rate": stream.frame_rate.map(|rate| json!({
            "num": rate.num(),
            "den": rate.den(),
            "fps": rate.as_f64(),
        })),
        "frame_count": stream.frame_count,
        "variable_frame_rate": stream.variable_frame_rate,
        "video": stream.video.as_ref().map(|video| json!({
            "width": video.width,
            "height": video.height,
            "color": video.color,
            "bit_depth": video.bit_depth,
        })),
        "audio": stream.audio.as_ref().map(|audio| json!({
            "sample_rate": audio.sample_rate,
            "channels": audio.channels,
            "initial_padding": audio.initial_padding,
            "trailing_padding": audio.trailing_padding,
        })),
    })
}

fn stream_human(stream: &media::StreamInfo) -> String {
    let mut line = format!(
        "stream {}: {} {}",
        stream.index,
        stream_kind(stream.kind),
        stream.codec
    );
    if let Some(video) = &stream.video {
        let _ = write!(line, " {}x{}", video.width, video.height);
    }
    if let Some(rate) = stream.frame_rate {
        let _ = write!(line, " {} fps", rate.label());
    }
    if let Some(audio) = &stream.audio {
        let _ = write!(line, " {} Hz {} ch", audio.sample_rate, audio.channels);
    }
    line
}

fn stream_kind(kind: media::StreamKind) -> &'static str {
    match kind {
        media::StreamKind::Video => "video",
        media::StreamKind::Audio => "audio",
        media::StreamKind::Other => "other",
    }
}

fn optional_tick_json(tick: Option<Tick>) -> Value {
    tick.map_or(Value::Null, tick_json)
}
