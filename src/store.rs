//! Ticket storage: markdown files with YAML frontmatter in a flat directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Environment variable naming the directory tickets are stored in.
pub const DIR_ENV: &str = "KK_TICKETS_DIR";
/// Environment variable naming the prefix used for new ticket IDs.
pub const PREFIX_ENV: &str = "KK_PREFIX";

#[derive(Debug, Default, Clone)]
pub struct Ticket {
    pub id: String,
    pub status: String,
    pub title: String,
    pub deps: Vec<String>,
    pub links: Vec<String>,
    pub parent: String,
    pub priority: String,
    pub assignee: String,
    pub tags: Vec<String>,
    /// All frontmatter fields in file order (used by `query`).
    pub fields: Vec<(String, String)>,
    pub mtime: Option<SystemTime>,
}

/// Parse `[a, b, c]` into its items.
pub fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.replace(['[', ']', ' '], ""))
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn format_list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

/// Split a frontmatter line into `(key, value)`.
fn split_field(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key, value.trim()))
}

pub fn parse_ticket(text: &str) -> Ticket {
    let mut t = Ticket::default();
    let mut in_front = false;
    for line in text.lines() {
        if line == "---" {
            in_front = !in_front;
            continue;
        }
        if in_front {
            let Some((key, value)) = split_field(line) else {
                continue;
            };
            match key {
                "id" => t.id = value.to_string(),
                "status" => t.status = value.to_string(),
                "parent" => t.parent = value.to_string(),
                "priority" => t.priority = value.to_string(),
                "assignee" => t.assignee = value.to_string(),
                "deps" => t.deps = parse_list(value),
                "links" => t.links = parse_list(value),
                "tags" => t.tags = parse_list(value),
                _ => {}
            }
            if key.starts_with(|c: char| c.is_ascii_alphabetic()) {
                t.fields.push((key.to_string(), value.to_string()));
            }
        } else if t.title.is_empty() {
            if let Some(title) = line.strip_prefix("# ") {
                t.title = title.to_string();
            }
        }
    }
    t
}

/// Paths of all ticket files, sorted by name.
fn ticket_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md") && p.is_file())
        .collect();
    files.sort();
    files
}

/// Load every ticket in the directory, sorted by file name.
pub fn load_all(dir: &Path) -> Vec<Ticket> {
    ticket_files(dir)
        .into_iter()
        .filter_map(|path| {
            let text = fs::read_to_string(&path).ok()?;
            let mut t = parse_ticket(&text);
            t.mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
            Some(t)
        })
        .collect()
}

/// The ticket ID implied by a file path (file name without `.md`).
pub fn file_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Resolve a full or partial ticket ID to its file.
pub fn resolve(dir: &Path, id: &str) -> Result<PathBuf, String> {
    let id = id.trim();
    let exact = dir.join(format!("{id}.md"));
    if exact.is_file() {
        return Ok(exact);
    }
    let mut matches = ticket_files(dir)
        .into_iter()
        .filter(|p| file_id(p).contains(id));
    match (matches.next(), matches.next()) {
        (Some(path), None) => Ok(path),
        (Some(_), Some(_)) => Err(format!(
            "Error: ambiguous ID '{id}' matches multiple tickets"
        )),
        _ => Err(format!("Error: ticket '{id}' not found")),
    }
}

/// Line indices `(start, end)` of the `---` lines delimiting the frontmatter.
fn frontmatter_bounds(lines: &[&str]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| *l == "---")?;
    let end = lines[start + 1..].iter().position(|l| *l == "---")? + start + 1;
    Some((start, end))
}

fn is_field(line: &str, field: &str) -> bool {
    line.strip_prefix(field).is_some_and(|r| r.starts_with(':'))
}

/// Read a frontmatter field's raw value.
pub fn get_field(text: &str, field: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let (start, end) = frontmatter_bounds(&lines)?;
    lines[start + 1..end]
        .iter()
        .find(|l| is_field(l, field))
        .map(|l| l[field.len() + 1..].trim().to_string())
}

/// Set a frontmatter field, adding it if missing. Returns the new file text.
pub fn set_field(text: &str, field: &str, value: &str) -> Result<String, String> {
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (start, end) = frontmatter_bounds(&refs).ok_or("Error: ticket file has no frontmatter")?;
    let new_line = format!("{field}: {value}");
    match (start + 1..end).find(|&i| is_field(&lines[i], field)) {
        Some(i) => lines[i] = new_line,
        None => lines.insert(start + 1, new_line),
    }
    Ok(lines.join("\n"))
}

/// Update one frontmatter field of a ticket file in place.
pub fn update_file_field(path: &Path, field: &str, value: &str) -> Result<(), String> {
    let text = read(path)?;
    write(path, &set_field(&text, field, value)?)
}

pub fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("Error: cannot read {}: {e}", path.display()))
}

pub fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("Error: cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nid: ab-1234\nstatus: open\ndeps: [ab-1, ab-2]\nlinks: []\n---\n# Hello: world\n\nstatus: not a field\n";

    #[test]
    fn parses_ticket() {
        let t = parse_ticket(SAMPLE);
        assert_eq!(t.id, "ab-1234");
        assert_eq!(t.status, "open");
        assert_eq!(t.deps, vec!["ab-1", "ab-2"]);
        assert!(t.links.is_empty());
        assert_eq!(t.title, "Hello: world");
    }

    #[test]
    fn sets_existing_field_only_in_frontmatter() {
        let out = set_field(SAMPLE, "status", "closed").unwrap();
        assert!(out.contains("\nstatus: closed\n"));
        assert!(out.ends_with("status: not a field\n"));
        assert_eq!(get_field(&out, "status").as_deref(), Some("closed"));
    }

    #[test]
    fn inserts_missing_field() {
        let out = set_field(SAMPLE, "assignee", "Sam").unwrap();
        assert!(out.starts_with("---\nassignee: Sam\nid: ab-1234\n"));
    }
}
