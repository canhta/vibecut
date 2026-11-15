//! What a start offers once on Home: a newer release and the last run's crash report.
#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use transport::{Error, HttpRequest, HttpResponse, HttpTransport};
use vibecut::{app::Workspace, egui, preferences::Preferences};

const DAY: u64 = 24 * 60 * 60;
const LATEST: &str = "https://api.github.com/repos/canhta/vibecut/releases/latest";

fn by(id: &'static str) -> By<'static> {
    By::new().predicate(move |node| node.author_id() == Some(id))
}

fn present(harness: &Harness<'static, Workspace>, id: &'static str) -> bool {
    harness.query_all(by(id)).count() > 0
}

fn build(directory: &Path, mut preferences: Preferences) -> Harness<'static, Workspace> {
    preferences.welcome_seen = true;
    let app = Workspace::with_projects(
        directory.join("recovery"),
        directory.join("projects"),
        preferences,
        None,
    )
    .unwrap();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, app: &mut Workspace| app.ui(ui), app);
    harness.run_steps(3);
    harness
}

/// What the check sent: method, URL, headers and body length.
type Sent = (&'static str, String, Vec<(String, String)>, usize);

/// The public repository as the check sees it: one canned reply, every request recorded.
struct Repository {
    reply: Result<(u16, String), ()>,
    calls: AtomicUsize,
    request: Mutex<Option<Sent>>,
}

impl HttpTransport for Repository {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.request.lock().unwrap() = Some((
            request.method,
            request.url,
            request.headers,
            request.body.len(),
        ));
        let (status, body) = self.reply.clone().map_err(|()| Error::Network)?;
        Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: body.into_bytes(),
        })
    }
}

/// Every URL the next few frames asked the platform to open.
fn opened(harness: &mut Harness<'static, Workspace>) -> Vec<String> {
    let mut urls = Vec::new();
    for _ in 0..3 {
        harness.step();
        urls.extend(
            harness
                .output()
                .platform_output
                .commands
                .iter()
                .filter_map(|command| match command {
                    egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                    _ => None,
                }),
        );
    }
    urls
}

/// Fails if the check asks more than once a day, asks while turned off, sends anything but a
/// bare GET with the generic User-Agent, shows a notice for an equal or failed reply, or the
/// notice's link misses the release page.
#[test]
fn update_check_asks_once_a_day_and_offers_only_a_newer_release() {
    let now = SystemTime::now();
    let seconds = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let tag = |tag: &str| Ok((200, format!(r#"{{"tag_name":"{tag}"}}"#)));
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    for (case, enabled, last, reply, asked, offered) in [
        ("turned off", false, None, tag("v999.0.0"), 0, false),
        (
            "checked within the day",
            true,
            Some(seconds - 3600),
            tag("v999.0.0"),
            0,
            false,
        ),
        ("offline", true, None, Err(()), 1, false),
        (
            "same version a day later",
            true,
            Some(seconds - DAY),
            tag(&current),
            1,
            false,
        ),
        ("newer", true, None, tag("v999.0.0"), 1, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let woken = crate::woken::Woken::bind();
        let mut harness = build(
            directory.path(),
            Preferences {
                update_check: enabled,
                update_checked_at: last,
                ..Preferences::default()
            },
        );
        woken.attach(&harness.ctx);
        let repository = Arc::new(Repository {
            reply,
            calls: AtomicUsize::new(0),
            request: Mutex::new(None),
        });
        // A second start in the same run is within the day of the first.
        for _ in 0..2 {
            harness
                .state_mut()
                .check_for_update(repository.clone(), now);
        }
        woken.step_until(&mut harness, case, |app| !app.checking_for_update());
        harness.run_steps(2);
        assert_eq!(repository.calls.load(Ordering::SeqCst), asked, "{case}");
        let recorded = harness.state().preferences.update_checked_at;
        assert_eq!(
            recorded,
            if asked == 0 { last } else { Some(seconds) },
            "{case}"
        );
        assert_eq!(present(&harness, "update.open"), offered, "{case}");
        if asked == 1 {
            assert_eq!(
                repository.request.lock().unwrap().clone(),
                Some((
                    "GET",
                    LATEST.to_owned(),
                    vec![(
                        "User-Agent".to_owned(),
                        format!("VibeCut/{}", env!("CARGO_PKG_VERSION"))
                    )],
                    0
                )),
                "{case}"
            );
        }
        if offered {
            assert!(
                harness
                    .query_by_label("VibeCut v999.0.0 is available.")
                    .is_some()
            );
            harness.get(by("update.open")).click();
            assert_eq!(
                opened(&mut harness),
                ["https://github.com/canhta/vibecut/releases/tag/v999.0.0"]
            );
            assert!(!present(&harness, "update.open"));
        }
    }
}

/// Fails if a report the last run left shows no notice, Open folder hands the shell anything
/// but the crash folder, or the next start offers the same report again.
#[test]
fn a_crash_report_from_the_last_run_offers_its_folder_once() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("crashes");
    vibecut::app::save_crash_report(&folder, "report").unwrap();
    let mut harness = build(directory.path(), Preferences::default());
    assert!(
        harness
            .query_by_label("VibeCut closed unexpectedly last time.")
            .is_some()
    );
    assert_eq!(harness.state_mut().take_folder_request(), None);
    harness.get(by("crash.open_folder")).click();
    harness.run_steps(2);
    assert_eq!(harness.state_mut().take_folder_request(), Some(folder));
    assert!(!present(&harness, "crash.open_folder"));
    drop(harness);

    let harness = build(directory.path(), Preferences::default());
    assert!(!present(&harness, "crash.open_folder"));
}
