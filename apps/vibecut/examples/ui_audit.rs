//! Offscreen UI audit scenes for design review.
//!
//! `cargo xtask ui-audit` renders named scenes through the real [`Editor`] and
//! [`Workspace`] widget paths at 1x and 2x, plus an `index.html` that places each
//! render beside its matching `.spikes/ui-reference/` image when one exists.
//! This is an audit tool, not a test: it never runs in `cargo xtask ci`.
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session, TrackPlacement};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, MediaError, SourceAsset, SourceProbe,
    SourceStream, StreamKind, TimeBase, VideoInfo,
};
use project::{
    AudioParams, Color, ItemId, ItemParams, Project, ProjectId, TextParams, TrackId, TrackKind,
    VideoParams,
};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, Inspection,
    catalog::{Locale, Preferences},
    egui,
    import::ImportProbe,
    playback::{Files, Playback},
    workspace::Workspace,
};

const SECOND: i64 = TICKS_PER_SECOND;
const WINDOW: [f32; 2] = [1440.0, 900.0];
const SCALES: [f32; 2] = [1.0, 2.0];

type Failure = Box<dyn std::error::Error>;
type AppHarness = Harness<'static, AuditApp>;

/// Either app root: Home (no editor) or an active editor.
enum App {
    Home {
        workspace: Box<Workspace>,
        _directory: tempfile::TempDir,
    },
    Editor(Box<Editor>),
}
impl App {
    fn ui(&mut self, ui: &mut egui::Ui) {
        match self {
            Self::Home { workspace, .. } => workspace.ui(ui),
            Self::Editor(editor) => editor.ui(ui),
        }
    }
    fn busy(&self) -> bool {
        match self {
            Self::Home { workspace, .. } => workspace.busy(),
            Self::Editor(_) => false,
        }
    }
    fn inspect(&self) -> Result<Inspection, Failure> {
        match self {
            Self::Editor(editor) => Ok(editor.inspect()?),
            Self::Home { .. } => Err("the Home scene has no editor inspection".into()),
        }
    }
}

/// eframe mode hands the app the same margin-free root Ui the native shell gets;
/// kittest's plain Ui mode inserts an 8 px central-panel frame that the real
/// window does not have.
struct AuditApp {
    app: App,
}
impl AuditApp {
    fn busy(&self) -> bool {
        self.app.busy()
    }
    fn inspect(&self) -> Result<Inspection, Failure> {
        self.app.inspect()
    }
}
impl eframe::App for AuditApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Theme {
    Dark,
    Light,
}
impl Theme {
    const ALL: [Self; 2] = [Self::Dark, Self::Light];
    fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    fn dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

struct Scene {
    name: &'static str,
    reference_dark: Option<&'static str>,
    reference_light: Option<&'static str>,
    build: fn(Theme, f32) -> Result<AppHarness, Failure>,
}
impl Scene {
    fn reference(&self, theme: Theme) -> Option<&'static str> {
        match theme {
            Theme::Dark => self.reference_dark,
            Theme::Light => self.reference_light,
        }
    }
}

const SCENES: &[Scene] = &[
    Scene {
        name: "home",
        reference_dark: Some("capcut/capcut-desktop-my-projects.png"),
        reference_light: Some("capcut/capcut-desktop-my-projects.png"),
        build: home,
    },
    Scene {
        name: "editor-empty",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: editor_empty,
    },
    Scene {
        name: "editor-media",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: editor_media,
    },
    Scene {
        name: "timeline-selection",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: timeline_selection,
    },
    Scene {
        name: "import-progress",
        reference_dark: Some("capcut/illustrations/capcut-desktop-modify-project.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-modify-project.png"),
        build: import_progress,
    },
    Scene {
        name: "export-dialog",
        reference_dark: Some("capcut/illustrations/capcut-export-panel.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-export-settings.png"),
        build: export_dialog,
    },
];

fn preferences(theme: Theme) -> Preferences {
    Preferences {
        locale: Locale::En,
        dark: theme.dark(),
        ..Preferences::default()
    }
}

fn harness(app: App, scale: f32) -> AppHarness {
    Harness::builder()
        .with_size(egui::vec2(WINDOW[0], WINDOW[1]))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp { app })
}

fn home(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let workspace = Workspace::new(directory.path().join("recovery"), preferences(theme), None)?;
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    // Discovery starts in the background; Home shows New/Open once it settles.
    for _ in 0..500 {
        app.run_steps(1);
        if !app.state().busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    Ok(app)
}

fn empty_session(name: &str) -> Result<Session, Failure> {
    Ok(Session::new(
        Project::empty(ProjectId(46), name.into())?,
        50,
    )?)
}

fn app_editor(editor: Editor, scale: f32) -> AppHarness {
    harness(App::Editor(Box::new(editor)), scale)
}

fn empty_editor(theme: Theme) -> Result<Editor, Failure> {
    let session = empty_session("Untitled Project")?;
    // Media import admits a 256 MiB probe/work lease before it opens a file.
    let mut playback = Playback::new(&session, ByteBudget::new(512 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    Ok(Editor::new(session, playback, preferences(theme)))
}

fn editor_empty(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    Ok(app_editor(empty_editor(theme)?, scale))
}

fn source_stream(kind: StreamKind, duration: Tick) -> Result<SourceStream, Failure> {
    Ok(SourceStream {
        index: 0,
        kind,
        codec: match kind {
            StreamKind::Video => "h264".into(),
            StreamKind::Audio => "aac".into(),
            StreamKind::Other => "data".into(),
        },
        time_base: TimeBase::new(1, 1_000)?,
        duration: Some(duration),
        start: None,
        frame_rate: (kind == StreamKind::Video).then_some(FrameRate::FPS_30),
        frame_count: None,
        variable_frame_rate: None,
        video: (kind == StreamKind::Video).then_some(VideoInfo {
            width: 1920,
            height: 1080,
            color: ColorInfo::default(),
            bit_depth: Some(8),
        }),
        audio: (kind == StreamKind::Audio).then_some(AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            initial_padding: 0,
            trailing_padding: 0,
        }),
    })
}

fn source_asset(id: u64, kind: StreamKind, path: PathBuf) -> Result<SourceAsset, Failure> {
    let duration = Tick(10 * SECOND);
    Ok(SourceAsset::try_from(AssetData {
        id: AssetId(id),
        path,
        content_hash: ContentHash::from_bytes(&[id as u8; 8]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(duration),
            streams: vec![source_stream(kind, duration)?],
        },
    })?)
}

fn range(start: i64, end: i64) -> Result<SourceRange, Failure> {
    Ok(SourceRange::new(Tick(start * SECOND), Tick(end * SECOND))?)
}

fn track(session: &mut Session, name: &str, kind: TrackKind) -> Result<TrackId, Failure> {
    let outcome = session.apply(Command::AddTrack {
        placement: TrackPlacement::Auto,
        name: name.into(),
        kind,
    })?;
    Ok(outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            CreatedId::Track(id) => Some(id),
            _ => None,
        })
        .ok_or("created track id")?)
}

fn insert(
    session: &mut Session,
    track: TrackId,
    start: Tick,
    source_range: SourceRange,
    asset: Option<AssetId>,
    content: ItemParams,
) -> Result<ItemId, Failure> {
    let outcome = session.apply(Command::InsertItem {
        placement: Placement::Track { track },
        item: NewItem {
            animations: Default::default(),
            source_asset: asset,
            source_range,
            start,
            content,
        },
    })?;
    Ok(outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            CreatedId::Item(id) => Some(id),
            _ => None,
        })
        .ok_or("created item id")?)
}

fn media_editor(theme: Theme) -> Result<(Editor, ItemId), Failure> {
    let mut session = empty_session("Launch cut")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(id, kind, PathBuf::from(format!("/media/source-{id}.mp4")))?,
        })?;
    }
    let main = session.project().data().scene().timeline.main.id;
    let first = insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 6)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
        }),
    )?;
    insert(
        &mut session,
        main,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
        }),
    )?;
    let audio = track(&mut session, "Audio", TrackKind::Audio)?;
    insert(
        &mut session,
        audio,
        Tick::ZERO,
        range(0, 10)?,
        Some(AssetId(2)),
        ItemParams::Audio(AudioParams {
            gain_db: 0.0,
            muted: false,
        }),
    )?;
    let text = track(&mut session, "Text", TrackKind::Text)?;
    insert(
        &mut session,
        text,
        Tick(2 * SECOND),
        range(2, 5)?,
        None,
        ItemParams::Text(TextParams {
            content: "Chapter one".into(),
            font_size: 48.0,
            color: Color([255, 255, 255, 255]),
        }),
    )?;
    let mut playback = Playback::new(&session, ByteBudget::new(64 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, preferences(theme));
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    Ok((editor, first))
}

fn editor_media(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme)?;
    let mut app = app_editor(editor, scale);
    app.run_steps(2);
    Ok(app)
}

fn timeline_selection(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, item) = media_editor(theme)?;
    let mut app = app_editor(editor, scale);
    app.run_steps(3);
    let id = format!("timeline.item.{}", item.0);
    let node = app
        .query_all(By::new().predicate(|node| node.author_id() == Some(id.as_str())))
        .next()
        .ok_or_else(|| format!("missing automation id {id}"))?;
    node.click();
    app.run_steps(3);
    Ok(app)
}

/// Blocks every file after the first until the batch is cancelled, so the
/// audit samples the real progress overlay while the job is genuinely running.
struct ProgressProbe {
    calls: AtomicUsize,
    release: (Mutex<bool>, Condvar),
}
impl ProgressProbe {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            release: (Mutex::new(false), Condvar::new()),
        }
    }
}
impl ImportProbe for ProgressProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<SourceAsset> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            let mut released = match self.release.0.lock() {
                Ok(released) => released,
                Err(_) => return Err(MediaError::Cancelled),
            };
            while !*released {
                if cancelled.load(Ordering::Acquire) {
                    return Err(MediaError::Cancelled);
                }
                released = match self
                    .release
                    .1
                    .wait_timeout(released, Duration::from_millis(5))
                {
                    Ok((released, _)) => released,
                    Err(_) => return Err(MediaError::Cancelled),
                };
            }
        }
        source_asset(id.0, StreamKind::Video, path).map_err(|_| MediaError::Unsupported("fixture"))
    }
}

fn export_dialog(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme)?;
    editor.activate("file.export")?;
    editor.export_picked_file(PathBuf::from("/media/launch-cut.mp4"))?;
    let mut app = app_editor(editor, scale);
    app.run_steps(4);
    Ok(app)
}

fn import_progress(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme)?;
    editor.set_import_probe(Arc::new(ProgressProbe::new()))?;
    editor.drop_files(vec![
        PathBuf::from("/media/one.mp4"),
        PathBuf::from("/media/two.mp4"),
        PathBuf::from("/media/three.mp4"),
    ])?;
    let mut app = app_editor(editor, scale);
    for _ in 0..500 {
        app.run_steps(1);
        let inspection = app.state().inspect()?;
        if inspection.import_completed > 0 {
            break;
        }
        if !inspection.importing {
            return Err("import finished before the audit could sample progress".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    let inspection = app.state().inspect()?;
    if !inspection.importing {
        return Err("import finished before the audit could render progress".into());
    }
    Ok(app)
}

fn main() -> Result<(), Failure> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize()?;
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("target"),
    };
    let output = target.join("ui-audit");
    std::fs::create_dir_all(&output)?;
    let references = reference_root(&root);
    let selected = select(std::env::args().skip(1))?;
    let mut rows = Vec::new();
    for (scene, theme) in &selected {
        let variant = format!("{}-{}", scene.name, theme.key());
        let mut images = Vec::new();
        for scale in SCALES {
            let mut app = (scene.build)(*theme, scale)?;
            app.run_steps(2);
            let image = app.render().map_err(std::io::Error::other)?;
            let image = opaque(image, vibecut::window_color(theme.dark()));
            let file = format!("{variant}@{}x.png", scale as u32);
            image.save(output.join(&file))?;
            images.push((file, scale));
        }
        let reference = scene.reference(*theme).and_then(|relative| {
            copy_reference(references.as_deref(), &output, &variant, relative)
        });
        rows.push(Row {
            variant,
            theme: theme.key(),
            images,
            reference,
        });
        eprintln!("ui-audit: {} rendered", scene.name);
    }
    let index = output.join("index.html");
    std::fs::write(&index, index_html(&rows))?;
    println!("ui-audit: {}", index.display());
    Ok(())
}

fn select(names: impl Iterator<Item = String>) -> Result<Vec<(&'static Scene, Theme)>, Failure> {
    let names: Vec<String> = names.collect();
    if names.is_empty() {
        return Ok(SCENES
            .iter()
            .flat_map(|scene| Theme::ALL.map(|theme| (scene, theme)))
            .collect());
    }
    let mut selected = BTreeSet::new();
    let mut unknown = Vec::new();
    for name in names {
        let mut matched = false;
        for (index, scene) in SCENES.iter().enumerate() {
            if name == scene.name {
                for (theme_index, _theme) in Theme::ALL.into_iter().enumerate() {
                    selected.insert((index, theme_index));
                }
                matched = true;
            } else {
                for (theme_index, theme) in Theme::ALL.into_iter().enumerate() {
                    if name == format!("{}-{}", scene.name, theme.key()) {
                        selected.insert((index, theme_index));
                        matched = true;
                    }
                }
            }
        }
        if !matched {
            unknown.push(name);
        }
    }
    if !unknown.is_empty() {
        let known: Vec<_> = SCENES
            .iter()
            .flat_map(|scene| {
                Theme::ALL
                    .into_iter()
                    .map(move |theme| format!("{}-{}", scene.name, theme.key()))
            })
            .collect();
        return Err(format!(
            "unknown scene(s): {}; known scenes: {}",
            unknown.join(", "),
            known.join(", ")
        )
        .into());
    }
    selected
        .into_iter()
        .map(|(index, theme_index)| {
            let scene = SCENES.get(index).ok_or("scene index")?;
            let theme = Theme::ALL
                .into_iter()
                .nth(theme_index)
                .ok_or("theme index")?;
            Ok((scene, theme))
        })
        .collect()
}

fn reference_root(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VIBECUT_UI_REFERENCE") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    let local = root.join(".spikes/ui-reference");
    if local.is_dir() {
        return Some(local);
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .map(|worktree| worktree.join(".spikes/ui-reference"))
        .filter(|path| path.is_dir())
}

fn copy_reference(
    root: Option<&Path>,
    output: &Path,
    variant: &str,
    relative: &str,
) -> Option<String> {
    let source = root?.join(relative);
    let extension = source.extension()?.to_str()?;
    let name = format!("{variant}.{extension}");
    let directory = output.join("reference");
    std::fs::create_dir_all(&directory).ok()?;
    std::fs::copy(&source, directory.join(&name)).ok()?;
    Some(format!("reference/{name}"))
}

/// egui renders premultiplied alpha over a transparent clear; the audit PNG is
/// opaque so it can be compared and reviewed outside a compositor.
fn opaque(image: image::RgbaImage, background: egui::Color32) -> image::RgbaImage {
    let [red, green, blue, _] = background.to_array();
    let mut output = image::RgbaImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let [source_red, source_green, source_blue, source_alpha] = pixel.0;
        let inverse = 255 - u32::from(source_alpha);
        let blend = |source: u8, destination: u8| {
            (u32::from(source) + u32::from(destination) * inverse / 255).min(255) as u8
        };
        output.put_pixel(
            x,
            y,
            image::Rgba([
                blend(source_red, red),
                blend(source_green, green),
                blend(source_blue, blue),
                255,
            ]),
        );
    }
    output
}

struct Row {
    variant: String,
    theme: &'static str,
    images: Vec<(String, f32)>,
    reference: Option<String>,
}

fn index_html(rows: &[Row]) -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>VibeCut UI audit</title>\n<style>\n\
         body{font:14px/1.4 system-ui,sans-serif;margin:24px;background:#0f1113;color:#fff}\n\
         h1{font-size:24px} h2{font-size:18px;margin-top:32px}\n\
         table{border-collapse:collapse;width:100%}\n\
         th,td{text-align:left;vertical-align:top;padding:8px;border-top:1px solid #26272a}\n\
         img{max-width:100%;display:block;border:1px solid #26272a}\n\
         .missing{color:#898a8b;font-style:italic}\n\
         </style>\n</head>\n<body>\n<h1>VibeCut UI audit</h1>\n\
         <p>Offscreen renders at 1x and 2x. Reference images are copied from \
         <code>.spikes/ui-reference/</code> when present.</p>\n",
    );
    let mut current = "";
    for Row {
        variant,
        theme,
        images,
        reference,
    } in rows
    {
        let scene = variant
            .strip_suffix("-dark")
            .or_else(|| variant.strip_suffix("-light"))
            .unwrap_or(variant);
        if scene != current {
            if !current.is_empty() {
                let _ = writeln!(html, "</table>");
            }
            let _ = writeln!(html, "<h2>{scene}</h2>\n<table>");
            let _ = writeln!(
                html,
                "<tr><th>theme</th><th>1x</th><th>2x</th><th>reference</th></tr>"
            );
            current = scene;
        }
        let _ = writeln!(html, "<tr><th>{theme}</th>");
        for (file, _) in images {
            let _ = writeln!(
                html,
                "<td><a href=\"{file}\"><img src=\"{file}\" alt=\"{variant}\"></a></td>"
            );
        }
        match reference {
            Some(reference) => {
                let _ = writeln!(
                    html,
                    "<td><img src=\"{reference}\" alt=\"reference for {variant}\"></td></tr>"
                );
            }
            None => {
                let _ = writeln!(
                    html,
                    "<td class=\"missing\">no matching reference found</td></tr>"
                );
            }
        }
    }
    if !rows.is_empty() {
        html.push_str("</table>\n");
    }
    html.push_str("</body>\n</html>\n");
    html
}
