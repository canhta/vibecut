//! Default cache discovery through the public runtime and consumer boundaries.
use ai::transport::{DownloadTransport, HttpStreamResponse};
use engine::{
    ByteBudget, EditError,
    cache::{ArtifactCache, CacheRoot},
    control::MediaAccess,
    models::ModelTransfers,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const PROBE: &str = "VIBECUT_TEST_CACHE_PROBE";
const BASE: &str = "VIBECUT_TEST_CACHE_BASE";

fn probe(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "runtime::cache_location::cache_location_child",
            "--nocapture",
        ])
        .env(PROBE, mode)
        .env_remove("HOME")
        .env_remove("LOCALAPPDATA")
        .env_remove("XDG_CACHE_HOME");
    command
}

fn select_base(command: &mut Command, root: &Path) {
    #[cfg(target_os = "windows")]
    command.env("LOCALAPPDATA", root).env(BASE, root);
    #[cfg(target_os = "macos")]
    command
        .env("HOME", root)
        .env(BASE, root.join("Library/Caches"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    command.env("XDG_CACHE_HOME", root).env(BASE, root);
}

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn discovery_preserves_native_paths_without_io() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = probe("paths");
    select_base(&mut command, &dir.path().join("Tiếng Việt cache"));
    run(&mut command);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(any(unix, windows))]
#[test]
fn discovery_preserves_non_utf8_native_environment_values() {
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"native cache \xff".to_vec())
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0x63, 0x61, 0x63, 0x68, 0x65, 0x20, 0xd800])
    };
    assert!(name.to_str().is_none());
    let dir = tempfile::tempdir().unwrap();
    let mut command = probe("paths");
    select_base(&mut command, &dir.path().join(name));
    run(&mut command);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn unavailable_location_is_runtime_unsupported_for_default_consumers() {
    let dir = tempfile::tempdir().unwrap();
    for selected in [None, Some(""), Some("relative cache")] {
        let mut command = probe("unavailable");
        command.current_dir(dir.path());
        if let Some(selected) = selected {
            select_base(&mut command, Path::new(selected));
        }
        // Unrelated absolute values must not rescue a missing/invalid selected base.
        #[cfg(target_os = "windows")]
        command
            .env("HOME", dir.path())
            .env("XDG_CACHE_HOME", dir.path());
        #[cfg(target_os = "macos")]
        command.env("XDG_CACHE_HOME", dir.path());
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        if selected.is_some() {
            command.env("HOME", dir.path());
        }
        run(&mut command);
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn default_consumers_lock_exact_existing_paths_and_preserve_injected_roots() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = probe("consumers");
    select_base(&mut command, &dir.path().join("Đường dẫn cache"));
    command.current_dir(dir.path());
    run(&mut command);
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[test]
fn xdg_precedes_home_and_only_absence_uses_home_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut xdg = probe("paths");
    select_base(&mut xdg, &dir.path().join("xdg"));
    xdg.env("HOME", dir.path().join("home"));
    run(&mut xdg);
    let mut home = probe("paths");
    home.env("HOME", dir.path().join("home"))
        .env(BASE, dir.path().join("home/.cache"));
    run(&mut home);
}

#[derive(Default)]
struct Offline(AtomicUsize);
impl DownloadTransport for Offline {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, ai::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ai::Error::Network)
    }
}

fn unsupported<T>(result: engine::Result<T>) {
    assert!(matches!(
        result,
        Err(EditError::Unsupported("OS cache location unavailable"))
    ));
}

fn locked(path: &Path, budget: &ByteBudget) {
    assert!(matches!(
        ArtifactCache::open(path, budget.clone(), 8),
        Err(EditError::CacheBusy)
    ));
}

#[test]
fn cache_location_child() {
    let Ok(mode) = std::env::var(PROBE) else {
        return;
    };
    let budget = ByteBudget::new(8 << 20).unwrap();
    let offline = Arc::new(Offline::default());
    if mode == "unavailable" {
        unsupported(CacheRoot::discover());
        unsupported(ModelTransfers::open_default(
            budget.clone(),
            8,
            offline.clone(),
        ));
        unsupported(MediaAccess::open(budget));
    } else {
        let base = PathBuf::from(std::env::var_os(BASE).unwrap());
        let location = CacheRoot::discover().unwrap();
        let models = base.join("VibeCut/models");
        assert_eq!(location.models(), models);
        assert_eq!(location.fonts(), base.join("VibeCut/fonts"));
        assert_eq!(location.waveform(), models.join("waveform"));
        assert_eq!(location.thumbnails(), models.join("thumbnails"));
        assert!(!base.exists(), "discovery performs no directory creation");
        if mode == "consumers" {
            let (transfers, diagnostics) =
                ModelTransfers::open_default(budget.clone(), 8, offline.clone()).unwrap();
            assert_eq!(diagnostics, 0);
            locked(&models, &budget);
            let media = MediaAccess::open(budget.clone()).unwrap();
            locked(&models.join("waveform"), &budget);
            locked(&models.join("thumbnails"), &budget);
            assert!(!base.join("VibeCut/waveform").exists());
            assert!(!base.join("VibeCut/thumbnails").exists());
            assert!(!location.fonts().exists());
            let explicit = MediaAccess::open_in(Path::new("explicit"), budget.clone()).unwrap();
            locked(Path::new("explicit/waveform"), &budget);
            locked(Path::new("explicit/thumbnails"), &budget);
            let (injected, _) =
                ArtifactCache::open(Path::new("injected"), budget.clone(), 8).unwrap();
            locked(Path::new("injected"), &budget);
            drop((transfers, media, explicit, injected));
        }
    }
    assert_eq!(offline.0.load(Ordering::SeqCst), 0);
}
