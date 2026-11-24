//! kk (KayKay) - minimal ticket system with dependency tracking.
//!
//! Tickets are markdown files with YAML frontmatter, stored in the directory
//! named by `KK_TICKETS_DIR`. New ticket IDs are prefixed with `KK_PREFIX`.

mod cmd;
mod deps;
mod plugin;
mod store;
mod util;

use std::path::PathBuf;
use std::process::ExitCode;

/// Command result. An empty error string means "fail without a message".
pub type Res = Result<(), String>;

fn help_text() -> String {
    let mut help = String::from(
        "\
kk - minimal ticket system with dependency tracking

Usage: kk <command> [args]

Commands:
  create [title] [options] Create ticket, prints ID
    -d, --description      Description text
    --design               Design notes
    --acceptance           Acceptance criteria
    -t, --type             Type (bug|feature|task|epic|chore) [default: task]
    -p, --priority         Priority 0-4, 0=highest [default: 2]
    -a, --assignee         Assignee [default: git user.name]
    --external-ref         External reference (e.g., gh-123, JIRA-456)
    --parent               Parent ticket ID
    --tags                 Comma-separated tags (e.g., --tags ui,backend,urgent)
  start <id>               Set status to in_progress
  close <id>               Set status to closed
  reopen <id>              Set status to open
  status <id> <status>     Update status (open|in_progress|closed)
  dep <id> <dep-id>        Add dependency (id depends on dep-id)
  dep tree [--full] <id>   Show dependency tree (--full disables dedup)
  dep cycle                Find dependency cycles in open tickets
  undep <id> <dep-id>      Remove dependency
  link <id> <id> [id...]   Link tickets together (symmetric)
  unlink <id> <target-id>  Remove link between tickets
  ls|list [--status=X] [-a X] [-T X]   List tickets
  ready [-a X] [-T X]      List open/in-progress tickets with deps resolved
  blocked [-a X] [-T X]    List open/in-progress tickets with unresolved deps
  closed [--limit=N] [-a X] [-T X] List recently closed tickets (default 20, by mtime)
  show <id>                Display ticket
  edit <id>                Open ticket in $EDITOR
  add-note <id> [text]     Append timestamped note (or pipe via stdin)
  query [jq-filter]        Output tickets as JSON, optionally filtered (requires jq)
  super <cmd> [args]       Bypass plugins, run built-in command directly
",
    );

    let plugins = plugin::list();
    if !plugins.is_empty() {
        help.push_str("\nPlugins (kk-<cmd> in PATH):\n");
        for (name, description) in plugins {
            help.push_str(&format!("  {name:<22} {description}\n"));
        }
    }

    help.push_str(&format!(
        "
Use 'super' to bypass plugins. Plugins receive {dir} and KK_SCRIPT
env vars; use '$KK_SCRIPT super <cmd>' to call built-ins.

Plugin descriptions: comment '# kk-plugin: text' or --kk-describe flag

Environment:
  {dir:<22} Directory tickets are stored in (required)
  {prefix:<22} Prefix for new ticket IDs (required by 'create')
  KK_PAGER               Pager for 'show' on a terminal (falls back to PAGER)

Tickets are stored as markdown files in ${dir}
Supports partial ID matching (e.g., 'kk show 5c4' matches 'nw-5c46')
",
        dir = store::DIR_ENV,
        prefix = store::PREFIX_ENV,
    ));
    help
}

/// The tickets directory from the environment, if set.
fn tickets_dir_env() -> Option<PathBuf> {
    std::env::var_os(store::DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn missing_dir(dir: &std::path::Path) -> String {
    format!(
        "Error: tickets directory '{}' does not exist",
        dir.display()
    )
}

/// Resolve the tickets directory for a built-in command.
fn tickets_dir(command: &str) -> Result<PathBuf, String> {
    let dir = tickets_dir_env().ok_or_else(|| {
        format!(
            "Error: {env} is not set\nSet it to the directory where tickets are stored \
             (e.g. export {env}=~/tickets)",
            env = store::DIR_ENV
        )
    })?;
    // Only `create` may initialize the directory.
    if command != "create" && !dir.is_dir() {
        return Err(missing_dir(&dir));
    }
    Ok(dir)
}

fn run(mut args: Vec<String>) -> Res {
    let bypass_plugins = args.first().is_some_and(|a| a == "super");
    if bypass_plugins {
        args.remove(0);
    }
    let command = args.first().cloned().unwrap_or_else(|| "help".to_string());
    let rest = args.get(1..).unwrap_or_default();

    if matches!(command.as_str(), "help" | "--help" | "-h") {
        util::emit(&help_text());
        return Ok(());
    }

    if !bypass_plugins {
        if let Some(path) = plugin::find(&command) {
            if let Some(dir) = tickets_dir_env().filter(|d| !d.is_dir()) {
                return Err(missing_dir(&dir));
            }
            return plugin::run(&path, rest);
        }
    }

    const BUILTINS: &[&str] = &[
        "create", "start", "close", "reopen", "status", "dep", "undep", "link", "unlink", "ls",
        "list", "ready", "blocked", "closed", "show", "edit", "add-note", "query",
    ];
    if !BUILTINS.contains(&command.as_str()) {
        return Err(format!(
            "Unknown command: {command}\n{}",
            help_text().trim_end()
        ));
    }

    let dir = &tickets_dir(&command)?;
    match command.as_str() {
        "create" => cmd::create(dir, rest),
        "start" => cmd::transition(dir, "start", "in_progress", rest),
        "close" => cmd::transition(dir, "close", "closed", rest),
        "reopen" => cmd::transition(dir, "reopen", "open", rest),
        "status" => cmd::status(dir, rest),
        "dep" => cmd::dep(dir, rest),
        "undep" => cmd::undep(dir, rest),
        "link" => cmd::link(dir, rest),
        "unlink" => cmd::unlink(dir, rest),
        "ls" | "list" => cmd::list(dir, rest),
        "ready" => cmd::ready(dir, rest),
        "blocked" => cmd::blocked(dir, rest),
        "closed" => cmd::closed(dir, rest),
        "show" => cmd::show(dir, rest),
        "edit" => cmd::edit(dir, rest),
        "add-note" => cmd::add_note(dir, rest),
        "query" => cmd::query(dir, rest),
        _ => unreachable!(),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            ExitCode::FAILURE
        }
    }
}
