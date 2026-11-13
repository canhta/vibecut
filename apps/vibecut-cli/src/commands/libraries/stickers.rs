//! Adapts the bundled sticker catalog to the registry commands `stickers.search` and `stickers.insert`.
use crate::args::CliError;
use crate::args::{parse_i64, parse_u32, utf8};
use crate::output::{Output, hex, json_path, tick_json};
use crate::session::{
    Context, Edit, History, Lent, WritePolicy, cli_runner, run_command, run_leased,
    standalone_session, with_stickers,
};
use engine::control::ControlResult;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use time::{TICKS_PER_SECOND, Tick};

/// Seconds a sticker lasts when `--duration` is absent, the length every other generated item gets.
const DEFAULT_SECONDS: i64 = 5;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args
        .next()
        .ok_or_else(|| CliError::Usage("usage: vibecut-cli stickers search|insert ...".into()))?;
    let action = utf8(&action, "stickers action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "search" => stickers_search(context, rest),
        "insert" => stickers_insert(context, rest),
        other => Err(CliError::Usage(format!(
            "unknown stickers action: {other}; expected search or insert"
        ))),
    }
}

fn stickers_search(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut category = "all".to_owned();
    let mut limit = None;
    let mut query = Vec::new();
    while let Some(arg) = args.next() {
        let value = utf8(&arg, "stickers search argument")?;
        match value {
            "--category" => {
                let raw = args.next().ok_or_else(|| {
                    CliError::Usage("--category requires all, flags or shapes".into())
                })?;
                category = utf8(&raw, "category")?.to_owned();
            }
            "--limit" => limit = Some(parse_u32(args.next(), "--limit")?),
            "--" => query.extend(args.by_ref().map(|arg| arg.to_string_lossy().into_owned())),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!(
                    "unknown stickers search flag: {other}"
                )));
            }
            other => query.push(other.to_owned()),
        }
    }
    let page = match stickers_call(
        context,
        "stickers.search",
        json!({"query": query.join(" "), "category": category, "limit": limit}),
        false,
    )? {
        ControlResult::StickerSearch { page } => page,
        _ => return Err(CliError::Rejected("stickers.search returned no page")),
    };
    let mut human = format!("{} match(es), showing {}\n", page.total, page.results.len());
    for entry in &page.results {
        let _ = writeln!(
            human,
            "{}  {}  {}x{}",
            entry.id, entry.name, entry.width, entry.height
        );
    }
    Ok(Output::new(
        serde_json::to_value(&page)?,
        human.trim_end().to_owned(),
        false,
        0,
    ))
}

fn stickers_insert(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let usage = || {
        CliError::Usage(
            "usage: vibecut-cli stickers insert <project> <id> [--start <ticks>] [--duration <ticks>]"
                .into(),
        )
    };
    let project = PathBuf::from(args.next().ok_or_else(usage)?);
    let id = args.next().ok_or_else(usage)?;
    let id = utf8(&id, "sticker id")?.to_owned();
    let mut start = Tick::ZERO;
    let mut duration = Tick(DEFAULT_SECONDS * TICKS_PER_SECOND);
    while let Some(arg) = args.next() {
        let value = utf8(&arg, "stickers insert flag")?;
        match value {
            "--start" => start = Tick(parse_i64(args.next(), "start")?),
            "--duration" => duration = Tick(parse_i64(args.next(), "duration")?),
            other => {
                return Err(CliError::Usage(format!(
                    "unknown stickers insert flag: {other}"
                )));
            }
        }
    }
    let mut runner = cli_runner(context.budget()?)?;
    let saved = with_stickers(|stickers| {
        run_leased(&project, History::Single, WritePolicy::Changed, |session| {
            let result = run_command(
                session,
                &mut runner,
                Lent {
                    stickers: Some(stickers),
                    ..Lent::default()
                },
                "stickers.insert",
                json!({
                    "placement": {"mode": "auto"},
                    "sticker": id,
                    "start": start.0,
                    "duration": duration.0,
                }),
                true,
            )?;
            let ControlResult::Edit { outcome } = result else {
                return Err(CliError::Rejected("stickers.insert returned no outcome"));
            };
            let changed = outcome.changed;
            Ok(Edit {
                value: outcome,
                changed,
            })
        })
    });
    runner.shutdown();
    let saved = saved?;
    let outcome = saved.value;
    let receipt = saved.receipt.ok_or(CliError::Unsupported(
        "stickers insert published no project receipt",
    ))?;
    let placements: Vec<Value> = outcome
        .placements
        .iter()
        .map(|placement| json!({"track": placement.track.0, "item": placement.item.0}))
        .collect();
    let value = json!({
        "path": json_path(receipt.path())?,
        "content_hash": hex(&receipt.content_hash().0),
        "sticker": id,
        "start": tick_json(start),
        "duration": tick_json(duration),
        "placements": placements,
    });
    Ok(Output::new(
        value,
        format!("inserted sticker {id} at {} ticks", start.0),
        false,
        0,
    ))
}

/// Run one catalog command through the registry with the bundled catalog lent.
fn stickers_call(
    context: &Context,
    command: &str,
    params: Value,
    effectful: bool,
) -> Result<ControlResult, CliError> {
    let mut session = standalone_session()?;
    let mut runner = cli_runner(context.budget()?)?;
    let result = with_stickers(|stickers| {
        run_command(
            &mut session,
            &mut runner,
            Lent {
                stickers: Some(stickers),
                ..Lent::default()
            },
            command,
            params,
            effectful,
        )
    });
    runner.shutdown();
    result
}
