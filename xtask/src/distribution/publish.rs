#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::BTreeSet;

fn api(
    tools: &mut Tools,
    method: &str,
    path: &str,
    data: Option<&Value>,
    optional: bool,
) -> Result<Option<Value>> {
    let repo = tools.env.get("GH_REPO").cloned().ok_or("GH_REPO missing")?;
    let mut command = Command::new("gh");
    command
        .args(["api", "--include", "--method", method])
        .arg(format!("repos/{repo}/{path}"));
    let temp = Temp::new(&tools.runner_temp)?;
    if let Some(data) = data {
        let file = temp.0.join("request.json");
        write_json(&file, data)?;
        command.arg("--input").arg(file);
    }
    let result = tools.execute(&mut command, Duration::from_secs(90))?;
    let status = result
        .stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1));
    if result.code != Some(0) {
        if optional && status == Some("404") {
            return Ok(None);
        }
        return Err(format!(
            "GitHub {method} {path} failed; response suppressed"
        ));
    }
    let body = result
        .stdout
        .split_once("\r\n\r\n")
        .or_else(|| result.stdout.split_once("\n\n"))
        .map(|(_, body)| body)
        .ok_or("missing GitHub response headers")?;
    if body.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_str(body).map_err(|_| "invalid GitHub JSON response")?,
    ))
}
fn required(tools: &mut Tools, path: &str) -> Result<Value> {
    api(tools, "GET", path, None, false)?.ok_or("missing GitHub response".into())
}
fn files(tools: &Tools, directory: &Path, release: bool) -> Result<Vec<PathBuf>> {
    let suffix = if release { "dmg" } else { "pkg" };
    let required: BTreeSet<_> = ["aarch64", "x86_64", "universal"]
        .iter()
        .map(|arch| format!("vibecut-{}-macos-{arch}.{suffix}", tools.version()))
        .collect();
    let mut allowed = required.clone();
    if !release {
        for arch in ["aarch64", "x86_64"] {
            allowed.insert(format!("vibecut-{}-windows-{arch}.zip", tools.version()));
        }
    }
    let mut packages = Vec::new();
    for path in entries(directory)? {
        if !matches!(
            path.extension().and_then(OsStr::to_str),
            Some("pkg" | "zip" | "dmg")
        ) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or("asset filename UTF-8")?;
        if !allowed.contains(name)
            || path.is_symlink()
            || fs::metadata(&path).map_err(|e| e.to_string())?.len() == 0
        {
            return Err(format!("invalid distribution asset: {name}"));
        }
        packages.push(path);
    }
    let actual: BTreeSet<_> = packages
        .iter()
        .filter_map(|p| p.file_name().and_then(OsStr::to_str))
        .collect();
    if !required.iter().all(|name| actual.contains(name.as_str())) {
        return Err("all three macOS distributions must pass before publication".into());
    }
    let sums = directory.join("SHA256SUMS");
    let mut lines = String::new();
    for path in &packages {
        lines.push_str(&format!(
            "{}  {}\n",
            digest(path)?,
            path.file_name().ok_or("asset name")?.to_string_lossy()
        ));
    }
    fs::write(&sums, lines).map_err(|e| e.to_string())?;
    packages.push(sums);
    Ok(packages)
}
fn publish(
    tools: &mut Tools,
    directory: &Path,
    release: bool,
    version: Option<&str>,
) -> Result<()> {
    if release && version != Some(tools.version()) {
        return Err("release version must match workspace version".into());
    }
    let assets = files(tools, directory, release)?;
    let sha = tools
        .env
        .get("GITHUB_SHA")
        .cloned()
        .ok_or("GITHUB_SHA missing")?;
    let repo = tools.env.get("GH_REPO").cloned().ok_or("GH_REPO missing")?;
    let tag = if release {
        format!("v{}", tools.version())
    } else {
        "nightly".into()
    };
    let mut existing = api(tools, "GET", &format!("releases/tags/{tag}"), None, true)?;
    let reference = api(tools, "GET", &format!("git/ref/tags/{tag}"), None, true)?;
    if let Some(existing) = &existing
        && (field(existing, "/immutable") == true
            || (!release && field(existing, "/prerelease") != true)
            || (release && field(existing, "/draft") != true))
    {
        return Err("refusing to modify a stable, published or immutable release".into());
    }
    let notes = if release {
        if reference
            .as_ref()
            .is_some_and(|value| field(value, "/object/sha").as_str() != Some(sha.as_str()))
        {
            return Err("existing version tag differs from verified commit".into());
        }
        "Signed and notarized macOS arm64, x86_64 and universal DMGs; clean-runner app/CLI smoke passed on both architectures.\n".to_owned()
    } else {
        if field(&required(tools, "git/ref/heads/main")?, "/object/sha").as_str()
            != Some(sha.as_str())
        {
            println!("superseded build; leaving nightly unchanged");
            return Ok(());
        }
        let mut commits = Vec::new();
        let previous = reference
            .as_ref()
            .and_then(|value| field(value, "/object/sha").as_str());
        if let Some(previous) = previous {
            if reference
                .as_ref()
                .is_some_and(|value| field(value, "/object/type") != "commit")
            {
                return Err("nightly must be a lightweight commit tag".into());
            }
            let comparison = required(
                tools,
                &format!("compare/{previous}...{sha}?per_page=100&page=1"),
            )?;
            if !matches!(
                field(&comparison, "/status").as_str(),
                Some("ahead" | "identical")
            ) {
                return Err("nightly history diverged; maintainer reconciliation required".into());
            }
            commits.extend(
                field(&comparison, "/commits")
                    .as_array()
                    .ok_or("missing comparison commits")?
                    .iter()
                    .cloned(),
            );
            let total = field(&comparison, "/total_commits")
                .as_u64()
                .unwrap_or(commits.len() as u64);
            if total > 20_000 {
                return Err("nightly commit list exceeds admission limit".into());
            }
            let mut page = 2;
            while (commits.len() as u64) < total {
                let response = required(
                    tools,
                    &format!("compare/{previous}...{sha}?per_page=100&page={page}"),
                )?;
                let batch = field(&response, "/commits")
                    .as_array()
                    .filter(|batch| !batch.is_empty())
                    .ok_or("incomplete nightly commit list")?;
                commits.extend(batch.iter().cloned());
                page += 1;
            }
        } else {
            commits.push(required(tools, &format!("commits/{sha}"))?);
        }
        let mut notes = format!("Unsigned test builds of {sha}.\n\n");
        for commit in commits {
            let id = field(&commit, "/sha").as_str().ok_or("commit SHA")?;
            let message = field(&commit, "/commit/message")
                .as_str()
                .ok_or("commit message")?
                .lines()
                .next()
                .unwrap_or_default();
            notes.push_str(&format!("- {} {message}\n", id.get(..7).unwrap_or(id)));
        }
        if let Some(previous) = previous {
            notes.push_str(&format!(
                "\nFull commit list: https://github.com/{repo}/compare/{previous}...{sha}\n"
            ));
        }
        notes.push_str(
            "\nWindows targets are advisory until p0-core/44. These builds are not notarized.\n",
        );
        if notes.len() > 120_000 {
            return Err("nightly commit notes exceed the release-body limit".into());
        }
        notes
    };
    if existing.is_none() {
        existing = api(
            tools,
            "POST",
            "releases",
            Some(
                &json!({"tag_name":tag,"target_commitish":sha,"name":if release{format!("VibeCut {tag}")}else{"Nightly".into()},"draft":true,"prerelease":!release,"body":notes}),
            ),
            false,
        )?;
    }
    let existing = existing.ok_or("release creation returned no response")?;
    let id = field(&existing, "/id").as_u64().ok_or("release ID")?;
    let mut upload = vec![arg("release"), arg("upload"), arg(&tag)];
    upload.extend(assets.iter().map(arg));
    if !release {
        upload.push(arg("--clobber"));
    }
    tools.run("gh", upload)?;
    if release {
        if api(tools, "GET", &format!("git/ref/tags/{tag}"), None, true)?.is_none() {
            api(
                tools,
                "POST",
                "git/refs",
                Some(&json!({"ref":format!("refs/tags/{tag}"),"sha":sha})),
                false,
            )?;
        }
        return Ok(());
    }
    if field(&required(tools, "git/ref/heads/main")?, "/object/sha").as_str() != Some(sha.as_str())
    {
        return Err("build superseded during upload; refusing to move nightly tag".into());
    }
    let desired: BTreeSet<_> = assets
        .iter()
        .filter_map(|p| p.file_name().and_then(OsStr::to_str))
        .collect();
    for asset in field(&existing, "/assets")
        .as_array()
        .ok_or("release assets missing")?
    {
        if !desired.contains(field(asset, "/name").as_str().ok_or("asset name")?) {
            api(
                tools,
                "DELETE",
                &format!(
                    "releases/assets/{}",
                    field(asset, "/id").as_u64().ok_or("asset ID")?
                ),
                None,
                false,
            )?;
        }
    }
    if reference.is_some() {
        api(
            tools,
            "PATCH",
            "git/refs/tags/nightly",
            Some(&json!({"sha":sha,"force":true})),
            false,
        )?;
    }
    api(
        tools,
        "PATCH",
        &format!("releases/{id}"),
        Some(
            &json!({"draft":false,"prerelease":true,"name":format!("Nightly {}",sha.get(..7).unwrap_or(&sha)),"body":notes,"make_latest":"false"}),
        ),
        false,
    )?;
    Ok(())
}
pub(super) fn run(tools: &mut Tools, args: &[String]) -> Result<()> {
    let (mode, args) = args.split_first().ok_or("publish mode missing")?;
    let (directory, args) = args.split_first().ok_or("publish directory missing")?;
    let o = Options::parse(args, &[], &["--version"])?;
    let release = match mode.as_str() {
        "nightly" => false,
        "release" => true,
        _ => return Err("publish mode must be nightly or release".into()),
    };
    publish(
        tools,
        Path::new(directory),
        release,
        o.values.get("--version").map(String::as_str),
    )
}
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_foreign_assets_refuse_before_remote_mutation() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let mut tools = Tools::new(&temp.0);
        tools.env.insert("GITHUB_SHA".into(), "new".into());
        tools.env.insert("GH_REPO".into(), "owner/repo".into());
        tools.runner = Box::new(|_, _| panic!("must not call remote"));
        assert!(publish(&mut tools, &temp.0, false, None).is_err());
        fs::write(temp.0.join("unexpected.pkg"), b"fixture").expect("fixture");
        assert!(publish(&mut tools, &temp.0, false, None).is_err());
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::wildcard_imports
)]
mod publication_tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    fn packages(root: &Path, version: &str, suffix: &str) {
        for arch in ["aarch64", "x86_64", "universal"] {
            fs::write(
                root.join(format!("vibecut-{version}-macos-{arch}.{suffix}")),
                b"synthetic package",
            )
            .expect("package");
        }
    }
    fn response(value: Option<Value>) -> Captured {
        Captured {
            code: Some(0),
            stdout: format!(
                "HTTP/2.0 200 OK\r\n\r\n{}",
                value.map(|v| v.to_string()).unwrap_or_default()
            ),
            stderr: String::new(),
        }
    }
    #[test]
    fn stable_release_and_stale_main_head_do_not_mutate_remote_state() {
        for stable in [true, false] {
            let temp = Temp::new(&env::temp_dir()).expect("temp");
            let mut tools = Tools::new(&temp.0);
            packages(&temp.0, tools.version(), "pkg");
            tools.env.insert("GH_REPO".into(), "owner/repo".into());
            tools.env.insert("GITHUB_SHA".into(), "new".into());
            let calls = Rc::new(RefCell::new(Vec::new()));
            let records = calls.clone();
            tools.runner = Box::new(move |command, _| {
                let args: Vec<_> = command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                assert_eq!(args.first().map(String::as_str), Some("api"));
                let method = args.get(3).expect("method").clone();
                let endpoint = args.get(4).expect("endpoint").clone();
                records.borrow_mut().push(method.clone());
                assert_eq!(method, "GET");
                if endpoint.contains("releases/tags/") {
                    if stable {
                        Ok(response(Some(
                            json!({"id":1,"prerelease":false,"assets":[]}),
                        )))
                    } else {
                        Ok(Captured {
                            code: Some(1),
                            stdout: "HTTP/2.0 404 Not Found\r\n\r\n{}".into(),
                            stderr: String::new(),
                        })
                    }
                } else if endpoint.ends_with("heads/main") {
                    Ok(response(Some(json!({"object":{"sha":"later"}}))))
                } else {
                    Ok(Captured {
                        code: Some(1),
                        stdout: "HTTP/2.0 404 Not Found\r\n\r\n{}".into(),
                        stderr: String::new(),
                    })
                }
            });
            assert_eq!(publish(&mut tools, &temp.0, false, None).is_err(), stable);
            assert!(calls.borrow().iter().all(|method| method == "GET"));
        }
    }
    #[test]
    fn upload_failure_preserves_tag_notes_and_old_assets() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let mut tools = Tools::new(&temp.0);
        packages(&temp.0, tools.version(), "pkg");
        tools.env.insert("GH_REPO".into(), "owner/repo".into());
        tools.env.insert("GITHUB_SHA".into(), "new".into());
        let calls = Rc::new(RefCell::new(Vec::new()));
        let records = calls.clone();
        tools.runner = Box::new(move |command, _| {
            let args: Vec<_> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            if args.first().map(String::as_str) == Some("release") {
                records.borrow_mut().push("UPLOAD".to_owned());
                return Ok(Captured {
                    code: Some(1),
                    stdout: String::new(),
                    stderr: "private synthetic tool diagnostic".into(),
                });
            }
            let method = args.get(3).expect("method");
            let endpoint = args.get(4).expect("endpoint");
            records.borrow_mut().push(method.clone());
            assert_eq!(method, "GET");
            let value = if endpoint.contains("releases/tags/") {
                json!({"id":1,"prerelease":true,"assets":[{"id":2,"name":"old.pkg"}]})
            } else if endpoint.ends_with("tags/nightly") {
                json!({"object":{"sha":"old","type":"commit"}})
            } else if endpoint.ends_with("heads/main") {
                json!({"object":{"sha":"new"}})
            } else {
                json!({"status":"ahead","commits":[]})
            };
            Ok(response(Some(value)))
        });
        let error = publish(&mut tools, &temp.0, false, None).expect_err("upload fails");
        assert!(!error.contains("private synthetic"));
        assert_eq!(calls.borrow().last().map(String::as_str), Some("UPLOAD"));
        assert!(
            calls
                .borrow()
                .iter()
                .all(|method| method == "GET" || method == "UPLOAD")
        );
    }
    #[test]
    fn tag_moves_after_all_assets_upload_and_notes_include_commits() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let mut tools = Tools::new(&temp.0);
        packages(&temp.0, tools.version(), "pkg");
        tools.env.insert("GH_REPO".into(), "owner/repo".into());
        tools.env.insert("GITHUB_SHA".into(), "new".into());
        let calls = Rc::new(RefCell::new(Vec::new()));
        let records = calls.clone();
        tools.runner = Box::new(move |command, _| {
            let args: Vec<_> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            if args.first().map(String::as_str) == Some("release") {
                records
                    .borrow_mut()
                    .push(("UPLOAD".into(), String::new(), Value::Null));
                return Ok(response(None));
            }
            let method = args.get(3).expect("method").clone();
            let endpoint = args.get(4).expect("endpoint").clone();
            let body = if let Some(index) = args.iter().position(|arg| arg == "--input") {
                serde_json::from_str(
                    &fs::read_to_string(args.get(index + 1).expect("input path")).expect("input"),
                )
                .expect("JSON")
            } else {
                Value::Null
            };
            records
                .borrow_mut()
                .push((method.clone(), endpoint.clone(), body));
            if method != "GET" {
                return Ok(response(None));
            }
            let value = if endpoint.contains("releases/tags/") {
                json!({"id":1,"prerelease":true,"assets":[{"id":2,"name":"old.pkg"}]})
            } else if endpoint.ends_with("tags/nightly") {
                json!({"object":{"sha":"old","type":"commit"}})
            } else if endpoint.ends_with("heads/main") {
                json!({"object":{"sha":"new"}})
            } else {
                json!({"status":"ahead","total_commits":1,"commits":[{"sha":"abcdef1234","commit":{"message":"Implemented\nbody"}}]})
            };
            Ok(response(Some(value)))
        });
        publish(&mut tools, &temp.0, false, None).expect("publish");
        let calls = calls.borrow();
        let upload = calls
            .iter()
            .position(|call| call.0 == "UPLOAD")
            .expect("upload");
        let moved = calls
            .iter()
            .position(|call| call.1.ends_with("git/refs/tags/nightly"))
            .expect("move");
        assert!(upload < moved);
        assert!(
            calls.last().expect("release patch").2["body"]
                .as_str()
                .expect("notes")
                .contains("- abcdef1 Implemented")
        );
        assert!(temp.0.join("SHA256SUMS").is_file());
    }
    #[test]
    fn signed_release_refuses_published_release_and_wrong_commit_tag() {
        for (draft, tag) in [(false, "new"), (true, "old")] {
            let temp = Temp::new(&env::temp_dir()).expect("temp");
            let mut tools = Tools::new(&temp.0);
            packages(&temp.0, tools.version(), "dmg");
            let version = tools.version();
            tools.env.insert("GH_REPO".into(), "owner/repo".into());
            tools.env.insert("GITHUB_SHA".into(), "new".into());
            tools.runner = Box::new(move |command, _| {
                let args: Vec<_> = command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                assert_eq!(args.get(3).map(String::as_str), Some("GET"));
                let endpoint = args.get(4).expect("endpoint");
                Ok(response(Some(if endpoint.contains("releases/tags/") {
                    json!({"id":1,"draft":draft,"assets":[]})
                } else {
                    json!({"object":{"sha":tag,"type":"commit"}})
                })))
            });
            assert!(publish(&mut tools, &temp.0, true, Some(version)).is_err());
        }
    }
}
