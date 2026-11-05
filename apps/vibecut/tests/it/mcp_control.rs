#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read as _, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(windows)]
fn foreground() -> String {
    use std::os::windows::process::CommandExt;
    let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type 'using System; using System.Runtime.InteropServices; public class Focus { [DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); }'; [Focus]::GetForegroundWindow().ToInt64()"])
        .creation_flags(0x08000000).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn mcp_real_app_background_attach_never_creates_a_window_or_takes_focus() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project.json");
    let snapshot = project::Project::empty(project::ProjectId(42), "background".into()).unwrap();
    std::fs::write(&project, project::encode(&snapshot, true).unwrap()).unwrap();
    let endpoint = if cfg!(windows) {
        std::path::PathBuf::from(format!("vibecut-background-{}", std::process::id()))
    } else {
        root.path().join("private/control.sock")
    };
    #[cfg(windows)]
    let focus = foreground();
    let mut launch = Command::new(env!("CARGO_BIN_EXE_vibecut"));
    launch
        .arg("--control-background")
        .arg(&project)
        .arg(&endpoint)
        .arg(root.path().join("recovery"))
        .arg("20")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        launch.creation_flags(0x08000000);
    }
    let mut app = Process(launch.spawn().unwrap());
    let cli = std::path::Path::new(env!("CARGO_BIN_EXE_vibecut"))
        .with_file_name(format!("vibecut-cli{}", std::env::consts::EXE_SUFFIX));
    assert!(
        cli.exists(),
        "run cargo test -p vibecut-cli -p vibecut mcp to build both binaries"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = app.0.try_wait().unwrap() {
            let mut diagnostics = String::new();
            if let Some(mut stderr) = app.0.stderr.take() {
                stderr.read_to_string(&mut diagnostics).unwrap();
            }
            panic!("background app exited early: {status}: {diagnostics}");
        }
        let mut child = Process(
            Command::new(&cli)
                .args(["mcp", "--attach", "--socket"])
                .arg(&endpoint)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut input = child.0.stdin.take().unwrap();
        if writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"app-test","version":"1"}}})).is_ok() {
            let output = child.0.stdout.take().unwrap();
            let mut reader = BufReader::new(output); let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) > 0 {
                let initialized: Value = serde_json::from_str(&line).unwrap(); assert!(initialized.get("result").is_some());
                writeln!(input, "{}", json!({"jsonrpc":"2.0","method":"notifications/initialized"})).unwrap();
                writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inspect","arguments":{"request_id":"inspect","command":"inspect","params":{}}}})).unwrap();
                line.clear(); reader.read_line(&mut line).unwrap();
                let response: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(response.pointer("/result/structuredContent/value/status"), Some(&json!("result")), "{response}");
                #[cfg(windows)] assert_eq!(foreground(), focus, "background attach stole focus");
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "background app socket did not become ready"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
