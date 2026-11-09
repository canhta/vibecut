//! Read-only local ticket graph and its existing board presentation.
mod tunnel;
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

fn is_open(ticket: &Ticket) -> bool {
    ticket.status == "open"
}

fn unresolved(ticket: &Ticket) -> bool {
    !finished(ticket)
}

#[derive(Debug, PartialEq, Eq)]
enum BlockerProblem {
    Missing { ticket: String, blocker: String },
    Wontfix { ticket: String, blocker: String },
}

impl BlockerProblem {
    fn ticket(&self) -> &str {
        match self {
            Self::Missing { ticket, .. } | Self::Wontfix { ticket, .. } => ticket,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Missing { .. } => "missing blocker",
            Self::Wontfix { .. } => "wontfix blocker",
        }
    }

    fn blocker(&self) -> &str {
        match self {
            Self::Missing { blocker, .. } | Self::Wontfix { blocker, .. } => blocker,
        }
    }
}

impl std::fmt::Display for BlockerProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { ticket, blocker } => write!(f, "ERROR missing {ticket} -> {blocker}"),
            Self::Wontfix { ticket, blocker } => {
                write!(f, "ERROR wontfix blocker {ticket} -> {blocker}")
            }
        }
    }
}

fn blocker_errors(tickets: &[Ticket]) -> Vec<BlockerProblem> {
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut errors = Vec::new();
    for ticket in tickets {
        for dep in &ticket.blocked {
            match by_id.get(dep.as_str()) {
                None => errors.push(BlockerProblem::Missing {
                    ticket: ticket.id.clone(),
                    blocker: dep.clone(),
                }),
                Some(blocker) if blocker.status == "wontfix" => {
                    errors.push(BlockerProblem::Wontfix {
                        ticket: ticket.id.clone(),
                        blocker: dep.clone(),
                    })
                }
                Some(_) => (),
            }
        }
    }
    errors
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

#[derive(Clone, Debug)]
struct Dependents {
    ids: BTreeSet<String>,
    depth: usize,
}

fn dependents(
    id: &str,
    reverse: &BTreeMap<&str, Vec<&str>>,
    memo: &mut BTreeMap<String, Dependents>,
    path: &mut BTreeSet<String>,
) -> Result<Dependents, String> {
    if let Some(dependents) = memo.get(id) {
        if path.len() + dependents.depth > 512 {
            return Err(format!("cyclic or over-512-depth ticket graph at {id}"));
        }
        return Ok(dependents.clone());
    }
    if path.len() >= 512 || !path.insert(id.to_owned()) {
        return Err(format!("cyclic or over-512-depth ticket graph at {id}"));
    }
    let mut result = Dependents {
        ids: BTreeSet::new(),
        depth: 1,
    };
    if let Some(children) = reverse.get(id) {
        for child in children {
            result.ids.insert((*child).to_owned());
            let child = dependents(child, reverse, memo, path)?;
            result.depth = result.depth.max(1 + child.depth);
            result.ids.extend(child.ids);
        }
    }
    path.remove(id);
    memo.insert(id.to_owned(), result.clone());
    Ok(result)
}

fn dependents_by_id(tickets: &[Ticket]) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let known: BTreeSet<&str> = tickets.iter().map(|t| t.id.as_str()).collect();
    let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for ticket in tickets {
        for blocker in &ticket.blocked {
            if known.contains(blocker.as_str()) {
                reverse
                    .entry(blocker.as_str())
                    .or_default()
                    .push(ticket.id.as_str());
            }
        }
    }
    let mut memo = BTreeMap::new();
    for ticket in tickets {
        dependents(&ticket.id, &reverse, &mut memo, &mut BTreeSet::new())?;
    }
    Ok(memo
        .into_iter()
        .map(|(id, dependents)| (id, dependents.ids))
        .collect())
}

fn count_dependents(
    id: &str,
    dependents: &BTreeMap<String, BTreeSet<String>>,
    by_id: &BTreeMap<&str, &Ticket>,
    keep: fn(&Ticket) -> bool,
) -> usize {
    dependents
        .get(id)
        .into_iter()
        .flatten()
        .filter_map(|dependent| by_id.get(dependent.as_str()))
        .filter(|ticket| keep(ticket))
        .count()
}

#[derive(Debug, PartialEq, Eq)]
struct Row {
    id: String,
    priority: String,
    assignee: String,
    status: String,
    filename: String,
    dependents: usize,
}

fn frontier_rows(tickets: &[Ticket], assignee: Option<&str>) -> Result<Vec<Row>, String> {
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    let dependents = dependents_by_id(tickets)?;
    let mut rows: Vec<_> = tickets
        .iter()
        .filter(|ticket| {
            is_open(ticket)
                && waiting(ticket, &by_id).is_empty()
                && assignee.is_none_or(|assignee| ticket.assignee == assignee)
        })
        .map(|ticket| Row {
            id: ticket.id.clone(),
            priority: ticket.priority.clone(),
            assignee: ticket.assignee.clone(),
            status: ticket.status.clone(),
            filename: ticket.filename.clone(),
            dependents: count_dependents(&ticket.id, &dependents, &by_id, is_open),
        })
        .collect();
    rows.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| b.dependents.cmp(&a.dependents))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(rows)
}

fn blocking_rows(tickets: &[Ticket]) -> Result<Vec<Row>, String> {
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    let dependents = dependents_by_id(tickets)?;
    let mut rows: Vec<_> = tickets
        .iter()
        .filter(|ticket| unresolved(ticket))
        .map(|ticket| Row {
            id: ticket.id.clone(),
            priority: ticket.priority.clone(),
            assignee: ticket.assignee.clone(),
            status: ticket.status.clone(),
            filename: ticket.filename.clone(),
            dependents: count_dependents(&ticket.id, &dependents, &by_id, unresolved),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.dependents
            .cmp(&a.dependents)
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(rows)
}

fn data(tickets: &[Ticket]) -> Result<Value, String> {
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    let dependents = dependents_by_id(tickets)?;
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
    let mut problems = Vec::new();
    for ticket in tickets {
        if ticket.status == "?" {
            problems.push(json!({
                "id": ticket.id, "severity": "warning", "message": "missing Status field"
            }));
        }
    }
    for problem in blocker_errors(tickets) {
        problems.push(json!({
            "id": problem.ticket(), "severity": "error",
            "message": format!("{}: {}", problem.message(), problem.blocker()),
        }));
    }
    let signature = tickets
        .iter()
        .map(|ticket| {
            format!(
                "{}:{}:{}:{}",
                ticket.id, ticket.mtime, ticket.status, ticket.priority
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    Ok(json!({
        "signature": signature,
        "problems": problems,
        "tickets": tickets.iter().map(|ticket| {
            let waiting = waiting(ticket, &by_id);
            json!({
                "id": ticket.id, "milestone": ticket.milestone, "title": ticket.title,
                "status": ticket.status, "type": ticket.kind, "priority": ticket.priority,
                "assignee": ticket.assignee, "blockedBy": ticket.blocked, "mtime": ticket.mtime,
                "path": format!(".scratch/{}/issues/{}", ticket.milestone, ticket.filename),
                "waitingOn": waiting, "ready": ticket.status == "open" && waiting.is_empty(),
                "unblocks": count_dependents(&ticket.id, &dependents, &by_id, is_open),
                "wave": waves.get(&ticket.id), "critical": critical.contains(&ticket.id),
            })
        }).collect::<Vec<_>>(),
    }))
}

fn ticket_data(tickets: &[Ticket], id: &str) -> Option<Value> {
    tickets.iter().find(|ticket| ticket.id == id).map(|ticket| {
        json!({
            "id": ticket.id, "mtime": ticket.mtime, "text": ticket.text,
        })
    })
}

fn percent_decode(input: &str) -> String {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let mut decoded = Vec::with_capacity(input.len());
    let mut rest = input.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%'
            && let [high, low, after @ ..] = tail
            && let (Some(high), Some(low)) = (hex(*high), hex(*low))
        {
            decoded.push(high * 16 + low);
            rest = after;
            continue;
        }
        decoded.push(if byte == b'+' { b' ' } else { byte });
        rest = tail;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn query_param(request: &str, name: &str) -> Option<String> {
    let query = request.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| percent_decode(value))
    })
}

pub fn frontier(root: &Path, assignee: Option<&str>) -> Result<(), String> {
    let tickets = load(root)?;
    for error in blocker_errors(&tickets) {
        println!("{error}");
    }
    for row in frontier_rows(&tickets, assignee)? {
        println!(
            "{} {} {} {} {}",
            row.priority, row.assignee, row.id, row.dependents, row.filename
        );
    }
    Ok(())
}

pub fn blocking(root: &Path) -> Result<(), String> {
    let tickets = load(root)?;
    for error in blocker_errors(&tickets) {
        println!("{error}");
    }
    for row in blocking_rows(&tickets)? {
        println!(
            "{} {} {} {} {} {}",
            row.dependents, row.status, row.priority, row.assignee, row.id, row.filename
        );
    }
    Ok(())
}

fn authorized(request: &str, token: Option<&str>) -> bool {
    token.is_none_or(|token| {
        request
            .split_whitespace()
            .nth(1)
            .and_then(|target| query_param(target, "token"))
            .as_deref()
            == Some(token)
    })
}

fn serve(root: &Path, stream: &mut TcpStream, token: Option<&str>) -> Result<(), String> {
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
    } else if !authorized(&request, token) {
        (
            "401 Unauthorized",
            "text/plain",
            "board token required".to_owned(),
        )
    } else {
        let target = fields.get(1).copied().unwrap_or("");
        match target.split('?').next().unwrap_or("") {
            "/" => (
                "200 OK",
                "text/html; charset=utf-8",
                include_str!("tickets/board.html").to_owned(),
            ),
            "/data.json" => match load(root).and_then(|tickets| data(&tickets)) {
                Ok(data) => ("200 OK", "application/json", data.to_string()),
                Err(error) => ("500 Internal Server Error", "text/plain", error),
            },
            "/ticket.json" => match query_param(target, "id") {
                None => ("400 Bad Request", "text/plain", "missing id".to_owned()),
                Some(id) => match load(root) {
                    Ok(tickets) => match ticket_data(&tickets, &id) {
                        Some(ticket) => ("200 OK", "application/json", ticket.to_string()),
                        None => ("404 Not Found", "text/plain", "unknown ticket".to_owned()),
                    },
                    Err(error) => ("500 Internal Server Error", "text/plain", error),
                },
            },
            _ => ("404 Not Found", "text/plain", "not found".to_owned()),
        }
    };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())
}

pub fn board(root: &Path, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let _tunnel = tunnel::start(root, port)?;
    let token = std::env::var("VIBECUT_BOARD_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    println!(
        "VibeCut board (read-only) at http://{}/  Ctrl+C to stop",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    if token.is_some() {
        println!("VIBECUT_BOARD_TOKEN is set; append ?token=<value> to every URL");
    }
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| e.to_string())?;
        if let Err(error) = serve(root, &mut stream, token.as_deref())
            && !error.contains("aborted")
            && !error.contains("reset")
            && !error.contains("closed")
        {
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
                "Status: open\nBlocked by: 01\nAssignee: opencode".into(),
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
        let out = out["tickets"].as_array().unwrap();
        assert_eq!(out[0]["wave"], 0);
        assert_eq!(out[1]["wave"], 1);
        assert_eq!(out[1]["ready"], true);
        assert_eq!(out[1]["unblocks"], 1);
        assert_eq!(out[2]["wave"], 2);
        assert_eq!(out[2]["waitingOn"], json!(["m/02", "missing/99"]));
        assert_eq!(out[4]["ready"], false);
        for ticket in out.iter().take(3) {
            assert_eq!(ticket["critical"], true);
        }
        let cyclic = vec![parse("m", "01-cycle.md", "Blocked by: 01".into(), 0.0)];
        assert!(data(&cyclic).unwrap_err().contains("cyclic"));
        assert_eq!(out[0]["unblocks"], 3);
        assert_eq!(out[3]["unblocks"], 1);
        assert_eq!(out[4]["unblocks"], 0);
    }

    fn ticket(
        milestone: &str,
        filename: &str,
        status: &str,
        priority: &str,
        blocked: &str,
    ) -> Ticket {
        parse(
            milestone,
            filename,
            format!("Status: {status}\nPriority: {priority}\nBlocked by: {blocked}"),
            0.0,
        )
    }

    fn open_counts(tickets: &[Ticket]) -> BTreeMap<String, usize> {
        let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
        let dependents = dependents_by_id(tickets).unwrap();
        tickets
            .iter()
            .map(|ticket| {
                (
                    ticket.id.clone(),
                    count_dependents(&ticket.id, &dependents, &by_id, is_open),
                )
            })
            .collect()
    }

    #[test]
    fn diamond_counts_transitive_open_dependents() {
        let tickets = vec![
            ticket("m", "01-top.md", "open", "P1", ""),
            ticket("m", "02-left.md", "open", "P1", "01"),
            ticket("m", "03-right.md", "open", "P1", "01"),
            ticket("m", "04-bottom.md", "open", "P1", "02, 03"),
        ];
        let counts = open_counts(&tickets);
        assert_eq!(counts["m/01"], 3);
        assert_eq!(counts["m/02"], 1);
        assert_eq!(counts["m/03"], 1);
        assert_eq!(counts["m/04"], 0);
    }

    #[test]
    fn chain_counts_transitive_open_dependents() {
        let tickets = vec![
            ticket("m", "01-a.md", "open", "P1", ""),
            ticket("m", "02-b.md", "open", "P1", "01"),
            ticket("m", "03-c.md", "open", "P1", "02"),
            ticket("m", "04-d.md", "open", "P1", "03"),
        ];
        let counts = open_counts(&tickets);
        assert_eq!(counts["m/01"], 3);
        assert_eq!(counts["m/02"], 2);
        assert_eq!(counts["m/03"], 1);
        assert_eq!(counts["m/04"], 0);
    }

    #[test]
    fn frontier_orders_priority_then_open_dependents_then_number() {
        let tickets = vec![
            ticket("m", "01-a.md", "open", "P0", ""),
            ticket("m", "02-a1.md", "open", "P0", "01"),
            ticket("m", "03-a2.md", "open", "P0", "02"),
            ticket("m", "04-a3.md", "open", "P0", "03"),
            ticket("m", "05-b.md", "open", "P0", ""),
            ticket("m", "06-b1.md", "open", "P0", "05"),
            ticket("m", "07-c.md", "open", "P0", ""),
            ticket("m", "08-d.md", "open", "P1", ""),
            ticket("m", "09-d1.md", "open", "P1", "08"),
            ticket("m", "10-d2.md", "open", "P1", "09"),
            ticket("m", "11-d3.md", "open", "P1", "10"),
            ticket("m", "12-e.md", "open", "P0", ""),
            ticket("m", "13-e1.md", "open", "P0", "12"),
            ticket("m", "14-e2.md", "open", "P0", "13"),
            ticket("m", "15-f.md", "open", "P0", ""),
            ticket("m", "16-f1.md", "open", "P0", "15"),
            ticket("m", "17-f2.md", "open", "P0", "16"),
        ];
        let rows = frontier_rows(&tickets, None).unwrap();
        let ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["m/01", "m/12", "m/15", "m/05", "m/07", "m/08"]);
        let counts: Vec<_> = rows.iter().map(|row| row.dependents).collect();
        assert_eq!(counts, [3, 2, 2, 1, 0, 3]);
    }

    #[test]
    fn frontier_assignee_filter_keeps_only_ready_matching_tickets() {
        let tickets = vec![
            parse(
                "m",
                "01-opencode.md",
                "Status: open\nPriority: P0\nAssignee: opencode".into(),
                0.0,
            ),
            parse(
                "m",
                "02-sonnet.md",
                "Status: open\nPriority: P0\nAssignee: claude-sonnet".into(),
                0.0,
            ),
        ];
        assert_eq!(frontier_rows(&tickets, None).unwrap().len(), 2);
        let rows = frontier_rows(&tickets, Some("opencode")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "m/01");
        assert_eq!(rows[0].assignee, "opencode");
    }

    #[test]
    fn blocking_lists_unresolved_tickets_with_status_by_unresolved_dependents() {
        let tickets = vec![
            ticket("m", "01-hub.md", "claimed", "P0", ""),
            ticket("m", "02-waiting.md", "blocked", "P0", "01"),
            ticket("m", "03-later.md", "open", "P0", "02"),
            ticket("m", "04-done.md", "resolved", "P0", "01"),
            ticket("m", "05-done-later.md", "open", "P0", "04"),
        ];
        let rows = blocking_rows(&tickets).unwrap();
        let ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["m/01", "m/02", "m/03", "m/05"]);
        let counts: Vec<_> = rows.iter().map(|row| row.dependents).collect();
        assert_eq!(counts, [3, 1, 0, 0]);
        let statuses: Vec<_> = rows.iter().map(|row| row.status.as_str()).collect();
        assert_eq!(statuses, ["claimed", "blocked", "open", "open"]);
        assert!(!rows.iter().any(|row| row.id == "m/04"));
    }

    #[test]
    fn frontier_reports_missing_and_wontfix_blockers() {
        let tickets = vec![
            ticket("m", "01-missing.md", "open", "P0", "99"),
            ticket("m", "02-dropped.md", "wontfix", "P0", ""),
            ticket("m", "03-blocked.md", "open", "P0", "02"),
        ];
        assert!(frontier_rows(&tickets, None).unwrap().is_empty());
        assert_eq!(open_counts(&tickets)["m/02"], 1);
        assert_eq!(
            blocker_errors(&tickets)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "ERROR missing m/01 -> m/99".to_owned(),
                "ERROR wontfix blocker m/03 -> m/02".to_owned(),
            ]
        );
    }

    #[test]
    fn dependency_counts_report_cycles_instead_of_looping() {
        let pair = vec![
            ticket("m", "01-a.md", "open", "P0", "02"),
            ticket("m", "02-b.md", "open", "P0", "01"),
        ];
        assert!(dependents_by_id(&pair).unwrap_err().contains("cyclic"));
        assert!(frontier_rows(&pair, None).unwrap_err().contains("cyclic"));
        let self_cycle = vec![ticket("m", "01-a.md", "open", "P0", "01")];
        assert!(
            dependents_by_id(&self_cycle)
                .unwrap_err()
                .contains("cyclic")
        );
    }

    #[test]
    fn dependency_counts_refuse_graphs_deeper_than_512() {
        let mut tickets = Vec::new();
        for number in 0..513 {
            let blocked = if number == 0 {
                String::new()
            } else {
                format!("{:03}", number - 1)
            };
            tickets.push(ticket(
                "m",
                &format!("{number:03}-t.md"),
                "open",
                "P0",
                &blocked,
            ));
        }
        for reverse in [false, true] {
            if reverse {
                tickets.reverse();
            }
            for result in [
                frontier_rows(&tickets, None).map(|_| ()),
                blocking_rows(&tickets).map(|_| ()),
                data(&tickets).map(|_| ()),
            ] {
                assert!(result.unwrap_err().contains("over-512-depth"));
            }
        }
        tickets.remove(0);
        assert!(dependents_by_id(&tickets).is_ok());
    }

    #[test]
    fn empty_graph_has_no_frontier_blocking_or_board_rows() {
        assert!(frontier_rows(&[], None).unwrap().is_empty());
        assert!(blocking_rows(&[]).unwrap().is_empty());
        assert_eq!(data(&[]).unwrap()["tickets"], json!([]));
        assert_eq!(data(&[]).unwrap()["problems"], json!([]));
    }

    #[test]
    fn cross_milestone_counts_and_frontier_order_are_stable() {
        let mut tickets = vec![
            ticket("a", "01-ready.md", "open", "P1", ""),
            ticket("b", "02-waiting.md", "open", "P1", "a/01, missing/99"),
            ticket("c", "03-waiting.md", "open", "P1", "b/02"),
            ticket("d", "04-ready.md", "open", "P1", ""),
        ];
        let rows = frontier_rows(&tickets, None).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (row.id.as_str(), row.dependents))
                .collect::<Vec<_>>(),
            [("a/01", 2), ("d/04", 0)]
        );
        let board = data(&tickets).unwrap();
        let board = board["tickets"].as_array().unwrap();
        assert_eq!(board[0]["unblocks"], 2);
        assert_eq!(board[1]["ready"], false);
        assert_eq!(
            blocker_errors(&tickets)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["ERROR missing b/02 -> missing/99"]
        );
        tickets.reverse();
        assert_eq!(frontier_rows(&tickets, None).unwrap(), rows);
    }

    #[test]
    fn board_payload_separates_ticket_text_and_reports_data_problems() {
        let tickets = vec![
            parse("m", "01-hub.md", "Priority: P0\n# hub\n\nBody".into(), 7.0),
            ticket("m", "02-missing.md", "open", "P1", "99"),
            ticket("m", "03-blocked.md", "open", "P1", "02"),
        ];
        let board = data(&tickets).unwrap();
        let tickets_json = board["tickets"].as_array().unwrap();
        assert_eq!(tickets_json[0]["id"], "m/01");
        assert!(tickets_json[0].get("text").is_none());
        assert_eq!(tickets_json[0]["path"], ".scratch/m/issues/01-hub.md");
        assert!(board["signature"].as_str().unwrap().contains("m/01:7:?:P0"));
        let problems = board["problems"].as_array().unwrap();
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0]["id"], "m/01");
        assert_eq!(problems[0]["message"], "missing Status field");
        assert_eq!(problems[1]["id"], "m/02");
        assert!(problems[1]["message"].as_str().unwrap().contains("m/99"));

        let ticket = ticket_data(&tickets, "m/01").unwrap();
        assert_eq!(ticket["text"], "Priority: P0\n# hub\n\nBody");
        assert!(ticket_data(&tickets, "../01").is_none());
        assert_eq!(
            query_param("/ticket.json?id=m%2F01", "id").as_deref(),
            Some("m/01")
        );
        assert_eq!(
            query_param("/ticket.json?id=m+01", "id").as_deref(),
            Some("m 01")
        );
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert!(authorized("GET /data.json HTTP/1.1\r\n", None));
        assert!(authorized(
            "GET /data.json?token=s3cret HTTP/1.1\r\n",
            Some("s3cret")
        ));
        assert!(!authorized("GET /data.json HTTP/1.1\r\n", Some("s3cret")));
        assert!(!authorized(
            "GET /data.json?token=wrong HTTP/1.1\r\n",
            Some("s3cret")
        ));
    }
}
