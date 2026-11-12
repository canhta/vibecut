//! Drives README capture staging, manifest verification, publication transfer.
#![allow(
    clippy::indexing_slicing,
    reason = "an offline audit capture indexes its own manifest and fixed-size frame buffers; a panic aborts the tool, never a person's project"
)]
use super::Failure;
use super::application::{App, SECOND, harness_sized};
use engine::Command;
use engine::CreatedId;
use engine::TrimEdge;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use time::Tick;
pub(super) fn unique_run_dir(target: &Path, slot: &str) -> Result<PathBuf, Failure> {
    let root = target.join("readme-media").join(slot);
    fs::create_dir_all(&root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock before epoch")?
        .as_millis();
    for attempt in 0..1000 {
        let run = root.join(format!("{stamp}-{}-{attempt}", std::process::id()));
        if !run.exists() {
            fs::create_dir(&run)?;
            return Ok(run);
        }
    }
    Err("could not allocate a unique README run directory".into())
}

pub(crate) fn readme_media(
    root: &Path,
    target: &Path,
    slot: &str,
    source: &Path,
) -> Result<(), Failure> {
    if !matches!(slot, "hero" | "timeline") {
        return Err(format!("unknown README slot `{slot}`; expected hero or timeline").into());
    }
    let source = std::fs::canonicalize(source).map_err(|error| {
        format!(
            "README source `{}` is not readable: {error}",
            source.display()
        )
    })?;
    if !source.is_file() {
        return Err("README source must be a local file".into());
    }
    let source_hash = stream_sha256(&source)?;
    let tools = [tool_identity("ffmpeg")?, tool_identity("ffprobe")?];
    let free = free_bytes(target)?;
    if free < README_SCRATCH {
        return Err(format!(
            "README capture needs at least 8 GiB scratch headroom; found {free} bytes"
        )
        .into());
    }
    let run = unique_run_dir(target, slot)?;
    let result = readme_media_run(root, slot, &source, &run, &source_hash, &tools);
    if result.is_err() && !run.join("preserve-staging").exists() {
        fs::remove_dir_all(&run).ok();
    }
    result
}

pub(super) fn readme_media_run(
    root: &Path,
    slot: &str,
    source: &Path,
    run: &Path,
    source_hash: &str,
    tools: &[String; 2],
) -> Result<(), Failure> {
    let commit = git_head(root)?;
    let (editor, first, _second) = readme_project(source)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), 1.0, README_WINDOW);
    readme_seek(&mut app, Tick(3 * SECOND))?;
    let at_three = readme_frame_digest(&mut app)?;
    readme_seek(&mut app, Tick(SECOND))?;
    let at_one = readme_frame_digest(&mut app)?;
    if at_three == at_one {
        return Err("decoded README source frames are identical".into());
    }
    let editor = readme_editor(&mut app)?;
    let project = editor.session.project();
    let asset = project
        .data()
        .assets
        .first()
        .ok_or("README project lost its asset")?;
    let probe = serde_json::to_value(asset.probe())?;
    let source_relative = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "excerpt.mp4".into());
    let outputs = root.join("docs/assets/readme");
    fs::create_dir_all(&outputs)?;
    let stage = run.join("stage");
    fs::create_dir_all(&stage)?;
    let mut manifest = serde_json::json!({
        "slot": slot,
        "command": format!("cargo xtask build -- cargo xtask ui-audit --readme {slot} --source <local Big Buck Bunny excerpt>"),
        "source_commit": commit,
        "source": {
            "kind": "Big Buck Bunny (2008, Blender Foundation)",
            "credit": BBB_CREDIT,
            "source_page": "https://peach.blender.org/download/",
            "license": "CC-BY-3.0",
            "license_url": "https://creativecommons.org/licenses/by/3.0/",
            "license_file": "docs/licenses/CC-BY-3.0.txt",
            "original": {
                "url": BBB_SOURCE_URL,
                "bytes": BBB_SOURCE_BYTES,
                "sha256": BBB_SOURCE_SHA256
            },
            "path": source_relative,
            "sha256": source_hash,
            "probe": probe,
            "excerpt": {
                "start_seconds": 50.0,
                "duration_seconds": 12.0,
                "transform": "fps=30,scale=1280:720:flags=lanczos,setsar=1,libx264 medium crf20 yuv420p gop30 BT.709 TV,movflags faststart,map_metadata -1,no data/subtitles; AAC 160k 48k stereo; original audio retained"
            }
        },
        "tools": tools,
        "decoded_frame_sha256": { "at_1s": at_one, "at_3s": at_three }
    });
    if slot == "hero" {
        readme_seek(&mut app, Tick(3 * SECOND))?;
        let raw = run.join("hero-frame.png");
        readme_capture(&mut app, &raw)?;
        let jpg = stage.join("hero.jpg");
        let jpg_report = encode_jpg(&raw, &jpg)?;
        let details = readme_boundary(&mut app, run, 0, "settled multitrack project")?;
        manifest["boundary"] = details;
        manifest["outputs"] = serde_json::json!({"hero.jpg": jpg_report});
    } else {
        let boundaries = vec![readme_boundary(
            &mut app,
            run,
            0,
            "initial multitrack project",
        )?];
        let mut commands = Vec::new();
        let split = {
            let editor = readme_editor(&mut app)?;
            let command = Command::SplitAt {
                item: first,
                at: Tick(4 * SECOND),
            };
            let command_json = serde_json::to_value(&command)?;
            let outcome = editor.session.apply(command)?;
            commands.push(serde_json::json!({
                "boundary": 1,
                "command": command_json,
                "changed": outcome.changed,
                "created": serde_json::to_value(&outcome.created)?,
                "placements": serde_json::to_value(&outcome.placements)?
            }));
            outcome
                .created
                .into_iter()
                .find_map(|created| match created {
                    CreatedId::Item(id) => Some(id),
                    _ => None,
                })
                .ok_or("split created no right item")?
        };
        let other_ranges: Vec<_> = {
            let editor = readme_editor(&mut app)?;
            let first_item = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == first)
                .ok_or("split lost the first item")?;
            if first_item.source_range.start() != Tick::ZERO
                || first_item.source_range.end() != Tick(4 * SECOND)
            {
                return Err("split first source range is not 0..4s".into());
            }
            let right = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == split)
                .ok_or("split right item missing")?;
            if right.start != Tick(4 * SECOND)
                || right.source_range.start() != Tick(4 * SECOND)
                || right.source_range.end() != Tick(6 * SECOND)
            {
                return Err("split right source range is not 4..6s".into());
            }
            editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| track.items.iter())
                .filter(|item| item.id != first && item.id != split)
                .map(|item| (item.id, item.source_range, item.start))
                .collect()
        };
        let boundaries = {
            let mut all = boundaries;
            all.push(readme_boundary(&mut app, run, 1, "split at 4s")?);
            all
        };
        let boundaries = {
            let editor = readme_editor(&mut app)?;
            let command = Command::TimelineTrim {
                items: vec![split],
                edge: TrimEdge::End,
                delta: Tick(-SECOND / 2),
            };
            let command_json = serde_json::to_value(&command)?;
            let outcome = editor.session.apply(command)?;
            commands.push(serde_json::json!({
                "boundary": 2,
                "command": command_json,
                "changed": outcome.changed,
                "created": serde_json::to_value(&outcome.created)?,
                "placements": serde_json::to_value(&outcome.placements)?
            }));
            let right = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == split)
                .ok_or("trim lost the right item")?;
            if right.source_range.end() != Tick(5 * SECOND + SECOND / 2) {
                return Err("trim did not remove 0.5s from the right end".into());
            }
            for (id, source_range, start) in &other_ranges {
                let item = editor
                    .session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| track.items.iter())
                    .find(|item| item.id == *id)
                    .ok_or("trim lost another item")?;
                if item.source_range != *source_range || item.start != *start {
                    return Err("trim changed another track/item".into());
                }
            }
            let mut all = boundaries;
            all.push(readme_boundary(
                &mut app,
                run,
                2,
                "right end trimmed by 0.5s",
            )?);
            all
        };
        let boundaries = {
            let editor = readme_editor(&mut app)?;
            let command = Command::Undo {};
            let command_json = serde_json::to_value(&command)?;
            let outcome = editor.session.apply(command)?;
            commands.push(serde_json::json!({
                "boundary": 3,
                "command": command_json,
                "changed": outcome.changed,
                "created": serde_json::to_value(&outcome.created)?,
                "placements": serde_json::to_value(&outcome.placements)?
            }));
            let first_item = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == first)
                .ok_or("undo lost the first item")?;
            let right = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == split)
                .ok_or("undo lost the split item")?;
            if first_item.source_range.end() != Tick(4 * SECOND)
                || right.source_range.start() != Tick(4 * SECOND)
                || right.source_range.end() != Tick(6 * SECOND)
            {
                return Err("undo did not restore the split state".into());
            }
            let mut all = boundaries;
            all.push(readme_boundary(
                &mut app,
                run,
                3,
                "undo restores the split",
            )?);
            all
        };
        let mut frames = Vec::with_capacity(README_GIF_FRAMES);
        for index in 0..README_GIF_FRAMES {
            let boundary = index / 20;
            let source = run.join(format!("boundary-{boundary}.png"));
            let frame = run.join(format!("frame-{index:05}.png"));
            fs::copy(&source, &frame)?;
            let mut entry = boundaries.get(boundary).cloned().unwrap_or_default();
            entry["frame"] = serde_json::json!(index);
            entry["boundary"] = serde_json::json!(boundary);
            frames.push(entry);
        }
        let gif_report = encode_gif(run, &stage.join("timeline.gif"))?;
        let jpg_report = encode_jpg(&run.join("boundary-2.png"), &stage.join("timeline.jpg"))?;
        manifest["snapshot_verification"] = verify_snapshots(run, first, split)?;
        manifest["commands"] = serde_json::json!(commands);
        manifest["boundaries"] = serde_json::json!(boundaries);
        manifest["frames"] = serde_json::json!(frames);
        manifest["outputs"] = serde_json::json!({
            "timeline.gif": gif_report,
            "timeline.jpg": jpg_report
        });
    }
    let source_note = format!(
        "source: Big Buck Bunny (2008, Blender Foundation), {BBB_CREDIT}; https://peach.blender.org/download/; 12s excerpt at 50s, excerpt SHA256 {}; captured by apps/vibecut/examples/ui_audit.rs --readme {slot} at {commit}",
        manifest["source"]["sha256"].as_str().unwrap_or("")
    );
    let notes = "notes: VibeCut original offscreen capture (GPL-3.0-only code/capture) over a CC-BY-3.0 Big Buck Bunny excerpt; authored Vietnamese/English caption is a demo edit, not recognition or translation; bundled Inter/Noto font and icon licences are recorded in docs/assets/readme/README.md; film credit retained";
    let files = if slot == "hero" {
        vec![(
            "hero.jpg",
            "VibeCut editor with a multitrack project",
            "The editor with file-backed video, audio, an authored title and bilingual caption.",
        )]
    } else {
        vec![
            (
                "timeline.gif",
                "Split, trim and undo on the VibeCut timeline",
                "An actual offscreen edit: split at 4 s, trim the right end by 0.5 s, undo.",
            ),
            (
                "timeline.jpg",
                "Split, trim and undo on the VibeCut timeline",
                "Poster frame for the timeline sequence: the right end after the 0.5 s trim.",
            ),
        ]
    };
    for (asset, title, description) in files {
        let sidecar = format!(
            "asset: {asset}\ntitle: {title}\nauthor: Blender Foundation; VibeCut contributors\n{source_note}\nlicense: CC-BY-3.0 (film excerpt); GPL-3.0-only (VibeCut capture)\nlicense-file: ../../licenses/CC-BY-3.0.txt\nadded: p0-core/22\n{notes}; capture: {description}\n"
        );
        fs::write(stage.join(format!("{asset}.attribution")), sidecar)?;
    }
    manifest["publication"] = serde_json::json!("staged");
    fs::write(
        run.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let (media, all): (&[&str], &[&str]) = if slot == "hero" {
        (&["hero.jpg"], &["hero.jpg", "hero.jpg.attribution"])
    } else {
        (
            &["timeline.gif", "timeline.jpg"],
            &[
                "timeline.gif",
                "timeline.gif.attribution",
                "timeline.jpg",
                "timeline.jpg.attribution",
            ],
        )
    };
    let run_id = run
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or("README run id")?;
    let publication = publish_outputs(&stage, &outputs, slot, media, all, run, &run_id)?;
    manifest["publication"] = serde_json::json!("published");
    let manifest = serde_json::to_vec_pretty(&manifest)?;
    finish_publication(publication, &run.join("manifest.json"), &manifest)?;
    println!("readme-media: {} {}", slot, run.display());
    Ok(())
}
mod media;
mod process;
mod project;
mod publication;
mod source;
use media::{encode_gif, encode_jpg};
#[cfg(test)]
pub(crate) use process::run_tool;
use process::{free_bytes, git_head, tool_identity};
use project::{
    readme_boundary, readme_capture, readme_editor, readme_frame_digest, readme_project,
    readme_seek, verify_snapshots,
};
pub(crate) use publication::{finish_publication, publish_outputs};
use source::{
    BBB_CREDIT, BBB_SOURCE_BYTES, BBB_SOURCE_SHA256, BBB_SOURCE_URL, README_GIF_FRAMES,
    README_SCRATCH, README_WINDOW, stream_sha256,
};
