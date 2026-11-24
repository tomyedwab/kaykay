//! End-to-end tests that drive the `kk` binary.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch area with a tickets directory, removed on drop.
struct Env {
    root: PathBuf,
    tickets: PathBuf,
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl From<Output> for Run {
    fn from(o: Output) -> Self {
        Run {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }
}

impl Env {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "kk-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let tickets = root.join("tickets");
        Env { root, tickets }
    }

    /// A command with no kk configuration in its environment.
    fn bare(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_kk"));
        c.current_dir(&self.root)
            .env_remove("KK_TICKETS_DIR")
            .env_remove("KK_PREFIX")
            .env_remove("KK_PAGER")
            .env_remove("PAGER");
        c
    }

    fn command(&self) -> Command {
        let mut c = self.bare();
        c.env("KK_TICKETS_DIR", &self.tickets)
            .env("KK_PREFIX", "tst");
        c
    }

    fn kk(&self, args: &[&str]) -> Run {
        self.command().args(args).output().unwrap().into()
    }

    /// Run a command that is expected to succeed and return its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let run = self.kk(args);
        assert!(run.ok, "kk {args:?} failed: {}{}", run.stdout, run.stderr);
        run.stdout
    }

    fn create(&self, args: &[&str]) -> String {
        let mut full = vec!["create"];
        full.extend(args);
        self.ok(&full).trim().to_string()
    }

    /// Write a ticket file directly, for tests needing a known ID.
    fn ticket(&self, id: &str, title: &str, extra: &str) {
        fs::create_dir_all(&self.tickets).unwrap();
        // Defaults, unless `extra` supplies the field itself.
        let defaults: String = [
            "status: open",
            "deps: []",
            "links: []",
            "created: 2026-01-01T00:00:00Z",
            "type: task",
            "priority: 2",
        ]
        .iter()
        .filter(|d| {
            let key = &d[..=d.find(':').unwrap()];
            !extra.lines().any(|l| l.starts_with(key))
        })
        .map(|d| format!("{d}\n"))
        .collect();
        let text = format!("---\nid: {id}\n{defaults}{extra}---\n# {title}\n\n");
        fs::write(self.file(id), text).unwrap();
    }

    fn file(&self, id: &str) -> PathBuf {
        self.tickets.join(format!("{id}.kk"))
    }

    fn read(&self, id: &str) -> String {
        fs::read_to_string(self.file(id)).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn is_ticket_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hash| {
            hash.len() == 4
                && hash
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

// --- Storage location and ID prefix come only from the environment ---

#[test]
fn requires_tickets_dir_env_and_ignores_cwd() {
    let env = Env::new();
    // A .tickets directory in the working directory must not be picked up.
    fs::create_dir_all(env.root.join(".tickets")).unwrap();
    for args in [&["ls"][..], &["create", "Nope"], &["ready"]] {
        let run: Run = env
            .bare()
            .env("KK_PREFIX", "tst")
            .args(args)
            .output()
            .unwrap()
            .into();
        assert!(!run.ok);
        assert!(
            run.stderr.contains("KK_TICKETS_DIR is not set"),
            "{}",
            run.stderr
        );
    }
    assert_eq!(fs::read_dir(env.root.join(".tickets")).unwrap().count(), 0);
}

#[test]
fn create_requires_prefix_env() {
    let env = Env::new();
    let run: Run = env
        .bare()
        .env("KK_TICKETS_DIR", &env.tickets)
        .args(["create", "No prefix"])
        .output()
        .unwrap()
        .into();
    assert!(!run.ok);
    assert!(
        run.stderr.contains("KK_PREFIX is not set"),
        "{}",
        run.stderr
    );
    assert!(!env.tickets.exists());
}

#[test]
fn prefix_comes_from_env_not_cwd() {
    let env = Env::new();
    let cwd = env.root.join("my-cool-project");
    fs::create_dir_all(&cwd).unwrap();
    let run: Run = env
        .command()
        .current_dir(&cwd)
        .env("KK_PREFIX", "abc")
        .args(["create", "Prefixed"])
        .output()
        .unwrap()
        .into();
    assert!(run.ok, "{}", run.stderr);
    let id = run.stdout.trim();
    assert!(is_ticket_id(id, "abc"), "unexpected id {id}");
    assert!(env.file(id).is_file());
    assert!(!cwd.join(".tickets").exists());
}

#[test]
fn create_initializes_directory_but_reads_do_not() {
    let env = Env::new();
    let run = env.kk(&["ls"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("does not exist"), "{}", run.stderr);
    assert!(!env.tickets.exists());

    env.create(&["First"]);
    assert!(env.tickets.is_dir());
    assert!(env.kk(&["ls"]).ok);
}

#[test]
fn help_works_without_configuration() {
    let env = Env::new();
    for args in [&["help"][..], &["--help"], &["-h"], &[]] {
        let run: Run = env.bare().args(args).output().unwrap().into();
        assert!(run.ok);
        assert!(run.stdout.contains("minimal ticket system"));
        assert!(run.stdout.contains("KK_TICKETS_DIR"));
    }
}

#[test]
fn unknown_command_fails_with_help() {
    let env = Env::new();
    let run = env.kk(&["frobnicate"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("Unknown command: frobnicate"));
    assert!(run.stderr.contains("Usage: kk"));
}

// --- File extensions ---

#[test]
fn new_tickets_use_kk_extension() {
    let env = Env::new();
    let id = env.create(&["Fresh"]);
    let names: Vec<_> = fs::read_dir(&env.tickets)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [format!("{id}.kk")]);
}

#[test]
fn legacy_md_tickets_are_read_and_updated_in_place() {
    let env = Env::new();
    env.ticket("tst-0002", "New style", "");
    let md = env.tickets.join("tst-0001.md");
    fs::rename(env.file("tst-0002"), &md).unwrap();
    fs::write(
        &md,
        fs::read_to_string(&md)
            .unwrap()
            .replace("tst-0002", "tst-0001"),
    )
    .unwrap();
    env.ticket("tst-0002", "New style", "");
    fs::write(env.tickets.join("notes.txt"), "---\nid: tst-9999\n---\n").unwrap();

    assert_eq!(
        env.ok(&["ls"]),
        "tst-0001 [open] - New style\ntst-0002 [open] - New style\n"
    );
    assert!(env.ok(&["show", "0001"]).starts_with("---\nid: tst-0001\n"));

    // Updates stay in the .md file rather than creating a .kk copy.
    env.ok(&["dep", "tst-0002", "tst-0001"]);
    env.ok(&["link", "tst-0001", "tst-0002"]);
    env.ok(&["close", "0001"]);
    env.ok(&["add-note", "0001", "legacy note"]);
    assert!(!env.file("tst-0001").exists());
    let text = fs::read_to_string(&md).unwrap();
    assert!(text.contains("status: closed\n") && text.contains("links: [tst-0002]\n"));
    assert!(text.ends_with("legacy note\n"));
    assert!(env.read("tst-0002").contains("deps: [tst-0001]\n"));
    assert_eq!(env.ok(&["ready"]), "tst-0002 [P2][open] - New style\n");
    assert_eq!(
        env.ok(&["edit", "0001"]),
        format!("Edit ticket file: {}\n", md.display())
    );

    // A child of a legacy ticket is a new .kk file.
    let child = env.create(&["Child", "--parent", "0001"]);
    assert!(env.read(&child).contains("parent: tst-0001\n"));
}

#[test]
fn kk_file_wins_when_both_extensions_exist() {
    let env = Env::new();
    env.ticket("tst-0001", "Current", "");
    let legacy = env.read("tst-0001").replace("# Current", "# Stale");
    fs::write(env.tickets.join("tst-0001.md"), legacy).unwrap();

    assert_eq!(env.ok(&["ls"]), "tst-0001 [open] - Current\n");
    assert!(env.ok(&["show", "0001"]).contains("# Current"));
    env.ok(&["close", "0001"]);
    assert!(env.read("tst-0001").contains("status: closed\n"));
    assert!(fs::read_to_string(env.tickets.join("tst-0001.md"))
        .unwrap()
        .contains("status: open\n"));
}

// --- Ticket format ---

#[test]
fn create_writes_expected_format() {
    let env = Env::new();
    let parent = env.create(&["Parent"]);
    let id = env.create(&[
        "My title",
        "-d",
        "Some description",
        "--design",
        "The design",
        "--acceptance",
        "It works",
        "-t",
        "bug",
        "-p",
        "1",
        "-a",
        "Sam",
        "--external-ref",
        "gh-12",
        "--parent",
        &parent[4..],
        "--tags",
        "ui,backend",
    ]);
    assert!(is_ticket_id(&id, "tst"));
    let text = env.read(&id);
    let created = text.lines().find(|l| l.starts_with("created: ")).unwrap();
    let stamp = created.trim_start_matches("created: ");
    assert_eq!(stamp.len(), 20, "{stamp}");
    assert!(stamp.ends_with('Z') && stamp.as_bytes()[10] == b'T');
    let expected = format!(
        "---\nid: {id}\nstatus: open\ndeps: []\nlinks: []\n{created}\ntype: bug\npriority: 1\n\
         assignee: Sam\nexternal-ref: gh-12\nparent: {parent}\ntags: [ui, backend]\n---\n\
         # My title\n\nSome description\n\n## Design\n\nThe design\n\n\
         ## Acceptance Criteria\n\nIt works\n\n"
    );
    assert_eq!(text, expected);
}

#[test]
fn create_defaults_and_errors() {
    let env = Env::new();
    let id = env.create(&["-a", "Sam"]);
    let text = env.read(&id);
    assert!(text.contains("# Untitled\n"));
    assert!(text.contains("type: task\npriority: 2\n"));

    let run = env.kk(&["create", "X", "--bogus"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("Unknown option: --bogus"));

    let run = env.kk(&["create", "X", "--parent", "missing"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("ticket 'missing' not found"));
}

// --- Status ---

#[test]
fn status_transitions() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "");
    assert_eq!(
        env.ok(&["start", "tst-0001"]),
        "Updated tst-0001 -> in_progress\n"
    );
    assert!(env.read("tst-0001").contains("status: in_progress\n"));
    assert_eq!(
        env.ok(&["close", "tst-0001"]),
        "Updated tst-0001 -> closed\n"
    );
    assert_eq!(
        env.ok(&["reopen", "tst-0001"]),
        "Updated tst-0001 -> open\n"
    );
    assert_eq!(
        env.ok(&["status", "tst-0001", "closed"]),
        "Updated tst-0001 -> closed\n"
    );

    let run = env.kk(&["status", "tst-0001", "bogus"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("invalid status 'bogus'"));
    assert!(run.stderr.contains("open in_progress closed"));
    assert!(env.read("tst-0001").contains("status: closed\n"));

    assert!(!env.kk(&["start"]).ok);
    assert!(!env.kk(&["status", "tst-0001"]).ok);
}

#[test]
fn status_update_leaves_body_untouched() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "");
    let mut text = env.read("tst-0001");
    text.push_str("status: this is body text\n");
    fs::write(env.file("tst-0001"), &text).unwrap();
    env.ok(&["close", "tst-0001"]);
    let updated = env.read("tst-0001");
    assert!(updated.contains("status: closed\n"));
    assert!(updated.ends_with("status: this is body text\n"));
}

// --- ID resolution ---

#[test]
fn partial_id_resolution() {
    let env = Env::new();
    env.ticket("abc-1234", "First", "");
    env.ticket("abc-5678", "Second", "");
    env.ticket("abc-12", "Short", "");

    assert!(env.ok(&["show", "5678"]).contains("# Second"));
    assert!(env.ok(&["show", "c-56"]).contains("# Second"));
    assert!(env.ok(&["show", " 5678 "]).contains("# Second"));
    // An exact match wins over partial matches.
    assert!(env.ok(&["show", "abc-12"]).contains("# Short"));

    let run = env.kk(&["show", "abc"]);
    assert!(!run.ok);
    assert!(run
        .stderr
        .contains("ambiguous ID 'abc' matches multiple tickets"));

    let run = env.kk(&["show", "zzz"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("ticket 'zzz' not found"));
}

// --- Dependencies ---

#[test]
fn dep_and_undep() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "");
    env.ticket("tst-0002", "Two", "");
    env.ticket("tst-0003", "Three", "");

    assert_eq!(
        env.ok(&["dep", "0001", "0002"]),
        "Added dependency: tst-0001 -> tst-0002\n"
    );
    assert!(env.read("tst-0001").contains("deps: [tst-0002]\n"));
    env.ok(&["dep", "tst-0001", "tst-0003"]);
    assert!(env
        .read("tst-0001")
        .contains("deps: [tst-0002, tst-0003]\n"));
    assert_eq!(
        env.ok(&["dep", "tst-0001", "tst-0002"]),
        "Dependency already exists\n"
    );

    assert!(!env.kk(&["dep", "tst-0001", "nope"]).ok);
    assert!(!env.kk(&["dep", "tst-0001"]).ok);

    assert_eq!(
        env.ok(&["undep", "tst-0001", "tst-0002"]),
        "Removed dependency: tst-0001 -/-> tst-0002\n"
    );
    assert!(env.read("tst-0001").contains("deps: [tst-0003]\n"));
    let run = env.kk(&["undep", "tst-0001", "tst-0002"]);
    assert!(!run.ok);
    assert_eq!(run.stdout, "Dependency not found\n");
    env.ok(&["undep", "tst-0001", "tst-0003"]);
    assert!(env.read("tst-0001").contains("deps: []\n"));
}

#[test]
fn ready_and_blocked() {
    let env = Env::new();
    env.ticket("tst-0001", "Blocked one", "deps: [tst-0002, tst-0003]\n");
    env.ticket(
        "tst-0002",
        "Open dep",
        "priority: 0\nassignee: Sam\ntags: [ui, api]\n",
    );
    env.ticket("tst-0003", "Closed dep", "status: closed\n");
    env.ticket(
        "tst-0004",
        "In progress",
        "status: in_progress\npriority: 1\n",
    );

    assert_eq!(
        env.ok(&["ready"]),
        "tst-0002 [P0][open] - Open dep\ntst-0004 [P1][in_progress] - In progress\n"
    );
    assert_eq!(
        env.ok(&["blocked"]),
        "tst-0001 [P2][open] - Blocked one <- [tst-0002]\n"
    );
    assert_eq!(
        env.ok(&["ready", "-a", "Sam"]),
        "tst-0002 [P0][open] - Open dep\n"
    );
    assert_eq!(env.ok(&["ready", "--assignee=Nobody"]), "");
    assert_eq!(
        env.ok(&["ready", "-T", "api"]),
        "tst-0002 [P0][open] - Open dep\n"
    );
    assert_eq!(env.ok(&["ready", "--tag=ap"]), "");
    assert_eq!(env.ok(&["blocked", "-T", "ui"]), "");

    env.ok(&["close", "tst-0002"]);
    assert_eq!(env.ok(&["blocked"]), "");
    assert!(env
        .ok(&["ready"])
        .contains("tst-0001 [P2][open] - Blocked one\n"));
}

#[test]
fn dep_tree() {
    let env = Env::new();
    env.ticket("tst-000a", "Root", "deps: [tst-000b, tst-000c, tst-000e]\n");
    env.ticket("tst-000b", "B", "deps: [tst-000d]\n");
    env.ticket("tst-000c", "C", "deps: [tst-000d]\n");
    env.ticket("tst-000d", "D", "deps: [tst-000e]\n");
    env.ticket("tst-000e", "E", "status: closed\n");

    // Each ticket appears once, at its deepest position.
    assert_eq!(
        env.ok(&["dep", "tree", "000a"]),
        "tst-000a [open] Root\n\
         ├── tst-000b [open] B\n\
         │   └── tst-000d [open] D\n\
         │       └── tst-000e [closed] E\n\
         └── tst-000c [open] C\n"
    );
    assert_eq!(
        env.ok(&["dep", "tree", "--full", "tst-000a"]),
        "tst-000a [open] Root\n\
         ├── tst-000b [open] B\n\
         │   └── tst-000d [open] D\n\
         │       └── tst-000e [closed] E\n\
         ├── tst-000c [open] C\n\
         │   └── tst-000d [open] D\n\
         │       └── tst-000e [closed] E\n\
         └── tst-000e [closed] E\n"
    );

    let run = env.kk(&["dep", "tree", "tst-000"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("ambiguous ID tst-000"));
    assert!(!env.kk(&["dep", "tree", "zzz"]).ok);
    assert!(!env.kk(&["dep", "tree"]).ok);
}

#[test]
fn dep_cycle() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "deps: [tst-0002]\n");
    env.ticket("tst-0002", "Two", "deps: [tst-0003]\n");
    env.ticket("tst-0003", "Three", "");
    assert_eq!(env.ok(&["dep", "cycle"]), "No dependency cycles found\n");

    env.ok(&["dep", "tst-0003", "tst-0001"]);
    assert_eq!(
        env.ok(&["dep", "cycle"]),
        "Cycle 1: tst-0001 -> tst-0002 -> tst-0003 -> tst-0001\n  \
         tst-0001 [open] One\n  tst-0002 [open] Two\n  tst-0003 [open] Three\n"
    );
    // A cycle through a tree is still a tree, and closed tickets break cycles.
    assert!(env
        .ok(&["dep", "tree", "tst-0001"])
        .ends_with("└── tst-0003 [open] Three\n"));
    env.ok(&["close", "tst-0002"]);
    assert_eq!(env.ok(&["dep", "cycle"]), "No dependency cycles found\n");
}

// --- Links ---

#[test]
fn link_and_unlink() {
    let env = Env::new();
    for (id, title) in [
        ("tst-0001", "One"),
        ("tst-0002", "Two"),
        ("tst-0003", "Three"),
    ] {
        env.ticket(id, title, "");
    }
    assert_eq!(
        env.ok(&["link", "0001", "0002", "0003"]),
        "Added 6 link(s) between 3 tickets\n"
    );
    assert!(env
        .read("tst-0001")
        .contains("links: [tst-0002, tst-0003]\n"));
    assert!(env
        .read("tst-0003")
        .contains("links: [tst-0001, tst-0002]\n"));
    assert_eq!(
        env.ok(&["link", "tst-0001", "tst-0002"]),
        "All links already exist\n"
    );

    assert_eq!(
        env.ok(&["unlink", "tst-0001", "tst-0002"]),
        "Removed link: tst-0001 <-> tst-0002\n"
    );
    assert!(env.read("tst-0001").contains("links: [tst-0003]\n"));
    assert!(env.read("tst-0002").contains("links: [tst-0003]\n"));
    let run = env.kk(&["unlink", "tst-0001", "tst-0002"]);
    assert!(!run.ok);
    assert_eq!(run.stdout, "Link not found\n");
    assert!(!env.kk(&["link", "tst-0001"]).ok);
    assert!(!env.kk(&["link", "tst-0001", "nope"]).ok);
}

// --- Listing ---

#[test]
fn ls_and_filters() {
    let env = Env::new();
    env.ticket(
        "tst-0001",
        "One",
        "deps: [tst-0002, tst-0003]\nassignee: Sam\n",
    );
    env.ticket("tst-0002", "Two", "status: closed\ntags: [ui]\n");
    env.ticket("tst-0003", "Three", "status: in_progress\n");

    let all = "tst-0001 [open] - One <- [tst-0002, tst-0003]\n\
               tst-0002 [closed] - Two\ntst-0003 [in_progress] - Three\n";
    assert_eq!(env.ok(&["ls"]), all);
    assert_eq!(env.ok(&["list"]), all);
    assert_eq!(
        env.ok(&["ls", "--status=closed"]),
        "tst-0002 [closed] - Two\n"
    );
    assert_eq!(
        env.ok(&["ls", "--status", "closed"]),
        "tst-0002 [closed] - Two\n"
    );
    assert_eq!(env.ok(&["ls", "-T", "ui"]), "tst-0002 [closed] - Two\n");
    assert!(env.ok(&["ls", "-a", "Sam"]).starts_with("tst-0001 "));
    assert_eq!(env.ok(&["ls", "--status=open", "--tag=ui"]), "");
}

#[test]
fn closed_lists_recent_first_with_limit() {
    let env = Env::new();
    for id in ["tst-0001", "tst-0002", "tst-0003"] {
        env.ticket(id, id, "");
    }
    for id in ["tst-0002", "tst-0001"] {
        std::thread::sleep(std::time::Duration::from_millis(20));
        env.ok(&["close", id]);
    }
    assert_eq!(
        env.ok(&["closed"]),
        "tst-0001 [closed] - tst-0001\ntst-0002 [closed] - tst-0002\n"
    );
    assert_eq!(
        env.ok(&["closed", "--limit=1"]),
        "tst-0001 [closed] - tst-0001\n"
    );
    assert!(!env.kk(&["closed", "--limit=x"]).ok);
}

// --- Show ---

#[test]
fn show_adds_relationship_sections() {
    let env = Env::new();
    env.ticket("tst-0001", "Parent", "");
    env.ticket(
        "tst-0002",
        "Main",
        "deps: [tst-0003, tst-0004]\nlinks: [tst-0005]\nparent: tst-0001\n",
    );
    env.ticket("tst-0003", "Open dep", "");
    env.ticket("tst-0004", "Closed dep", "status: closed\n");
    env.ticket("tst-0005", "Friend", "deps: [tst-0002]\n");

    let out = env.ok(&["show", "0002"]);
    assert!(out.starts_with("---\nid: tst-0002\n"));
    assert!(out.contains("parent: tst-0001  # Parent\n"));
    assert!(out.ends_with(
        "\n## Blockers\n\n- tst-0003 [open] Open dep\n\
         \n## Blocking\n\n- tst-0005 [open] Friend\n\
         \n## Linked\n\n- tst-0005 [open] Friend\n"
    ));
    // The file itself is not modified by show.
    assert!(env.read("tst-0002").contains("parent: tst-0001\n"));

    let parent = env.ok(&["show", "tst-0001"]);
    assert!(parent.ends_with("\n## Children\n\n- tst-0002 [open] Main\n"));
    assert!(!env.kk(&["show"]).ok);
}

// --- Notes ---

#[test]
fn add_note_from_args_and_stdin() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "");
    assert_eq!(
        env.ok(&["add-note", "0001", "first", "note"]),
        "Note added to tst-0001\n"
    );

    let mut child = env
        .command()
        .args(["add-note", "tst-0001"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"piped\nnote\n")
        .unwrap();
    assert!(child.wait().unwrap().success());

    let text = env.read("tst-0001");
    assert_eq!(text.matches("## Notes").count(), 1);
    let notes = text.split("## Notes\n").nth(1).unwrap();
    let lines: Vec<&str> = notes.lines().collect();
    assert_eq!(lines.len(), 9, "{notes:?}");
    assert!(lines[1].starts_with("**20") && lines[1].ends_with("Z**"));
    assert_eq!(lines[3], "first note");
    assert_eq!(&lines[7..], ["piped", "note"]);
    assert!(!env.kk(&["add-note"]).ok);
}

// --- Edit and query ---

#[test]
fn edit_prints_path_when_not_interactive() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "");
    let out = env.ok(&["edit", "0001"]);
    assert_eq!(
        out,
        format!("Edit ticket file: {}\n", env.file("tst-0001").display())
    );
}

#[test]
fn query_outputs_json_lines() {
    let env = Env::new();
    env.ticket(
        "tst-0001",
        "One",
        "deps: [tst-0002, tst-0003]\nassignee: Sam \"Q\" \\ Smith\n",
    );
    assert_eq!(
        env.ok(&["query"]),
        "{\"id\":\"tst-0001\",\"status\":\"open\",\"links\":[],\
         \"created\":\"2026-01-01T00:00:00Z\",\"type\":\"task\",\"priority\":\"2\",\
         \"deps\":[\"tst-0002\",\"tst-0003\"],\"assignee\":\"Sam \\\"Q\\\" \\\\ Smith\"}\n"
    );
}

// --- Plugins ---

#[cfg(unix)]
fn install_plugin(dir: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn plugins_extend_and_override_commands() {
    let env = Env::new();
    env.ticket("tst-0001", "One", "");
    let bin = env.root.join("bin");
    install_plugin(
        &bin,
        "kk-hello",
        "#!/bin/sh\n# kk-plugin: Say hello\necho \"hello $1 from $KK_TICKETS_DIR\"\n\"$KK_SCRIPT\" super ls\n",
    );
    install_plugin(&bin, "kk-ls", "#!/bin/sh\necho overridden\n");
    install_plugin(
        &bin,
        "kk-bin",
        "#!/bin/sh\n[ \"$1\" = --kk-describe ] && echo 'kk-plugin: From flag'\n",
    );
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let run = |args: &[&str]| -> Run {
        env.command()
            .env("PATH", &path)
            .args(args)
            .output()
            .unwrap()
            .into()
    };

    let hello = run(&["hello", "world"]);
    assert!(hello.ok, "{}", hello.stderr);
    assert_eq!(
        hello.stdout,
        format!(
            "hello world from {}\ntst-0001 [open] - One\n",
            env.tickets.display()
        )
    );
    assert_eq!(run(&["ls"]).stdout, "overridden\n");
    assert_eq!(run(&["super", "ls"]).stdout, "tst-0001 [open] - One\n");

    let help = run(&["help"]).stdout;
    assert!(help.contains("Plugins (kk-<cmd> in PATH):"));
    assert!(help.contains("  hello                  Say hello\n"));
    assert!(help.contains("  bin                    From flag\n"));
    assert!(help.contains("  ls                     (no description)\n"));
}
