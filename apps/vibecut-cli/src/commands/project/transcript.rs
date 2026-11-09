//! Reads caption words and deletes a measured word range from a leased project.
use crate::args::{CliError, parse_u64, utf8};
use crate::output::{Output, json_path};
use crate::session::Context;
use engine::{Session, WordId};
use project::WriterLease;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;

/// `transcript <project>` lists cues and words with output times; `--item <id>
/// --cue <id> --words <first>-<last>` deletes that cue's measured word range as
/// one undoable edit.
pub(crate) fn run(_context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(transcript_usage)?;
    let mut item: Option<u64> = None;
    let mut cue: Option<String> = None;
    let mut words: Option<(usize, usize)> = None;
    while let Some(flag) = args.next() {
        match utf8(&flag, "transcript flag")? {
            "--item" => item = Some(parse_u64(args.next(), "--item")?),
            "--cue" => {
                let value = args.next().ok_or_else(transcript_usage)?;
                cue = Some(utf8(&value, "--cue")?.to_owned());
            }
            "--words" => {
                let value = args.next().ok_or_else(transcript_usage)?;
                words = Some(parse_range(utf8(&value, "--words")?)?);
            }
            "--help" | "-h" => {
                return Ok(Output::new(
                    json!({"usage": usage_text()}),
                    usage_text().to_owned(),
                    false,
                    0,
                ));
            }
            other => return Err(CliError::Usage(format!("unknown transcript flag: {other}"))),
        }
    }
    let selection = match (item, cue, words) {
        (None, None, None) => None,
        (Some(item), Some(cue), Some(words)) => Some((item, cue, words)),
        _ => return Err(transcript_usage()),
    };

    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    session.restore_metadata(lease.metadata())?;
    let view = session.transcript()?;

    let Some((item, cue, (first, last))) = selection else {
        let value = serde_json::to_value(&view)?;
        let words: usize = view.cues.iter().map(|cue| cue.words.len()).sum();
        let human = format!("{} cues, {} words", view.cues.len(), words);
        return Ok(Output::new(
            json!({"project": json_path(&project_path)?, "transcript": value}),
            human,
            false,
            0,
        ));
    };

    let entry = view
        .cues
        .iter()
        .find(|entry| entry.item.0 == item && entry.id.as_str() == cue.as_str())
        .ok_or_else(|| CliError::Usage("--item and --cue must name a listed cue".into()))?;
    if last >= entry.words.len() || first > last {
        return Err(CliError::Usage(
            "--words must name measured words of that cue".into(),
        ));
    }
    let ids: Vec<WordId> = (first..=last)
        .map(|index| WordId {
            item: entry.item,
            cue: entry.id.clone(),
            word: index,
        })
        .collect();
    let count = ids.len();
    let outcome = session.execute("transcript.delete_range", json!({ "word_ids": ids }))?;
    if outcome.changed {
        let _receipt = lease.save(session.project().as_ref())?;
        session.saved()?;
        let mut metadata = session.metadata().clone();
        metadata.touch();
        lease.save_metadata(&metadata)?;
    }
    let remaining = session
        .transcript()?
        .cues
        .iter()
        .map(|cue| cue.words.len())
        .sum::<usize>();
    let value = json!({
        "project": json_path(&project_path)?,
        "deleted": count,
        "changed": outcome.changed,
        "remaining_words": remaining,
    });
    let human = if outcome.changed {
        format!("deleted {count} words, {remaining} remain")
    } else {
        format!("no change: {remaining} words remain")
    };
    Ok(Output::new(value, human, false, 0))
}

/// `first-last` inclusive zero-based word indices.
fn parse_range(value: &str) -> Result<(usize, usize), CliError> {
    let (first, last) = value
        .split_once('-')
        .ok_or_else(|| CliError::Usage("--words takes <first>-<last>".into()))?;
    let first: usize = first
        .parse()
        .map_err(|_| CliError::Usage("--words takes <first>-<last>".into()))?;
    let last: usize = last
        .parse()
        .map_err(|_| CliError::Usage("--words takes <first>-<last>".into()))?;
    if first > last {
        return Err(CliError::Usage("--words is an inclusive range".into()));
    }
    Ok((first, last))
}

fn transcript_usage() -> CliError {
    CliError::Usage(usage_text().into())
}

fn usage_text() -> &'static str {
    "usage: vibecut-cli transcript <project> \
     [--item <id> --cue <id> --words <first>-<last>]"
}
