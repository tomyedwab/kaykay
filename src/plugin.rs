//! Plugins: executables named `kk-<cmd>` on PATH extend or override commands.

use crate::Res;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PREFIX: &str = "kk-";

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// Find the plugin executable for a command, if any.
pub fn find(command: &str) -> Option<PathBuf> {
    if command.is_empty() || command.contains(['/', '\\']) {
        return None;
    }
    let name = format!("{PREFIX}{command}");
    path_dirs()
        .into_iter()
        .map(|dir| dir.join(&name))
        .find(|p| is_executable(p))
}

/// Run a plugin in place of this process, passing context via environment.
pub fn run(path: &Path, args: &[String]) -> Res {
    let mut command = Command::new(path);
    command.args(args);
    if let Ok(exe) = std::env::current_exe() {
        command.env("KK_SCRIPT", exe);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        Err(format!(
            "Error: cannot run plugin {}: {err}",
            path.display()
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|e| format!("Error: cannot run plugin {}: {e}", path.display()))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Description from a `# kk-plugin: text` comment in the first 10 lines.
fn comment_description(path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut buf)
        .ok()?;
    String::from_utf8_lossy(&buf)
        .lines()
        .take(10)
        .find_map(|l| l.strip_prefix("# kk-plugin:"))
        .map(|d| d.trim().to_string())
}

/// Description from running the plugin with `--kk-describe` (1s timeout).
fn flag_description(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--kk-describe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut out = String::new();
    child
        .stdout
        .take()?
        .take(4096)
        .read_to_string(&mut out)
        .ok()?;
    out.lines()
        .next()?
        .strip_prefix("kk-plugin:")
        .map(|d| d.trim().to_string())
}

/// Installed plugins as `(command, description)`, sorted by command.
pub fn list() -> Vec<(String, String)> {
    let mut plugins: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in path_dirs() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(command) = name.strip_prefix(PREFIX).filter(|c| !c.is_empty()) else {
                continue;
            };
            // Earlier PATH entries win, as with command lookup.
            if !plugins.contains_key(command) && is_executable(&entry.path()) {
                plugins.insert(command.to_string(), entry.path());
            }
        }
    }
    plugins
        .into_iter()
        .map(|(command, path)| {
            let description = comment_description(&path)
                .filter(|d| !d.is_empty())
                .or_else(|| flag_description(&path))
                .unwrap_or_else(|| "(no description)".to_string());
            (command, description)
        })
        .collect()
}
