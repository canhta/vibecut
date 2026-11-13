//! Reads bounded views of the fork's project for the model.
use engine::Session;
use project::{Item, ItemParams, Track};
use serde_json::{Value, json};

/// Characters of a text item the model reads; the rest is cut.
const TEXT_CHARS: usize = 200;

fn kind_name(content: &ItemParams) -> &'static str {
    match content {
        ItemParams::Video(_) => "video",
        ItemParams::Audio(_) => "audio",
        ItemParams::Text(_) => "text",
        ItemParams::Caption(_) => "caption",
        ItemParams::Overlay(_) => "overlay",
        ItemParams::Graphic(_) => "graphic",
        ItemParams::Sticker(_) => "sticker",
    }
}

fn track_json(track: &Track) -> Value {
    json!({
        "id": track.id,
        "name": track.name.as_str(),
        "kind": track.kind,
        "muted": track.muted,
        "hidden": track.hidden,
        "locked": track.locked,
        "items": track.items.len(),
    })
}

fn item_json(track: &Track, item: &Item) -> Value {
    let mut entry = json!({
        "id": item.id,
        "track": track.id,
        "kind": kind_name(&item.content),
        "start": item.start,
        "end": item.end().ok(),
        "hidden": item.hidden,
        "source_asset": item.source_asset,
    });
    if let (ItemParams::Text(text), Some(fields)) = (&item.content, entry.as_object_mut()) {
        let shown: String = text.content.as_str().chars().take(TEXT_CHARS).collect();
        fields.insert("text".into(), Value::String(shown));
    }
    entry
}

/// Project name, canvas, duration, tracks and media. Media file names appear only when
/// the conversation may send clip names; folders never appear.
pub(crate) fn project_summary(session: &Session, clip_names: bool) -> Value {
    let project = session.project();
    let data = project.data();
    let scene = data.scene();
    let tracks: Vec<Value> = scene.timeline.iter().rev().map(track_json).collect();
    let assets: Vec<Value> = data
        .assets
        .iter()
        .map(|asset| {
            let mut entry = json!({"id": asset.id()});
            if clip_names
                && let (Some(name), Some(fields)) = (
                    asset.path().file_name().and_then(|name| name.to_str()),
                    entry.as_object_mut(),
                )
            {
                fields.insert("name".into(), Value::String(name.to_owned()));
            }
            entry
        })
        .collect();
    json!({
        "revision": session.revision(),
        "name": data.name.as_str(),
        "canvas": {"width": data.canvas.width, "height": data.canvas.height, "fps": data.canvas.fps},
        "duration": project.duration(),
        "scene": {"id": scene.id, "name": scene.name.as_str()},
        "scenes": data.scenes.iter().count(),
        "tracks": tracks,
        "assets": assets,
    })
}

/// One page of the active scene's items in screen order, top track first, optionally
/// limited to one track.
pub(crate) fn timeline_read(
    session: &Session,
    track: Option<u64>,
    cursor: usize,
    limit: usize,
) -> Value {
    let scene = session.project().data().scene();
    let mut entries = scene
        .timeline
        .iter()
        .rev()
        .filter(|candidate| track.is_none_or(|wanted| candidate.id.0 == wanted))
        .flat_map(|candidate| {
            candidate
                .items
                .iter()
                .map(move |item| item_json(candidate, item))
        })
        .skip(cursor);
    let items: Vec<Value> = entries.by_ref().take(limit).collect();
    let next = entries.next().is_some().then(|| cursor + items.len());
    json!({"revision": session.revision(), "items": items, "next_cursor": next})
}

/// One page of caption cues in output time. Words are left out; `timeline.*` and
/// `transcript.*` commands address them by cue.
pub(crate) fn transcript_read(
    session: &Session,
    cursor: usize,
    limit: usize,
) -> Result<Value, String> {
    let view = session
        .transcript()
        .map_err(|_| "the transcript cannot be mapped to output time".to_owned())?;
    let mut cues = view.cues.iter().skip(cursor);
    let mut page: Vec<Value> = Vec::new();
    for cue in cues.by_ref().take(limit) {
        let mut entry = serde_json::to_value(cue).map_err(|_| "cue encoding failed".to_owned())?;
        if let Some(fields) = entry.as_object_mut() {
            fields.remove("words");
        }
        page.push(entry);
    }
    let next = cues.next().is_some().then(|| cursor + page.len());
    Ok(json!({"revision": session.revision(), "cues": page, "next_cursor": next}))
}
