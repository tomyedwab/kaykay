//! Built-in command handlers.

use crate::store::{self, Ticket};
use crate::util::{emit, iso_now, random_suffix};
use crate::{deps, Res};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{IsTerminal, Read, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};

const VALID_STATUSES: [&str; 3] = ["open", "in_progress", "closed"];

/// Print a message to stdout and fail without a further error message.
fn fail_quietly(msg: &str) -> Res {
    emit(&format!("{msg}\n"));
    Err(String::new())
}

fn usage(text: &str) -> Res {
    Err(format!("Usage: kk {text}"))
}

fn git_user_name() -> String {
    Command::new("git")
        .args(["config", "user.name"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn create(dir: &Path, args: &[String]) -> Res {
    let prefix = std::env::var(store::PREFIX_ENV).unwrap_or_default();
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return Err(format!(
            "Error: {} is not set\nSet it to the prefix for new ticket IDs (e.g. export {}=kk)",
            store::PREFIX_ENV,
            store::PREFIX_ENV
        ));
    }
    if prefix.contains(['/', '\\']) || prefix.contains(char::is_whitespace) {
        return Err(format!(
            "Error: {} must not contain whitespace or path separators",
            store::PREFIX_ENV
        ));
    }

    let mut title = String::new();
    let (mut description, mut design, mut acceptance) =
        (String::new(), String::new(), String::new());
    let (mut priority, mut issue_type) = ("2".to_string(), "task".to_string());
    let (mut external_ref, mut parent, mut tags) = (String::new(), String::new(), String::new());
    let mut assignee = None;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let target = match arg.as_str() {
            "-d" | "--description" => &mut description,
            "--design" => &mut design,
            "--acceptance" => &mut acceptance,
            "-p" | "--priority" => &mut priority,
            "-t" | "--type" => &mut issue_type,
            "-a" | "--assignee" => assignee.insert(String::new()),
            "--external-ref" => &mut external_ref,
            "--parent" => &mut parent,
            "--tags" => &mut tags,
            s if s.starts_with('-') => return Err(format!("Unknown option: {s}")),
            _ => {
                title = arg.clone();
                continue;
            }
        };
        *target = it
            .next()
            .ok_or_else(|| format!("Error: option {arg} requires a value"))?
            .clone();
    }
    let assignee = assignee.unwrap_or_else(git_user_name);

    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Error: cannot create '{}': {e}", dir.display()))?;

    if !parent.is_empty() {
        parent = store::file_id(&store::resolve(dir, &parent)?);
    }
    if title.is_empty() {
        title = "Untitled".to_string();
    }

    let (id, file) = loop {
        let id = format!("{prefix}-{}", random_suffix(4));
        if store::exact_path(dir, &id).is_none() {
            let file = store::new_ticket_path(dir, &id);
            break (id, file);
        }
    };

    let mut t = String::new();
    let _ = writeln!(t, "---\nid: {id}\nstatus: open\ndeps: []\nlinks: []");
    let _ = writeln!(
        t,
        "created: {}\ntype: {issue_type}\npriority: {priority}",
        iso_now()
    );
    if !assignee.is_empty() {
        let _ = writeln!(t, "assignee: {assignee}");
    }
    if !external_ref.is_empty() {
        let _ = writeln!(t, "external-ref: {external_ref}");
    }
    if !parent.is_empty() {
        let _ = writeln!(t, "parent: {parent}");
    }
    if !tags.is_empty() {
        let _ = writeln!(t, "tags: [{}]", tags.replace(',', ", "));
    }
    let _ = writeln!(t, "---\n# {title}\n");
    if !description.is_empty() {
        let _ = writeln!(t, "{description}\n");
    }
    if !design.is_empty() {
        let _ = writeln!(t, "## Design\n\n{design}\n");
    }
    if !acceptance.is_empty() {
        let _ = writeln!(t, "## Acceptance Criteria\n\n{acceptance}\n");
    }
    store::write(&file, &t)?;
    emit(&format!("{id}\n"));
    Ok(())
}

fn set_status(dir: &Path, id: &str, status: &str) -> Res {
    if !VALID_STATUSES.contains(&status) {
        return Err(format!(
            "Error: invalid status '{status}'. Must be one of: {}",
            VALID_STATUSES.join(" ")
        ));
    }
    let file = store::resolve(dir, id)?;
    store::update_file_field(&file, "status", status)?;
    emit(&format!("Updated {} -> {status}\n", store::file_id(&file)));
    Ok(())
}

pub fn status(dir: &Path, args: &[String]) -> Res {
    match args {
        [id, status, ..] => set_status(dir, id, status),
        _ => Err(format!(
            "Usage: kk status <id> <status>\nValid statuses: {}",
            VALID_STATUSES.join(" ")
        )),
    }
}

/// `start`, `close` and `reopen`.
pub fn transition(dir: &Path, name: &str, status: &str, args: &[String]) -> Res {
    match args.first() {
        Some(id) => set_status(dir, id, status),
        None => usage(&format!("{name} <id>")),
    }
}

/// Read a list-valued frontmatter field from a ticket file.
fn read_list(path: &Path, field: &str) -> Result<Vec<String>, String> {
    let text = store::read(path)?;
    Ok(store::parse_list(
        &store::get_field(&text, field).unwrap_or_default(),
    ))
}

fn write_list(path: &Path, field: &str, items: &[String]) -> Res {
    store::update_file_field(path, field, &store::format_list(items))
}

pub fn dep(dir: &Path, args: &[String]) -> Res {
    match args.first().map(String::as_str) {
        Some("tree") => return dep_tree(dir, &args[1..]),
        Some("cycle") => {
            emit(&deps::cycles(&store::load_all(dir)));
            return Ok(());
        }
        _ => {}
    }
    let [id, dep_id, ..] = args else {
        return Err("Usage: kk dep <id> <dependency-id>\n       \
                    kk dep tree <id>  - show dependency tree\n       \
                    kk dep cycle      - find dependency cycles"
            .to_string());
    };
    let file = store::resolve(dir, id)?;
    let dep_id = store::file_id(&store::resolve(dir, dep_id)?);

    let mut current = read_list(&file, "deps")?;
    if current.contains(&dep_id) {
        emit("Dependency already exists\n");
        return Ok(());
    }
    current.push(dep_id.clone());
    write_list(&file, "deps", &current)?;
    emit(&format!(
        "Added dependency: {} -> {dep_id}\n",
        store::file_id(&file)
    ));
    Ok(())
}

fn dep_tree(dir: &Path, args: &[String]) -> Res {
    let mut full = false;
    let mut root = None;
    for arg in args {
        match arg.as_str() {
            "--full" => full = true,
            _ => root = Some(arg.trim()),
        }
    }
    let Some(root) = root.filter(|r| !r.is_empty()) else {
        return usage("dep tree [--full] <id>");
    };
    emit(&deps::tree(&store::load_all(dir), root, full)?);
    Ok(())
}

pub fn undep(dir: &Path, args: &[String]) -> Res {
    let [id, dep_id, ..] = args else {
        return usage("undep <id> <dependency-id>");
    };
    let file = store::resolve(dir, id)?;
    let dep_id = store::file_id(&store::resolve(dir, dep_id)?);

    let mut current = read_list(&file, "deps")?;
    if !current.contains(&dep_id) {
        return fail_quietly("Dependency not found");
    }
    current.retain(|d| *d != dep_id);
    write_list(&file, "deps", &current)?;
    emit(&format!(
        "Removed dependency: {} -/-> {dep_id}\n",
        store::file_id(&file)
    ));
    Ok(())
}

pub fn link(dir: &Path, args: &[String]) -> Res {
    if args.len() < 2 {
        return usage("link <id> <id> [id...]");
    }
    let files = args
        .iter()
        .map(|a| store::resolve(dir, a))
        .collect::<Result<Vec<_>, _>>()?;
    let ids: Vec<String> = files.iter().map(|f| store::file_id(f)).collect();

    let mut added = 0;
    for (file, own_id) in files.iter().zip(&ids) {
        let mut links = read_list(file, "links")?;
        let before = links.len();
        for other in ids.iter().filter(|other| *other != own_id) {
            if !links.contains(other) {
                links.push(other.clone());
            }
        }
        if links.len() > before {
            added += links.len() - before;
            write_list(file, "links", &links)?;
        }
    }

    if added == 0 {
        emit("All links already exist\n");
    } else {
        emit(&format!(
            "Added {added} link(s) between {} tickets\n",
            ids.len()
        ));
    }
    Ok(())
}

pub fn unlink(dir: &Path, args: &[String]) -> Res {
    let [id, target, ..] = args else {
        return usage("unlink <id> <target-id>");
    };
    let file = store::resolve(dir, id)?;
    let target_file = store::resolve(dir, target)?;
    let (id, target_id) = (store::file_id(&file), store::file_id(&target_file));

    if !read_list(&file, "links")?.contains(&target_id) {
        return fail_quietly("Link not found");
    }
    for (path, other) in [(&file, &target_id), (&target_file, &id)] {
        let mut links = read_list(path, "links")?;
        if links.contains(other) {
            links.retain(|l| l != other);
            write_list(path, "links", &links)?;
        }
    }
    emit(&format!("Removed link: {id} <-> {target_id}\n"));
    Ok(())
}

/// Filters shared by the listing commands.
#[derive(Default)]
struct Filters {
    status: Option<String>,
    assignee: Option<String>,
    tag: Option<String>,
    limit: Option<String>,
}

impl Filters {
    /// Accepts `-a X`, `-T X` and `--status|--assignee|--tag|--limit` as
    /// either `--name=X` or `--name X`. Anything else is ignored.
    fn parse(args: &[String]) -> Self {
        let mut f = Filters::default();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
                _ => (arg.as_str(), None),
            };
            let slot = match name {
                "--status" => &mut f.status,
                "-a" | "--assignee" => &mut f.assignee,
                "-T" | "--tag" => &mut f.tag,
                "--limit" => &mut f.limit,
                _ => continue,
            };
            *slot = inline
                .or_else(|| it.next().cloned())
                .filter(|v| !v.is_empty());
        }
        f
    }

    fn matches(&self, t: &Ticket) -> bool {
        !t.id.is_empty()
            && self.status.as_ref().is_none_or(|s| *s == t.status)
            && self.assignee.as_ref().is_none_or(|a| *a == t.assignee)
            && self.tag.as_ref().is_none_or(|tag| t.tags.contains(tag))
    }
}

fn is_active(t: &Ticket) -> bool {
    t.status == "open" || t.status == "in_progress"
}

fn priority_of(t: &Ticket) -> &str {
    if t.priority.is_empty() {
        "2"
    } else {
        &t.priority
    }
}

/// Dependencies of `t` that are not closed (including unknown tickets).
fn open_deps<'a>(t: &'a Ticket, statuses: &HashMap<&str, &str>) -> Vec<&'a str> {
    t.deps
        .iter()
        .map(String::as_str)
        .filter(|d| statuses.get(d).copied() != Some("closed"))
        .collect()
}

/// Active tickets matching the filters, sorted by priority then ID, each
/// paired with its unresolved dependencies.
fn active_with_blockers<'a>(
    tickets: &'a [Ticket],
    filters: &Filters,
) -> Vec<(&'a Ticket, Vec<&'a str>)> {
    let statuses: HashMap<&str, &str> = tickets
        .iter()
        .map(|t| (t.id.as_str(), t.status.as_str()))
        .collect();
    let mut rows: Vec<_> = tickets
        .iter()
        .filter(|t| is_active(t) && filters.matches(t))
        .map(|t| (t, open_deps(t, &statuses)))
        .collect();
    rows.sort_by(|(a, _), (b, _)| {
        let (pa, pb) = (priority_of(a), priority_of(b));
        match (pa.parse::<f64>(), pb.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.total_cmp(&y),
            _ => pa.cmp(pb),
        }
        .then_with(|| a.id.cmp(&b.id))
    });
    rows
}

pub fn ready(dir: &Path, args: &[String]) -> Res {
    let tickets = store::load_all(dir);
    let mut out = String::new();
    for (t, blockers) in active_with_blockers(&tickets, &Filters::parse(args)) {
        if blockers.is_empty() {
            let _ = writeln!(
                out,
                "{:<8} [P{}][{}] - {}",
                t.id,
                priority_of(t),
                t.status,
                t.title
            );
        }
    }
    emit(&out);
    Ok(())
}

pub fn blocked(dir: &Path, args: &[String]) -> Res {
    let tickets = store::load_all(dir);
    let mut out = String::new();
    for (t, blockers) in active_with_blockers(&tickets, &Filters::parse(args)) {
        if !blockers.is_empty() {
            let _ = writeln!(
                out,
                "{:<8} [P{}][{}] - {} <- [{}]",
                t.id,
                priority_of(t),
                t.status,
                t.title,
                blockers.join(", ")
            );
        }
    }
    emit(&out);
    Ok(())
}

pub fn closed(dir: &Path, args: &[String]) -> Res {
    let filters = Filters::parse(args);
    let limit = match &filters.limit {
        Some(l) => l
            .parse::<usize>()
            .map_err(|_| format!("Error: invalid limit '{l}'"))?,
        None => 20,
    };
    let mut tickets = store::load_all(dir);
    // Most recently modified first; only the 100 newest files are considered.
    tickets.sort_by_key(|t| std::cmp::Reverse(t.mtime));
    let mut out = String::new();
    for t in tickets
        .iter()
        .take(100)
        .filter(|t| (t.status == "closed" || t.status == "done") && filters.matches(t))
        .take(limit)
    {
        let _ = writeln!(out, "{:<8} [{}] - {}", t.id, t.status, t.title);
    }
    emit(&out);
    Ok(())
}

pub fn list(dir: &Path, args: &[String]) -> Res {
    let filters = Filters::parse(args);
    let mut out = String::new();
    for t in store::load_all(dir).iter().filter(|t| filters.matches(t)) {
        let _ = write!(out, "{:<8} [{}] - {}", t.id, t.status, t.title);
        if !t.deps.is_empty() {
            let _ = write!(out, " <- [{}]", t.deps.join(", "));
        }
        out.push('\n');
    }
    emit(&out);
    Ok(())
}

fn render_show(dir: &Path, file: &Path) -> Result<String, String> {
    let target = store::file_id(file);
    let tickets = store::load_all(dir);
    let by_id: HashMap<&str, &Ticket> = tickets
        .iter()
        .filter(|t| !t.id.is_empty())
        .map(|t| (t.id.as_str(), t))
        .collect();
    let describe = |id: &str| {
        let (status, title) = by_id
            .get(id)
            .map(|t| (t.status.as_str(), t.title.as_str()))
            .unwrap_or_default();
        format!("- {id} [{status}] {title}\n")
    };

    // The ticket itself, with the parent's title added as a comment.
    let mut out = String::new();
    let mut in_front = false;
    for line in store::read(file)?.lines() {
        out.push_str(line);
        if line == "---" {
            in_front = !in_front;
        } else if in_front {
            if let Some(parent) = line.strip_prefix("parent:") {
                if let Some(p) = by_id.get(parent.trim()) {
                    let _ = write!(out, "  # {}", p.title);
                }
            }
        }
        out.push('\n');
    }

    let mut section = |name: &str, ids: Vec<&str>| {
        if !ids.is_empty() {
            let _ = write!(out, "\n## {name}\n\n");
            ids.iter().for_each(|id| out.push_str(&describe(id)));
        }
    };
    let is_closed = |id: &str| by_id.get(id).is_some_and(|t| t.status == "closed");
    let me = by_id.get(target.as_str());

    let blockers = me.map(|t| {
        t.deps
            .iter()
            .map(String::as_str)
            .filter(|d| !is_closed(d))
            .collect()
    });
    section("Blockers", blockers.unwrap_or_default());
    let blocking = tickets
        .iter()
        .filter(|t| !t.id.is_empty() && t.status != "closed" && t.deps.contains(&target))
        .map(|t| t.id.as_str())
        .collect();
    section("Blocking", blocking);
    let children = tickets
        .iter()
        .filter(|t| !t.id.is_empty() && t.parent == target)
        .map(|t| t.id.as_str())
        .collect();
    section("Children", children);
    let linked = me.map(|t| t.links.iter().map(String::as_str).collect());
    section("Linked", linked.unwrap_or_default());
    Ok(out)
}

pub fn show(dir: &Path, args: &[String]) -> Res {
    let Some(id) = args.first() else {
        return usage("show <id>");
    };
    let file = store::resolve(dir, id)?;
    let out = render_show(dir, &file)?;

    let pager = std::env::var("KK_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .unwrap_or_default();
    let mut pager_cmd = pager.split_whitespace();
    if let (true, Some(program)) = (std::io::stdout().is_terminal(), pager_cmd.next()) {
        let child = Command::new(program)
            .args(pager_cmd)
            .stdin(Stdio::piped())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(out.as_bytes());
            }
            let _ = child.wait();
            return Ok(());
        }
    }
    emit(&out);
    Ok(())
}

pub fn add_note(dir: &Path, args: &[String]) -> Res {
    let Some(id) = args.first() else {
        return usage("add-note <id> [note text]");
    };
    let file = store::resolve(dir, id)?;

    let note = if args.len() > 1 {
        args[1..].join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("Error: cannot read note from stdin: {e}"))?;
        buf.trim_end_matches('\n').to_string()
    } else {
        return Err("Error: no note provided".to_string());
    };

    let mut text = store::read(&file)?;
    if !text.lines().any(|l| l.starts_with("## Notes")) {
        text.push_str("\n## Notes\n");
    }
    let _ = write!(text, "\n**{}**\n\n{note}\n", iso_now());
    store::write(&file, &text)?;
    emit(&format!("Note added to {}\n", store::file_id(&file)));
    Ok(())
}

pub fn edit(dir: &Path, args: &[String]) -> Res {
    let Some(id) = args.first() else {
        return usage("edit <id>");
    };
    let file = store::resolve(dir, id)?;
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        let editor = std::env::var("EDITOR").unwrap_or_default();
        let mut editor_cmd = editor.split_whitespace();
        let program = editor_cmd.next().unwrap_or("vi");
        let status = Command::new(program)
            .args(editor_cmd)
            .arg(&file)
            .status()
            .map_err(|e| format!("Error: cannot run editor '{program}': {e}"))?;
        if !status.success() {
            return Err(String::new());
        }
    } else {
        emit(&format!("Edit ticket file: {}\n", file.display()));
    }
    Ok(())
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn ticket_json(t: &Ticket) -> String {
    let fields: Vec<String> = t
        .fields
        .iter()
        .map(|(key, value)| {
            let json_value = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                Some(inner) => {
                    let items: Vec<String> = inner
                        .split(',')
                        .map(str::trim)
                        .filter(|i| !i.is_empty())
                        .map(json_string)
                        .collect();
                    format!("[{}]", items.join(","))
                }
                None => json_string(value),
            };
            format!("{}:{json_value}", json_string(key))
        })
        .collect();
    format!("{{{}}}\n", fields.join(","))
}

pub fn query(dir: &Path, args: &[String]) -> Res {
    let json: String = store::load_all(dir)
        .iter()
        .filter(|t| !t.fields.is_empty())
        .map(ticket_json)
        .collect();

    let Some(filter) = args.first().filter(|f| !f.is_empty()) else {
        emit(&json);
        return Ok(());
    };
    let mut child = Command::new("jq")
        .args(["-c", &format!("select({filter})")])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Error: jq is required for query filters ({e})"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(json.as_bytes());
    }
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        _ => Err(String::new()),
    }
}
