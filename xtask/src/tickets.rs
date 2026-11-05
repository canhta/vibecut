//! Read-only local ticket graph and its existing board presentation.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug)]
struct Ticket {
    id: String,
    milestone: String,
    filename: String,
    title: String,
    status: String,
    kind: String,
    priority: String,
    assignee: String,
    blocked: Vec<String>,
    mtime: f64,
    text: String,
}

fn field<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':').map(str::trim))
        .unwrap_or("")
}

fn parse(milestone: &str, filename: &str, text: String, mtime: f64) -> Ticket {
    let blocked = field(&text, "Blocked by")
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty() && !matches!(*s, "none" | "-"))
        .map(|s| {
            if s.contains('/') {
                s.to_owned()
            } else {
                format!("{milestone}/{s}")
            }
        })
        .collect();
    let default = |name, fallback| {
        let value = field(&text, name);
        if value.is_empty() { fallback } else { value }.to_owned()
    };
    Ticket {
        id: format!(
            "{milestone}/{}",
            filename
                .split('-')
                .next()
                .unwrap_or(filename)
                .trim_end_matches(".md")
        ),
        milestone: milestone.to_owned(),
        filename: filename.to_owned(),
        title: text
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .unwrap_or(filename.trim_end_matches(".md"))
            .to_owned(),
        status: default("Status", "?"),
        kind: field(&text, "Type").to_owned(),
        priority: default("Priority", "P?"),
        assignee: default("Assignee", "-"),
        blocked,
        mtime,
        text,
    }
}

fn load(root: &Path) -> Result<Vec<Ticket>, String> {
    let scratch = crate::checkout::main(root)?.join(".scratch");
    let mut tickets = Vec::new();
    for milestone in fs::read_dir(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))? {
        let milestone = milestone.map_err(|e| e.to_string())?;
        if !milestone.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let issues = milestone.path().join("issues");
        if !issues.is_dir() {
            continue;
        }
        let name = milestone
            .file_name()
            .into_string()
            .map_err(|_| "milestone filename is not UTF-8")?;
        for file in fs::read_dir(issues).map_err(|e| e.to_string())? {
            let file = file.map_err(|e| e.to_string())?;
            if !file.file_type().map_err(|e| e.to_string())?.is_file()
                || file.path().extension().is_none_or(|ext| ext != "md")
            {
                continue;
            }
            let filename = file
                .file_name()
                .into_string()
                .map_err(|_| "ticket filename is not UTF-8")?;
            let text = fs::read_to_string(file.path()).map_err(|e| e.to_string())?;
            let mtime = file
                .metadata()
                .and_then(|m| m.modified())
                .map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            tickets.push(parse(&name, &filename, text, mtime));
        }
    }
    tickets.sort_by(|a, b| a.id.cmp(&b.id));
    if tickets
        .windows(2)
        .any(|pair| pair.first().map(|t| &t.id) == pair.get(1).map(|t| &t.id))
    {
        return Err("duplicate ticket ID".into());
    }
    Ok(tickets)
}

fn finished(ticket: &Ticket) -> bool {
    matches!(ticket.status.as_str(), "resolved" | "wontfix")
}

fn waiting(ticket: &Ticket, by_id: &BTreeMap<&str, &Ticket>) -> Vec<String> {
    ticket
        .blocked
        .iter()
        .filter(|id| {
            by_id
                .get(id.as_str())
                .is_none_or(|t| t.status != "resolved")
        })
        .cloned()
        .collect()
}

fn wave(
    id: &str,
    by_id: &BTreeMap<&str, &Ticket>,
    memo: &mut BTreeMap<String, usize>,
    seen: &mut BTreeSet<String>,
) -> Result<usize, String> {
    if let Some(wave) = memo.get(id) {
        return Ok(*wave);
    }
    if seen.len() >= 512 || !seen.insert(id.to_owned()) {
        return Err(format!("cyclic or over-512-depth ticket graph at {id}"));
    }
    let mut result = 0;
    if let Some(ticket) = by_id.get(id) {
        for dep in &ticket.blocked {
            if let Some(ticket) = by_id.get(dep.as_str())
                && ticket.status != "wontfix"
            {
                result = result.max(1 + wave(dep, by_id, memo, seen)?);
            }
        }
    }
    seen.remove(id);
    memo.insert(id.to_owned(), result);
    Ok(result)
}

fn data(tickets: &[Ticket]) -> Result<Value, String> {
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut waves = BTreeMap::new();
    for ticket in tickets {
        wave(&ticket.id, &by_id, &mut waves, &mut BTreeSet::new())?;
    }
    let mut critical = BTreeSet::new();
    let mut current = tickets
        .iter()
        .rev()
        .filter(|t| !finished(t))
        .max_by_key(|t| (waves.get(&t.id), t.priority == "P0"));
    while let Some(ticket) = current {
        critical.insert(ticket.id.clone());
        current = ticket
            .blocked
            .iter()
            .rev()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .filter(|t| t.status != "wontfix")
            .max_by_key(|t| waves.get(&t.id));
    }
    Ok(Value::Array(tickets.iter().map(|ticket| {
        let waiting = waiting(ticket, &by_id);
        json!({
            "id": ticket.id, "milestone": ticket.milestone, "title": ticket.title,
            "status": ticket.status, "type": ticket.kind, "priority": ticket.priority,
            "assignee": ticket.assignee, "blockedBy": ticket.blocked, "mtime": ticket.mtime, "text": ticket.text,
            "waitingOn": waiting, "ready": ticket.status == "open" && waiting.is_empty(),
            "unblocks": tickets.iter().filter(|other| !finished(other) && other.blocked.contains(&ticket.id)).count(),
            "wave": waves.get(&ticket.id), "critical": critical.contains(&ticket.id),
        })
    }).collect()))
}

pub fn frontier(root: &Path, assignee: Option<&str>) -> Result<(), String> {
    let mut tickets = load(root)?;
    tickets.sort_by(|a, b| (&a.priority, &a.id).cmp(&(&b.priority, &b.id)));
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    for ticket in &tickets {
        for dep in &ticket.blocked {
            match by_id.get(dep.as_str()) {
                None => println!("ERROR missing {} -> {dep}", ticket.id),
                Some(t) if t.status == "wontfix" => {
                    println!("ERROR wontfix blocker {} -> {dep}", ticket.id)
                }
                _ => (),
            }
        }
        if ticket.status == "open"
            && waiting(ticket, &by_id).is_empty()
            && assignee.is_none_or(|a| ticket.assignee == a)
        {
            println!(
                "{} {} {} {}",
                ticket.priority, ticket.assignee, ticket.id, ticket.filename
            );
        }
    }
    // Diagnose cycles instead of leaving apparently ready but unreachable work.
    data(&tickets).map(|_| ())
}

fn serve(root: &Path, stream: &mut TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let mut first = Vec::new();
    BufReader::new((&mut *stream).take(8193))
        .read_until(b'\n', &mut first)
        .map_err(|e| e.to_string())?;
    let request = String::from_utf8(first).map_err(|e| e.to_string())?;
    let fields: Vec<_> = request.split_whitespace().collect();
    let (status, kind, body) = if request.len() > 8192 || fields.len() != 3 {
        (
            "400 Bad Request",
            "text/plain",
            "invalid request".to_owned(),
        )
    } else if fields.first() != Some(&"GET") {
        (
            "405 Method Not Allowed",
            "text/plain",
            "read-only server".to_owned(),
        )
    } else {
        match fields.get(1).and_then(|path| path.split('?').next()) {
            Some("/") => (
                "200 OK",
                "text/html; charset=utf-8",
                include_str!("board.html").to_owned(),
            ),
            Some("/data.json") => match load(root).and_then(|tickets| data(&tickets)) {
                Ok(data) => ("200 OK", "application/json", data.to_string()),
                Err(error) => ("500 Internal Server Error", "text/plain", error),
            },
            _ => ("404 Not Found", "text/plain", "not found".to_owned()),
        }
    };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())
}

pub fn board(root: &Path, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    println!(
        "VibeCut board (read-only) at http://{}/  Ctrl+C to stop",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| e.to_string())?;
        if let Err(error) = serve(root, &mut stream) {
            eprintln!("board request: {error}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_graph_preserves_frontier_waves_and_critical_path() {
        let tickets = vec![
            parse(
                "m",
                "01-base.md",
                "Status: resolved\nPriority: P0\n# base".into(),
                1.0,
            ),
            parse(
                "m",
                "02-ready.md",
                "Status: open\nBlocked by: 01\nAssignee: codex".into(),
                2.0,
            ),
            parse(
                "other",
                "03-later.md",
                "Status: open\nBlocked by: m/02, missing/99\nPriority: P0".into(),
                3.0,
            ),
            parse(
                "m",
                "04-dropped.md",
                "Status: wontfix\nBlocked by: 01".into(),
                4.0,
            ),
            parse(
                "m",
                "05-stuck.md",
                "Status: open\nBlocked by: 04".into(),
                5.0,
            ),
        ];
        let out = data(&tickets).unwrap();
        assert_eq!(out[0]["wave"], 0);
        assert_eq!(out[1]["wave"], 1);
        assert_eq!(out[1]["ready"], true);
        assert_eq!(out[1]["unblocks"], 1);
        assert_eq!(out[2]["wave"], 2);
        assert_eq!(out[2]["waitingOn"], json!(["m/02", "missing/99"]));
        assert_eq!(out[4]["ready"], false);
        for i in 0..3 {
            assert_eq!(out[i]["critical"], true);
        }
        let cyclic = vec![parse("m", "01-cycle.md", "Blocked by: 01".into(), 0.0)];
        assert!(data(&cyclic).unwrap_err().contains("cyclic"));
    }
}
