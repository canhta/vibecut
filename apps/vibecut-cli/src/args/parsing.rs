//! Interprets native CLI argument values.
use crate::args::CliError;
use engine::subtitles::LanguageTag;
use std::ffi::OsString;
use std::path::PathBuf;
use time::Tick;

pub(crate) const HELP: &str = "\
vibecut-cli - headless VibeCut control

Usage:
  vibecut-cli scenes <file> [--threshold 0.28] [--fade-threshold 0.20] [--min-scene-ticks <ticks>]
  vibecut-cli color lut <project> --import <file> --item <id> [--strength <0..100>]
  vibecut-cli probe <file>
  vibecut-cli media waveform <project> --asset <id>
  vibecut-cli media thumbnails <project> --item <id> --pixels-per-second <zoom> [--start <tick> --end <tick>]
  vibecut-cli project new <path>
  vibecut-cli project info <path>
  vibecut-cli exec <project> <command_id> <json>
  vibecut-cli normalize <project> [--clip] [--master] [--target-lufs -14] [--true-peak-db -1]
  vibecut-cli control <project> [--allow-overwrite] [--no-save]
  vibecut-cli mcp <project> [--output-dir <directory>] [--all-tools] [--no-save]
  vibecut-cli mcp --attach --socket <endpoint>
  vibecut-cli export <project> --out <mp4|mov|webm> [--settings <JSON>] [--aspect 9:16] [--allow-degraded] [--overwrite]
  vibecut-cli batch <job-file> [--allow-degraded] [--allow-overwrite]
  vibecut-cli captions import <project> <file.ass> [--language <tag>] [--start <ticks>]
                       [--track <id>] [--name <name>]
  vibecut-cli cut <project> --remove-pauses [--min-pause-ms 500] [--pad-ms 20]
                   [--crossfade-ms 20] [--dry-run]
  vibecut-cli transcript <project> [--item <id> --cue <id> --words <first>-<last>]
  vibecut-cli reframe <project> --item <id> --target 9:16|1:1|16:9
                   [--replace-manual] [--dry-run]
  vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]
                   [--srt <out>] [--burn <out.mp4>] [--aspect 9:16]
                   [--provider <id>] [--model <id>] [--translate-provider <id>]
                   [--translate-model <id>] [--yes-upload] [--overwrite]
  vibecut-cli clips <in> [--from vi|en|zh|auto] [--count N]
                    [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s]
                    [--provider heuristic|openai|anthropic] [--model <id>]
                    [--asr-provider <id>] [--asr-model <id>] [--keywords a,b]
                    [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual]
                     [--aspect 9:16] [--translate-provider <id>] [--overwrite]
                     [--allow-degraded]]
                    [--yes-upload]
  vibecut-cli chapters <project> --language vi|en|zh|zh-Hans|zh-Hant
                     [--max-chapters 10] [--provider openai|anthropic] [--model <id>]
                     [--export <dir>] [--yes-upload]
  vibecut-cli fillers <project> --language vi|en|zh|zh-Hans|zh-Hant --llm
                     [--include-markers] [--provider openai|anthropic] [--model <id>]
                     [--yes-upload]
  vibecut-cli providers [--operation transcribe|translate|clips|chapters|fillers] [--language <tag>]
  vibecut-cli sounds search --library <dir> [--page N] [--page-size N] [--all-licences] [query]
  vibecut-cli sounds save <library> <sound-json>
  vibecut-cli sounds unsave <library> <id>
  vibecut-cli sounds add <project> <library> <id> [--start <ticks>]
  vibecut-cli model list
  vibecut-cli model info <id>
  vibecut-cli model download <id> --yes-download
  vibecut-cli fonts list [--query <text>]
  vibecut-cli fonts resolve <family>
  vibecut-cli fonts download <family> --accept-license
  vibecut-cli commands
  vibecut-cli schema
  vibecut-cli --help
  vibecut-cli --version

Global:
  --json  print a JSON result on stdout; progress and errors go to stderr

Commands mutate projects only through the engine registry. Run
`vibecut-cli commands --json` for the generated command and parameter schema.";

pub(crate) fn number_arg(
    args: &mut std::vec::IntoIter<OsString>,
    flag: &str,
) -> Result<usize, CliError> {
    let raw = args
        .next()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a number")))?;
    let value = utf8(&raw, flag)?;
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("{flag} requires a nonnegative integer")))
}
pub(crate) fn seconds_arg(
    args: &mut std::vec::IntoIter<OsString>,
    flag: &str,
) -> Result<f64, CliError> {
    let raw = args
        .next()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a duration such as 20s")))?;
    let value = utf8(&raw, flag)?;
    let number = value.strip_suffix('s').unwrap_or(value);
    let parsed: f64 = number
        .parse()
        .map_err(|_| CliError::Usage(format!("{flag} must be seconds such as 20s")))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(CliError::Usage(format!("{flag} must be positive")));
    }
    Ok(parsed)
}
pub(crate) fn seconds_ticks(value: f64) -> Result<Tick, CliError> {
    Ok(Tick::from_seconds_f64(value, time::Rounding::Nearest)?)
}
pub(crate) fn parse_selection(value: &str) -> Result<Vec<usize>, CliError> {
    let mut result: Vec<usize> = Vec::new();
    for part in value.split(',') {
        let number: usize = part.trim().parse().map_err(|_| {
            CliError::Usage("--select takes comma-separated 1-based indices".into())
        })?;
        if number == 0 {
            return Err(CliError::Usage("--select indices start at 1".into()));
        }
        let index = number - 1;
        if result.contains(&index) {
            return Err(CliError::Usage("--select indices must be distinct".into()));
        }
        result.push(index);
    }
    if result.is_empty() || result.len() > 1000 {
        return Err(CliError::Usage(
            "--select takes 1..=1000 comma-separated indices".into(),
        ));
    }
    Ok(result)
}

pub(crate) fn parse_u32(value: Option<OsString>, what: &str) -> Result<u32, CliError> {
    let value = value.ok_or_else(|| CliError::Usage(format!("{what} requires a value")))?;
    utf8(&value, what)?
        .parse()
        .map_err(|_| CliError::Usage(format!("{what} must be a non-negative integer")))
}
pub(crate) fn parse_u64(value: Option<OsString>, what: &str) -> Result<u64, CliError> {
    let value = value.ok_or_else(|| CliError::Usage(format!("{what} requires a value")))?;
    utf8(&value, what)?
        .parse()
        .map_err(|_| CliError::Usage(format!("{what} must be a non-negative integer")))
}
pub(crate) fn parse_i64(value: Option<OsString>, what: &str) -> Result<i64, CliError> {
    let value = value.ok_or_else(|| CliError::Usage(format!("{what} requires a value")))?;
    utf8(&value, what)?
        .parse()
        .map_err(|_| CliError::Usage(format!("{what} must be an integer")))
}
pub(crate) fn parse_f64(value: Option<OsString>, what: &str) -> Result<f64, CliError> {
    let value = value.ok_or_else(|| CliError::Usage(format!("{what} requires a value")))?;
    let value: f64 = utf8(&value, what)?
        .parse()
        .map_err(|_| CliError::Usage(format!("{what} must be a number")))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CliError::Usage(format!("{what} must be finite")))
    }
}

pub(crate) fn one_utf8(args: Vec<OsString>, usage: &str) -> Result<String, CliError> {
    let mut args = args.into_iter();
    let value = args
        .next()
        .ok_or_else(|| CliError::Usage(usage.to_owned()))?;
    if args.next().is_some() {
        return Err(CliError::Usage(usage.to_owned()));
    }
    Ok(utf8(&value, "argument")?.to_owned())
}

pub(crate) fn one_path(args: Vec<OsString>, usage: &str) -> Result<PathBuf, CliError> {
    let mut args = args.into_iter();
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Usage(usage.to_owned()))?;
    if args.next().is_some() {
        return Err(CliError::Usage(usage.to_owned()));
    }
    Ok(path)
}

pub(crate) fn utf8<'a>(value: &'a OsString, what: &str) -> Result<&'a str, CliError> {
    value
        .to_str()
        .ok_or_else(|| CliError::Usage(format!("{what} must be valid UTF-8")))
}

pub(crate) fn translation_required(
    source: &LanguageTag,
    target: &LanguageTag,
    bilingual: bool,
) -> bool {
    bilingual || source != target
}
