//! Adapts sound-library requests to the libraries owner.
use crate::args::CliError;
use crate::args::{parse_i64, parse_u32, parse_u64, utf8};
use crate::output::tick_json;
use crate::output::{Output, hex, json_path};
use crate::session::{
    Context, Edit, History, Lent, WritePolicy, cli_runner, run_command, run_leased,
    standalone_session, with_sounds,
};
use engine::control::{ControlResult, SoundEntry, SoundSource};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use time::Tick;

/// `sounds` owns no provider logic: it composes the lent library and cached-add
/// service. Search runs remote with a user token and degrades to the saved local
/// library without one; add never touches the network.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli sounds search|save|unsave|add ...".into())
    })?;
    let action = utf8(&action, "sounds action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "search" => sounds_search(context, rest),
        "save" => sounds_save(context, rest),
        "unsave" => sounds_unsave(context, rest),
        "add" => sounds_add(rest),
        other => Err(CliError::Usage(format!(
            "unknown sounds action: {other}; expected search, save, unsave or add"
        ))),
    }
}

fn sounds_search(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut library = None;
    let mut page = 1u32;
    let mut page_size = 15u32;
    let mut commercial_only = true;
    let mut query = Vec::new();
    while let Some(arg) = args.next() {
        let value = utf8(&arg, "sounds search argument")?;
        match value {
            "--library" => {
                library = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--library requires a directory".into())
                })?));
            }
            "--page" => {
                page = parse_u32(args.next(), "--page")?;
            }
            "--page-size" => {
                page_size = parse_u32(args.next(), "--page-size")?;
            }
            "--all-licences" => commercial_only = false,
            "--" => query.extend(args.by_ref().map(|arg| arg.to_string_lossy().into_owned())),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!(
                    "unknown sounds search flag: {other}"
                )));
            }
            other => query.push(other.to_owned()),
        }
    }
    let library_path =
        library.ok_or_else(|| CliError::Usage("sounds search requires --library <dir>".into()))?;
    let query = query.join(" ");
    let page = match sounds_call(
        context,
        &library_path,
        "sounds.search",
        json!({
            "query": query,
            "page": page,
            "page_size": page_size,
            "commercial_only": commercial_only,
        }),
    )? {
        ControlResult::SoundSearch { page } => page,
        _ => return Err(CliError::Rejected("sounds.search returned no page")),
    };
    let results: Vec<Value> = page
        .results
        .iter()
        .map(|sound| {
            json!({
                "id": sound.id,
                "name": sound.name,
                "author": sound.author,
                "licence": sound.licence,
                "attribution": sound.attribution,
                "source_url": sound.source_url,
                "audio_url": sound.audio_url,
                "duration_ms": sound.duration_ms,
                "downloads": sound.downloads,
            })
        })
        .collect();
    let source = match page.source {
        SoundSource::Remote => "remote",
        SoundSource::Local => "local",
    };
    let value = json!({
        "source": source,
        "count": page.count,
        "page": page.page,
        "page_size": page.page_size,
        "has_next": page.has_next,
        "commercial_only": page.commercial_only,
        "results": results,
    });
    let mut human = format!(
        "{} result(s) from {source}; page {} of size {}\n",
        page.results.len(),
        page.page,
        page.page_size
    );
    for sound in &page.results {
        let _ = writeln!(
            human,
            "{}  {} — {} — {} — {} downloads",
            sound.id, sound.name, sound.author, sound.licence, sound.downloads
        );
    }
    Ok(Output::new(value, human.trim_end().to_owned(), false, 0))
}

fn sounds_save(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let library = PathBuf::from(args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli sounds save <library> <sound-json>".into())
    })?);
    let raw = args
        .next()
        .ok_or_else(|| CliError::Usage("missing sound JSON".into()))?;
    if args.next().is_some() {
        return Err(CliError::Usage(
            "usage: vibecut-cli sounds save <library> <sound-json>".into(),
        ));
    }
    let raw = utf8(&raw, "sound JSON")?;
    if raw.len() > 64 * 1024 {
        return Err(CliError::Usage("sound JSON is too large".into()));
    }
    let sound: SoundEntry =
        serde_json::from_str(raw).map_err(|_| CliError::Usage("invalid sound JSON".into()))?;
    let saved = match sounds_call(context, &library, "sounds.save", json!({"sound": sound}))? {
        ControlResult::SoundSaved { saved } => saved,
        _ => return Err(CliError::Rejected("sounds.save returned no record")),
    };
    Ok(Output::new(
        json!({
            "id": saved.id,
            "saved_at_unix_ms": saved.saved_at_unix_ms,
            "attribution": saved.attribution,
            "licence": saved.licence,
        }),
        format!("saved sound {}", saved.id),
        false,
        0,
    ))
}

fn sounds_unsave(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let library = PathBuf::from(args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli sounds unsave <library> <id>".into())
    })?);
    let id = parse_u64(args.next(), "sound id")?;
    if args.next().is_some() {
        return Err(CliError::Usage(
            "usage: vibecut-cli sounds unsave <library> <id>".into(),
        ));
    }
    let removed = match sounds_call(context, &library, "sounds.unsave", json!({"id": id}))? {
        ControlResult::SoundUnsaved { removed, .. } => removed,
        _ => return Err(CliError::Rejected("sounds.unsave returned no record")),
    };
    Ok(Output::new(
        json!({"id": id, "removed": removed}),
        if removed {
            format!("unsaved sound {id}")
        } else {
            format!("sound {id} was not saved")
        },
        false,
        0,
    ))
}

fn sounds_add(args: Vec<OsString>) -> Result<Output, CliError> {
    use libraries::sounds::{SoundLibrary, add_saved};
    let mut args = args.into_iter();
    let project = PathBuf::from(args.next().ok_or_else(|| {
        CliError::Usage(
            "usage: vibecut-cli sounds add <project> <library> <id> [--start <ticks>]".into(),
        )
    })?);
    let library = PathBuf::from(
        args.next()
            .ok_or_else(|| CliError::Usage("missing sound library directory".into()))?,
    );
    let id = parse_u64(args.next(), "sound id")?;
    let mut start = Tick::ZERO;
    while let Some(arg) = args.next() {
        let value = utf8(&arg, "sounds add flag")?;
        match value {
            "--start" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--start requires ticks".into()))?;
                start = Tick(parse_i64(Some(raw), "start")?);
            }
            other => {
                return Err(CliError::Usage(format!("unknown sounds add flag: {other}")));
            }
        }
    }
    let library = SoundLibrary::open(&library)?;
    // Project-owned media sits beside the project file, so library cleanup never
    // removes bytes this project references.
    let media = project::media_dir(Some(project.as_path()));
    let saved = run_leased(&project, History::Single, WritePolicy::Always, |session| {
        let outcome = add_saved(
            session,
            &library,
            id,
            &media,
            engine::Placement::Auto,
            start,
        )?;
        let changed = outcome.changed;
        Ok(Edit {
            value: outcome,
            changed,
        })
    })?;
    let outcome = saved.value;
    let receipt = saved.receipt.ok_or(CliError::Unsupported(
        "sounds add published no project receipt",
    ))?;
    let placements: Vec<Value> = outcome
        .placements
        .iter()
        .map(|placement| json!({"track": placement.track.0, "item": placement.item.0}))
        .collect();
    let value = json!({
        "path": json_path(receipt.path())?,
        "content_hash": hex(&receipt.content_hash().0),
        "sound": id,
        "start": tick_json(start),
        "placements": placements,
    });
    Ok(Output::new(
        value,
        format!("added sound {id} at {} ticks", start.0),
        false,
        0,
    ))
}

/// Run one library command through the registry with the opened root and token lent.
fn sounds_call(
    context: &Context,
    library: &Path,
    command: &str,
    params: Value,
) -> Result<ControlResult, CliError> {
    let mut session = standalone_session()?;
    let mut runner = cli_runner(context.budget()?)?;
    let result = with_sounds(library, |sounds| {
        run_command(
            &mut session,
            &mut runner,
            Lent {
                sounds: Some(sounds),
                music: None,
                ..Lent::default()
            },
            command,
            params,
            false,
        )
    });
    runner.shutdown();
    result
}
