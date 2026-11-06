//! Verifies native fonts command behaviour.
use crate::cli::process::{bin, code, error_json, os, stdout_json};
use std::ffi::OsStr;
use std::process::Command;

#[test]
fn fonts_list_and_resolve_are_offline_and_download_requires_acceptance() {
    let home = tempfile::tempdir().unwrap();
    let font_cli = |args: &[&OsStr]| {
        Command::new(bin())
            .env("HOME", home.path())
            .env("LOCALAPPDATA", home.path())
            .env("XDG_CACHE_HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let listed = font_cli(&[
        os("fonts"),
        os("list"),
        os("--query"),
        os("lobster"),
        os("--json"),
    ]);
    assert_eq!(
        code(&listed),
        0,
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let fonts = stdout_json(&listed)["fonts"].clone();
    assert_eq!(fonts.as_array().unwrap().len(), 1);
    assert_eq!(fonts[0]["family"], "Lobster");
    assert_eq!(fonts[0]["origin"], "google");
    assert_eq!(fonts[0]["state"], "not_downloaded");
    assert_eq!(fonts[0]["license"], "OFL-1.1");
    assert!(
        fonts[0]["source"]
            .as_str()
            .unwrap()
            .starts_with("https://github.com/google/fonts/")
    );

    let bundled = font_cli(&[os("fonts"), os("resolve"), os("inter"), os("--json")]);
    assert_eq!(code(&bundled), 0);
    let value = stdout_json(&bundled);
    assert_eq!(value["state"], "loaded");
    assert_eq!(value["origin"], "bundled");

    // A cold catalog family and an unknown family are typed states with the fallback named.
    for (family, state) in [("Lobster", "not_downloaded"), ("No Such Family", "unknown")] {
        let cold = font_cli(&[os("fonts"), os("resolve"), os(family), os("--json")]);
        assert_eq!(code(&cold), 4, "{family}");
        let value = stdout_json(&cold);
        assert_eq!(value["state"], state);
        assert_eq!(value["effective_family"], "Inter");
    }

    let refused = font_cli(&[os("fonts"), os("download"), os("Lobster"), os("--json")]);
    assert_eq!(code(&refused), 5);
    assert_eq!(error_json(&refused)["error"]["code"], "rejected");
    let outside = font_cli(&[
        os("fonts"),
        os("download"),
        os("Inter"),
        os("--accept-license"),
        os("--json"),
    ]);
    assert_eq!(code(&outside), 5);
    assert_eq!(code(&font_cli(&[os("fonts"), os("nope"), os("--json")])), 2);
    assert!(
        !home.path().join("VibeCut").join("fonts").exists(),
        "refusals write nothing"
    );
}
