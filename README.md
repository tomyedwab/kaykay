# kaykay

A simple file-based ticket tracker. `kk` is a Rust port of
[wedow/ticket](https://github.com/wedow/ticket) (`tk`): tickets are markdown
files with YAML frontmatter, with dependency tracking, links, and notes. The
commands, options, and ticket file format are the same as `tk`.

## Differences from `tk`

- The CLI is named `kk`.
- **Ticket storage is set only by `KK_TICKETS_DIR`.** `kk` never looks at the
  working directory or its parents for a `.tickets/` folder.
- **Ticket ID prefixes are set by `KK_PREFIX`**, not derived from the working
  directory name. IDs are `<prefix>-<4 random chars>`, e.g. `kk-5c46`.
- `ls`/`list`, `edit`, and `query` are built in rather than shipped as plugins.
  `migrate-beads` is not included.
- Plugins are executables named `kk-<cmd>`. They receive `KK_TICKETS_DIR` and
  `KK_SCRIPT`, and describe themselves with `# kk-plugin: ...` or
  `--kk-describe`.
- The pager for `show` is `KK_PAGER` (falling back to `PAGER`).

## Install

```bash
cargo install --path .
```

There are no dependencies beyond the Rust standard library. `kk query` with a
filter needs `jq` on your PATH.

## Configuration

```bash
export KK_TICKETS_DIR=~/tickets/my-project   # required by every command
export KK_PREFIX=mp                          # required by `kk create`
```

`kk create` creates `KK_TICKETS_DIR` if it does not exist yet. Every other
command fails if the directory is missing.

## Usage

```
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
```

Partial IDs work anywhere an ID is expected: `kk show 5c4` matches `mp-5c46`.

## Ticket format

```markdown
---
id: mp-5c46
status: open
deps: []
links: []
created: 2026-11-24T22:38:09Z
type: task
priority: 2
assignee: Sam
tags: [ui, backend]
---
# Ticket title

Description text
```

## Plugins

An executable named `kk-<cmd>` on your PATH is run for `kk <cmd>`, so you can
add commands or override built-in ones. Use `kk super <cmd>` to bypass plugins.

```bash
cat > ~/.local/bin/kk-hello <<'EOF'
#!/bin/bash
# kk-plugin: Say hello
echo "Hello from plugin!"
"$KK_SCRIPT" super ls
EOF
chmod +x ~/.local/bin/kk-hello

kk hello   # runs kk-hello
kk help    # lists it under "Plugins"
```

## Development

```bash
cargo test
```

## License

MIT
