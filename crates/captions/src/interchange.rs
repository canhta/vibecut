//! Round-trips SRT and WebVTT subtitle files, and exports plain TXT cue text.
pub mod ass;

use crate::{Cue, CueText, CueTrack, Error, LanguageTag, Warning, WarningKind};
use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use std::collections::BTreeMap;
use time::{Rounding, SourceRange, Tick};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Srt,
    WebVtt,
    /// Plain caption text: one cue per line, no timings or header.
    Txt,
}
#[derive(Clone, Debug)]
pub struct Import {
    pub track: CueTrack,
    pub warnings: Vec<Warning>,
}
#[derive(Clone)]
pub struct Export {
    pub text: String,
    pub warnings: Vec<Warning>,
}
fn warn(warnings: &mut Vec<Warning>, position: usize, kind: WarningKind) {
    warnings.push(Warning { position, kind });
}
pub(crate) fn decode(bytes: &[u8], warnings: &mut Vec<Warning>) -> Result<String, Error> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error::TooLarge);
    }
    let (encoding, body) = if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        (UTF_8, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        (UTF_16LE, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        (UTF_16BE, rest)
    } else if std::str::from_utf8(bytes).is_ok() {
        (UTF_8, bytes)
    } else {
        (WINDOWS_1252, bytes)
    };
    if body.len() != bytes.len() {
        warn(warnings, 0, WarningKind::Bom);
    }
    if encoding != UTF_8 {
        warn(warnings, 0, WarningKind::Encoding);
    }
    let (text, replaced) = encoding.decode_without_bom_handling(body);
    if replaced {
        warn(warnings, 0, WarningKind::Replacement);
    }
    if text.contains('\r') {
        warn(warnings, 0, WarningKind::LineEndings);
    }
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}
fn clock(value: &str) -> Option<Tick> {
    let (whole, fraction) = value.rsplit_once([',', '.']).unwrap_or((value, "0"));
    if fraction.is_empty() || fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts: Vec<_> = whole.split(':').collect();
    let number = |s: &str| -> Option<i64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else {
            s.parse().ok()
        }
    };
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (number(h)?, number(m)?, number(s)?),
        [m, s] => (0, number(m)?, number(s)?),
        _ => return None,
    };
    if m >= 60 || s >= 60 {
        return None;
    }
    let scale = 10_i64.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let seconds = h.checked_mul(3600)?.checked_add(m * 60)?.checked_add(s)?;
    let whole = Tick::from_units(seconds, 1, Rounding::Nearest).ok()?;
    whole.checked_add(Tick::from_units(fraction.parse().ok()?, scale, Rounding::Nearest).ok()?)
}
fn timing(line: &str) -> Option<(Tick, Tick, bool)> {
    let (start, rest) = line.split_once("-->")?;
    let mut parts = rest.split_whitespace();
    Some((
        clock(start.trim())?,
        clock(parts.next()?)?,
        parts.next().is_some(),
    ))
}
fn format_ms(ms: i64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// One timed cue block with repaired bounds and its text lines.
struct Block<'a> {
    start: Tick,
    end: Tick,
    body: Vec<&'a str>,
}

/// Split decoded text into timed blocks, reporting VTT header/metadata differences.
fn read_blocks<'a>(
    text: &'a str,
    format: Format,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<Block<'a>>, Error> {
    if format == Format::Txt {
        return Err(Error::Invalid("TXT is export-only"));
    }
    let lines: Vec<_> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    if format == Format::WebVtt {
        if lines.first().is_some_and(|s| {
            *s == "WEBVTT" || s.starts_with("WEBVTT ") || s.starts_with("WEBVTT\t")
        }) {
            i = 1;
            while lines
                .get(i)
                .is_some_and(|s| !s.trim().is_empty() && timing(s).is_none())
            {
                warn(warnings, i + 1, WarningKind::Metadata);
                i += 1;
            }
        } else {
            warn(warnings, 0, WarningKind::MissingHeader);
        }
    }
    while let Some(line) = lines.get(i) {
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if format == Format::WebVtt
            && (*line == "NOTE"
                || line.starts_with("NOTE ")
                || line.starts_with("NOTE\t")
                || *line == "STYLE"
                || *line == "REGION")
            && !lines.get(i + 1).is_some_and(|s| timing(s).is_some())
        {
            warn(warnings, i + 1, WarningKind::Metadata);
            while lines.get(i).is_some_and(|s| !s.trim().is_empty()) {
                i += 1;
            }
            continue;
        }
        let position = i + 1;
        let Some((start, mut end, settings)) = timing(line) else {
            if !lines.get(i + 1).is_some_and(|s| timing(s).is_some()) {
                warn(warnings, position, WarningKind::Malformed);
            }
            i += 1;
            continue;
        };
        if settings {
            warn(warnings, position, WarningKind::Metadata);
        }
        i += 1;
        let mut body = Vec::new();
        while let Some(line) = lines.get(i) {
            if line.trim().is_empty() || timing(line).is_some() {
                break;
            }
            if line.trim().bytes().all(|b| b.is_ascii_digit())
                && lines.get(i + 1).is_some_and(|s| timing(s).is_some())
            {
                break;
            }
            body.push(*line);
            i += 1;
        }
        if end <= start {
            warn(warnings, position, WarningKind::Duration);
            end = start
                .checked_add(Tick::from_units(1, 1000, Rounding::Nearest)?)
                .ok_or(Error::Invalid("duration overflow"))?;
        }
        blocks.push(Block { start, end, body });
    }
    Ok(blocks)
}

/// Build one cue from a timed block; VTT entity decoding happens before storage.
fn cue(
    block: Block<'_>,
    id: usize,
    language: &LanguageTag,
    decode_entities: bool,
) -> Result<Cue, Error> {
    let text = block.body.join("\n");
    let text = if decode_entities {
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    } else {
        text
    };
    Ok(Cue {
        id: id.to_string().into(),
        range: SourceRange::new(block.start, block.end)?,
        source_lang: language.clone(),
        texts: BTreeMap::from([(
            language.clone(),
            CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        )]),
        word_ranges: Vec::new(),
        style: None,
    })
}

/// Decode and parse a single source language. Malformed entries produce positional warnings.
pub fn parse(bytes: &[u8], format: Format, language: LanguageTag) -> Result<Import, Error> {
    let mut warnings = Vec::new();
    let text = decode(bytes, &mut warnings)?;
    let decode_entities = format == Format::WebVtt;
    let cues = read_blocks(&text, format, &mut warnings)?
        .into_iter()
        .enumerate()
        .map(|(index, block)| cue(block, index + 1, &language, decode_entities))
        .collect::<Result<Vec<_>, _>>()?;
    let mut track = CueTrack {
        styles: Vec::new(),
        cues,
    };
    warnings.extend(track.repair()?);
    Ok(Import { track, warnings })
}

/// Export one language or an ordered bilingual pair. Missing/empty entries warn and are omitted.
pub fn export(
    track: &CueTrack,
    format: Format,
    languages: &[LanguageTag],
) -> Result<Export, Error> {
    if languages.is_empty()
        || languages.len() > 2
        || (languages.len() == 2 && languages.first() == languages.get(1))
    {
        return Err(Error::Invalid(
            "select one language or two distinct languages",
        ));
    }
    let mut track = track.clone();
    let mut warnings = track.repair()?;
    let mut text = if format == Format::WebVtt {
        "WEBVTT\n\n".to_owned()
    } else {
        String::new()
    };
    let mut count = 0;
    let mut last_end = 0;
    for (i, cue) in track.cues.iter().enumerate() {
        let mut body = Vec::new();
        for lang in languages {
            let Some(entry) = cue.texts.get(lang) else {
                warn(&mut warnings, i + 1, WarningKind::MissingLanguage);
                continue;
            };
            if entry.text.as_str().trim().is_empty() {
                warn(&mut warnings, i + 1, WarningKind::EmptyText);
                continue;
            }
            let normalized = entry
                .text
                .as_str()
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let compact = normalized
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if compact != entry.text.as_str() {
                warn(&mut warnings, i + 1, WarningKind::TextLayout);
            }
            body.push(compact);
        }
        if body.is_empty() {
            continue;
        }
        if format == Format::Txt {
            // Plain download: one cue per line, languages side by side, no clock.
            let line = body.join(" / ");
            let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if line.is_empty() {
                continue;
            }
            text.push_str(&line);
            text.push('\n');
            continue;
        }
        let mut start = cue.range.start().to_units(1000, Rounding::Nearest)?;
        let mut end = cue.range.end().to_units(1000, Rounding::Nearest)?;
        start = start.max(last_end);
        if end <= start {
            end = start
                .checked_add(1)
                .ok_or(Error::Invalid("millisecond overflow"))?;
            warn(&mut warnings, i + 1, WarningKind::RoundedDuration);
        }
        // Exported timestamps must remain representable when imported again.
        Tick::from_units(end, 1000, Rounding::Nearest)?;
        last_end = end;
        count += 1;
        if format == Format::Srt {
            text.push_str(&format!("{count}\n"));
        }
        let separator = if format == Format::Srt { ',' } else { '.' };
        text.push_str(&format!(
            "{} --> {}\n",
            format_ms(start, separator),
            format_ms(end, separator)
        ));
        let payload = body.join("\n");
        if format == Format::Srt && payload.lines().any(|line| timing(line).is_some()) {
            return Err(Error::Invalid("SRT text resembles a timing line"));
        }
        let payload = if format == Format::WebVtt {
            payload
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        } else {
            payload
        };
        text.push_str(&payload);
        text.push_str("\n\n");
    }
    Ok(Export { text, warnings })
}
