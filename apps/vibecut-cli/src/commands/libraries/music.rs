//! Adapts music-library requests to the registry and the libraries owner.
use crate::args::CliError;
use crate::args::{parse_i64, parse_u32, utf8};
use crate::output::tick_json;
use crate::output::{Output, hex, json_path};
use crate::session::{
    Context, Edit, History, Lent, WritePolicy, cli_runner, resolve_target, run_command, run_leased,
    standalone_session, with_music,
};
use engine::control::{ControlResult, MusicEntry, MusicSource};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use time::Tick;

/// `music` owns no provider logic: it composes the lent library adapter and the
/// cached-add service. Search runs remote anonymously, or with the user's own key
/// from the environment, and reads the saved local collection on request; add
/// never touches the network.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli music search|save|unsave|add ...".into())
    })?;
    let action = utf8(&action, "music action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "search" => music_search(context, rest),
        "save" => music_save(context, rest),
        "unsave" => music_unsave(context, rest),
        "add" => music_add(rest),
        other => Err(CliError::Usage(format!(
            "unknown music action: {other}; expected search, save, unsave or add"
        ))),
    }
}

fn music_search(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut library = None;
    let mut page = 1u32;
    let mut page_size = 15u32;
    let mut tag = None;
    let mut saved = false;
    let mut query = Vec::new();
    while let Some(arg) = args.next() {
        let value = utf8(&arg, "music search argument")?;
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
            "--tag" => {
                tag = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--tag requires a value".into()))?,
                        "--tag",
                    )?
                    .to_owned(),
                );
            }
            "--saved" => saved = true,
            "--" => query.extend(args.by_ref().map(|arg| arg.to_string_lossy().into_owned())),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!(
                    "unknown music search flag: {other}"
                )));
            }
            other => query.push(other.to_owned()),
        }
    }
    let library_path =
        library.ok_or_else(|| CliError::Usage("music search requires --library <dir>".into()))?;
    let page = match music_call(
        context,
        &library_path,
        !saved,
        "music.search",
        json!({
            "query": query.join(" "),
            "tag": tag,
            "page": page,
            "page_size": page_size,
            "saved": saved,
        }),
    )? {
        ControlResult::MusicSearch { page } => page,
        _ => return Err(CliError::Rejected("music.search returned no page")),
    };
    let results: Vec<Value> = page
        .results
        .iter()
        .map(|music| {
            json!({
                "id": music.id,
                "title": music.title,
                "author": music.author,
                "licence": music.licence,
                "licence_url": music.licence_url,
                "attribution": music.attribution,
                "source_url": music.source_url,
                "audio_url": music.audio_url,
                "duration_ms": music.duration_ms,
            })
        })
        .collect();
    let source = match page.source {
        MusicSource::Remote => "remote",
        MusicSource::Local => "local",
    };
    let value = json!({
        "source": source,
        "count": page.count,
        "page": page.page,
        "page_size": page.page_size,
        "has_next": page.has_next,
        "results": results,
    });
    let mut human = format!(
        "{} result(s) from {source}; page {} of size {}\n",
        page.results.len(),
        page.page,
        page.page_size
    );
    for music in &page.results {
        let _ = writeln!(
            human,
            "{}  {} - {} - {} - {} ms",
            music.id, music.title, music.author, music.licence, music.duration_ms
        );
    }
    Ok(Output::new(value, human.trim_end().to_owned(), false, 0))
}

fn music_save(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let library = PathBuf::from(args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli music save <library> <track-json>".into())
    })?);
    let raw = args
        .next()
        .ok_or_else(|| CliError::Usage("missing track JSON".into()))?;
    if args.next().is_some() {
        return Err(CliError::Usage(
            "usage: vibecut-cli music save <library> <track-json>".into(),
        ));
    }
    let raw = utf8(&raw, "track JSON")?;
    if raw.len() > 64 * 1024 {
        return Err(CliError::Usage("track JSON is too large".into()));
    }
    let music: MusicEntry =
        serde_json::from_str(raw).map_err(|_| CliError::Usage("invalid track JSON".into()))?;
    let saved = match music_call(
        context,
        &library,
        false,
        "music.save",
        json!({"music": music}),
    )? {
        ControlResult::MusicSaved { saved } => saved,
        _ => return Err(CliError::Rejected("music.save returned no record")),
    };
    Ok(Output::new(
        json!({
            "id": &saved.music.id,
            "saved_at_unix_ms": saved.saved_at_unix_ms,
            "attribution": saved.music.attribution,
            "licence": saved.music.licence,
            "licence_url": saved.music.licence_url,
        }),
        format!("saved track {}", saved.music.id),
        false,
        0,
    ))
}

fn music_unsave(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let library =
        PathBuf::from(args.next().ok_or_else(|| {
            CliError::Usage("usage: vibecut-cli music unsave <library> <id>".into())
        })?);
    let id = utf8(
        &args
            .next()
            .ok_or_else(|| CliError::Usage("missing track id".into()))?,
        "track id",
    )?
    .to_owned();
    if id.is_empty() || id.len() > 128 || args.next().is_some() {
        return Err(CliError::Usage(
            "usage: vibecut-cli music unsave <library> <id>".into(),
        ));
    }
    let removed = match music_call(context, &library, false, "music.unsave", json!({"id": &id}))? {
        ControlResult::MusicUnsaved { removed, .. } => removed,
        _ => return Err(CliError::Rejected("music.unsave returned no record")),
    };
    Ok(Output::new(
        json!({"id": id, "removed": removed}),
        if removed {
            format!("unsaved track {id}")
        } else {
            format!("track {id} was not saved")
        },
        false,
        0,
    ))
}

fn music_add(args: Vec<OsString>) -> Result<Output, CliError> {
    use libraries::music::{MusicLibrary, add_saved};
    let mut args = args.into_iter();
    let project = PathBuf::from(args.next().ok_or_else(|| {
        CliError::Usage(
            "usage: vibecut-cli music add <project> <library> <id> [--start <ticks>]".into(),
        )
    })?);
    let library = PathBuf::from(
        args.next()
            .ok_or_else(|| CliError::Usage("missing music library directory".into()))?,
    );
    let id = utf8(
        &args
            .next()
            .ok_or_else(|| CliError::Usage("missing track id".into()))?,
        "track id",
    )?
    .to_owned();
    let mut start = Tick::ZERO;
    while let Some(arg) = args.next() {
        let value = utf8(&arg, "music add flag")?;
        match value {
            "--start" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--start requires ticks".into()))?;
                start = Tick(parse_i64(Some(raw), "start")?);
            }
            other => {
                return Err(CliError::Usage(format!("unknown music add flag: {other}")));
            }
        }
    }
    let library = MusicLibrary::open(&library)?;
    // Resolve the project once: the asset keeps the absolute media path, so it stays
    // valid from any later working directory or alias of the project.
    let project = resolve_target(&project)?;
    // Project-owned media sits beside the project file, so library cleanup never
    // removes bytes this project references.
    let media = project::media_dir(Some(project.as_path()));
    let saved = run_leased(&project, History::Single, WritePolicy::Always, |session| {
        let outcome = add_saved(
            session,
            &library,
            &id,
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
        "music add published no project receipt",
    ))?;
    let placements: Vec<Value> = outcome
        .placements
        .iter()
        .map(|placement| json!({"track": placement.track.0, "item": placement.item.0}))
        .collect();
    let value = json!({
        "path": json_path(receipt.path())?,
        "content_hash": hex(&receipt.content_hash().0),
        "music": &id,
        "start": tick_json(start),
        "placements": placements,
    });
    Ok(Output::new(
        value,
        format!("added track {id} at {} ticks", start.0),
        false,
        0,
    ))
}

/// Run one library command through the registry with the opened root lent. Only a
/// remote search is lent the credential resolver; Saved, save and unsave are
/// offline administration and never resolve a key.
fn music_call(
    context: &Context,
    library: &Path,
    remote: bool,
    command: &str,
    params: Value,
) -> Result<ControlResult, CliError> {
    let mut session = standalone_session()?;
    let mut runner = cli_runner(context.budget()?)?;
    let result = with_music(library, remote, |music| {
        run_command(
            &mut session,
            &mut runner,
            Lent {
                music: Some(music),
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
