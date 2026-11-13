//! Offscreen UI audit scenes for design review.
//!
//! `cargo xtask ui-audit` renders named scenes through the real
//! [`vibecut::Editor`] and [`vibecut::app::Workspace`] widget paths at 1x
//! and 2x, plus an `index.html` that places each render beside its matching
//! reference image from the local reference set when one exists.
//! This is an audit tool, not a test: it never runs in `cargo xtask ci`.
mod audit;

use audit::Failure;

fn main() -> Result<(), Failure> {
    audit::run(std::env::args().skip(1).collect())
}

#[cfg(test)]
mod tests {
    use super::audit::{
        App, Theme, finish_publication, inspector_mixed_vi, publish_outputs, readme_media, run_tool,
    };
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};
    use vibecut::egui;

    /// Interactive nodes known to lack a label or an automation id. It only shrinks.
    const UNADDRESSED: &[&str] = &[];

    #[test]
    #[ignore = "run once after the combined build to fill UNADDRESSED or fix the nodes, then drop this attribute"]
    fn every_interactive_node_has_a_label_and_an_automation_id() -> Result<(), Failure> {
        let found = super::audit::unaddressed_nodes()?;
        let new: Vec<_> = found
            .iter()
            .filter(|node| !UNADDRESSED.contains(&node.as_str()))
            .collect();
        assert!(
            new.is_empty(),
            "{} interactive nodes lack a label or an automation id:
{new:#?}",
            new.len()
        );
        let fixed: Vec<_> = UNADDRESSED
            .iter()
            .filter(|node| !found.contains(**node))
            .collect();
        assert!(
            fixed.is_empty(),
            "remove from UNADDRESSED:
{fixed:#?}"
        );
        Ok(())
    }

    #[test]
    fn readme_capture_refuses_wrong_source_before_staging_and_keeps_history() -> Result<(), Failure>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let media = root.join("docs/assets/readme");
        fs::create_dir_all(&media)?;
        fs::write(media.join("hero.jpg"), b"keep")?;
        let old_run = root.join("target/readme-media/hero/old-run");
        fs::create_dir_all(&old_run)?;
        fs::write(old_run.join("manifest.json"), b"history")?;
        let source = root.join("wrong.mp4");
        fs::write(&source, b"not the admitted excerpt")?;
        let error = readme_media(root, &root.join("target"), "hero", &source).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not the admitted Big Buck Bunny")
        );
        assert_eq!(fs::read(media.join("hero.jpg"))?, b"keep");
        assert_eq!(fs::read(old_run.join("manifest.json"))?, b"history");
        assert_eq!(
            fs::read_dir(root.join("target/readme-media/hero"))
                .map_err(|error| error.to_string())?
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn publish_rolls_back_after_a_real_file_failure() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        let owned = "asset: hero.jpg\ntitle: old\nauthor: VibeCut contributors\nsource: apps/vibecut/examples/ui_audit.rs --readme hero at old\nlicense: CC-BY-3.0 (film excerpt); GPL-3.0-only (VibeCut capture)\nlicense-file: ../../licenses/CC-BY-3.0.txt\nadded: p0-core/22\nnotes: old\n";
        fs::write(finals.join("hero.jpg"), b"old-media")?;
        fs::write(finals.join("hero.jpg.attribution"), owned)?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), owned)?;
        let error = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg", "hero.jpg.attribution", "timeline.jpg"],
            &run,
            "run1",
        )
        .unwrap_err();
        assert!(error.to_string().contains("staged README output"));
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"old-media");
        assert_eq!(
            fs::read_to_string(finals.join("hero.jpg.attribution"))?,
            owned
        );
        assert!(!finals.join(".readme-media-backup-run1").exists());
        assert!(!run.join("preserve-staging").exists());
        Ok(())
    }

    #[test]
    fn publish_refuses_an_unrelated_existing_final() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        fs::write(finals.join("hero.jpg"), b"unrelated")?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), b"asset: hero.jpg\n")?;
        let error = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg"],
            &run,
            "run2",
        )
        .unwrap_err();
        assert!(error.to_string().contains("refusing unrelated"));
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"unrelated");
        Ok(())
    }

    #[test]
    fn publish_refuses_readable_but_incomplete_ownership_sidecar() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        fs::write(finals.join("hero.jpg"), b"unrelated")?;
        fs::write(finals.join("hero.jpg.attribution"), b"asset: hero.jpg\n")?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), b"asset: hero.jpg\n")?;
        let error = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg", "hero.jpg.attribution"],
            &run,
            "run3",
        )
        .unwrap_err();
        assert!(error.to_string().contains("refusing unrelated"));
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"unrelated");
        Ok(())
    }

    #[test]
    fn publication_manifest_failure_rolls_back_finals() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        let owned = "asset: hero.jpg\ntitle: old\nauthor: VibeCut contributors\nsource: apps/vibecut/examples/ui_audit.rs --readme hero at old\nlicense: CC-BY-3.0 (film excerpt); GPL-3.0-only (VibeCut capture)\nlicense-file: ../../licenses/CC-BY-3.0.txt\nadded: p0-core/22\nnotes: old\n";
        fs::write(finals.join("hero.jpg"), b"old-media")?;
        fs::write(finals.join("hero.jpg.attribution"), owned)?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), owned)?;
        let publication = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg", "hero.jpg.attribution"],
            &run,
            "run4",
        )?;
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"new-media");
        let manifest_directory = root.join("manifest-dir");
        fs::create_dir_all(&manifest_directory)?;
        let error = finish_publication(publication, &manifest_directory, b"manifest").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("publication manifest write failed")
        );
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"old-media");
        assert_eq!(
            fs::read_to_string(finals.join("hero.jpg.attribution"))?,
            owned
        );
        assert!(!finals.join(".readme-media-backup-run4").exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn supervised_runner_does_not_wait_for_inherited_output_handles() {
        let start = Instant::now();
        let result = run_tool(
            "cmd",
            &[
                "/c",
                "start",
                "/b",
                "powershell",
                "-NoProfile",
                "-Command",
                "Start-Sleep -Seconds 5",
            ],
            Duration::from_secs(2),
            1024,
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        if let Err(error) = result {
            assert!(!error.to_string().contains("timed out"), "{error}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn supervised_runner_times_out_and_reaps_a_stalled_process() {
        let start = Instant::now();
        let error = run_tool(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 5",
            ],
            Duration::from_millis(150),
            1024,
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[cfg(windows)]
    #[test]
    fn supervised_runner_refuses_bounded_output_overflow() {
        let error = run_tool(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.Write('x' * 10000)",
            ],
            Duration::from_secs(5),
            100,
        )
        .unwrap_err();
        assert!(error.to_string().contains("bounded cap"), "{error}");
    }

    #[test]
    fn readme_capture_refuses_unknown_slots_before_touching_final_or_staging() -> Result<(), Failure>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let media = root.join("docs/assets/readme");
        fs::create_dir_all(&media)?;
        fs::write(media.join("hero.jpg"), b"keep")?;
        let error = readme_media(
            root,
            &root.join("target"),
            "unknown",
            &media.join("hero.jpg"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown README slot"));
        assert_eq!(fs::read(media.join("hero.jpg"))?, b"keep");
        assert!(!root.join("target/readme-media").exists());
        Ok(())
    }

    #[test]
    fn mixed_selection_preview_matches_its_edited_timeline() -> Result<(), Failure> {
        let app = inspector_mixed_vi(Theme::Light, 1.0)?;
        let App::Editor(editor) = &app.state().app else {
            return Err("expected editor scene".into());
        };
        let frame = editor.playback.frame().ok_or("missing preview frame")?;
        // This corner lies outside the text. It must show the scene's solid
        // overlay, rather than the synthetic video deleted during setup.
        let corner = frame.image.pixels.first().ok_or("empty preview frame")?;
        assert_eq!(*corner, egui::Color32::from_rgb(35, 45, 65));
        Ok(())
    }
}
