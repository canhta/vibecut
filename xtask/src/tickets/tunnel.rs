//! Optional Cloudflare publication for the read-only project board.
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Child, Command};

pub(super) struct Tunnel(Child);

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start only when this checkout has an explicit local tunnel configuration.
pub(super) fn start(root: &Path, port: u16) -> Result<Option<Tunnel>, String> {
    let private = root.join(".privates");
    let config = match fs::read_to_string(private.join("board-tunnel.json")) {
        Ok(config) => config,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read board tunnel configuration: {error}")),
    };
    let config: Value = serde_json::from_str(&config)
        .map_err(|error| format!("invalid board tunnel configuration: {error}"))?;
    let field = |key: &str| {
        config
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("board tunnel configuration requires {key}"))
    };
    let executable = field("executable")?;
    let id = field("tunnel")?;
    let hostname = field("hostname")?;
    let credentials = field("credentials")?;
    if !Path::new(credentials).is_file() {
        return Err("board tunnel credentials file is missing".into());
    }
    // JSON quoted strings are also YAML scalars, including Windows paths.
    let ingress = format!(
        "tunnel: {}\ncredentials-file: {}\ningress:\n  - hostname: {}\n    service: http://127.0.0.1:{port}\n  - service: http_status:404\n",
        serde_json::json!(id),
        serde_json::json!(credentials),
        serde_json::json!(hostname),
    );
    let ingress_path = private.join("board-cloudflared.yml");
    fs::write(&ingress_path, ingress)
        .map_err(|error| format!("write board tunnel ingress: {error}"))?;
    // Windows otherwise leaves the tunnel alive after a forced launcher exit.
    #[cfg(windows)]
    {
        use win_custody::{Job, JobLimits};
        Job::create(&JobLimits::new().kill_on_close(true))
            .and_then(|job| job.assign_current_process())
            .map_err(|error| format!("contain board tunnel process: {error}"))?;
    }
    let child = Command::new(executable)
        .args(["tunnel", "--config"])
        .arg(&ingress_path)
        .args(["--no-autoupdate", "run", id])
        .spawn()
        .map_err(|error| format!("start cloudflared for board: {error}"))?;
    println!("Public board: https://{hostname}/ (connecting through Cloudflare)");
    Ok(Some(Tunnel(child)))
}
