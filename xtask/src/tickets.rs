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

fn is_open(ticket: &Ticket) -> bool {
    ticket.status == "open"
}

fn unresolved(ticket: &Ticket) -> bool {
    !finished(ticket)
}

fn blocker_errors(tickets: &[Ticket]) -> Vec<String> {
    let by_id: BTreeMap<_, _> = tickets.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut errors = Vec::new();
    for ticket in tickets {
        for dep in &ticket.blocked {
            match by_id.get(dep.as_str()) {
                None => errors.push(format!("ERROR missing {} -> {dep}", ticket.id)),
                Some(blocker) if blocker.status == "wontfix" => {
                    errors.push(format!("ERROR wontfix blocker {} -> {dep}", ticket.id))
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

fn dependents(
    id: &str,
    reverse: &BTreeMap<&str, Vec<&str>>,
    memo: &mut BTreeMap<String, BTreeSet<String>>,
    path: &mut BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    if let Some(dependents) = memo.get(id) {
        return Ok(dependents.clone());
    }
    if path.len() >= 512 || !path.insert(id.to_owned()) {
        return Err(format!("cyclic or over-512-depth ticket graph at {id}"));
    }
    let mut result = BTreeSet::new();
    if let Some(children) = reverse.get(id) {
        for child in children {
            result.insert((*child).to_owned());
            result.extend(dependents(child, reverse, memo, path)?);
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
    Ok(memo)
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
    Ok(Value::Array(tickets.iter().map(|ticket| {
        let waiting = waiting(ticket, &by_id);
        json!({
            "id": ticket.id, "milestone": ticket.milestone, "title": ticket.title,
            "status": ticket.status, "type": ticket.kind, "priority": ticket.priority,
            "assignee": ticket.assignee, "blockedBy": ticket.blocked, "mtime": ticket.mtime, "text": ticket.text,
            "waitingOn": waiting, "ready": ticket.status == "open" && waiting.is_empty(),
            "unblocks": count_dependents(&ticket.id, &dependents, &by_id, is_open),
            "wave": waves.get(&ticket.id), "critical": critical.contains(&ticket.id),
        })
    }).collect()))
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
                "01-codex.md",
                "Status: open\nPriority: P0\nAssignee: codex".into(),
                0.0,
            ),
            parse(
                "m",
                "02-opencode.md",
                "Status: open\nPriority: P0\nAssignee: opencode".into(),
                0.0,
            ),
        ];
        assert_eq!(frontier_rows(&tickets, None).unwrap().len(), 2);
        let rows = frontier_rows(&tickets, Some("codex")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "m/01");
        assert_eq!(rows[0].assignee, "codex");
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
        assert_eq!(
            blocker_errors(&tickets),
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
        assert!(
            dependents_by_id(&tickets)
                .unwrap_err()
                .contains("over-512-depth")
        );
    }
}
