use std::path::PathBuf;

use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9920;
const HOOK_TIMEOUT_SECS: i64 = 30;

/// Path to the Codex config file. Honors `CODEX_HOME` (defaults to `~/.codex`).
fn config_path() -> PathBuf {
    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(".codex")
        });
    codex_home.join("config.toml")
}

/// Install the Parallax hooks into the Codex config (`~/.codex/config.toml`).
///
/// Three hooks are wired:
/// - `notify` — fire-and-forget program invoked when an agent turn completes.
/// - `[[hooks.PreToolUse]]` — runs before a tool call; blocks it (exit 2) on a
///   `block` verdict.
/// - `[[hooks.PostToolUse]]` — fire-and-forget, runs after a tool call.
pub fn setup(host: &str, port: u16) {
    let path = config_path();

    println!("Installing Codex hooks");
    println!();

    let exe = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("parallax"))
        .display()
        .to_string();

    let mut doc: DocumentMut = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.parse::<DocumentMut>().ok())
            .unwrap_or_default()
    } else {
        DocumentMut::new()
    };

    if let Some(existing) = doc.get("notify") {
        if !is_parallax_notify(existing) {
            println!("  WARNING: Replacing an existing 'notify' program.");
            if let Some(arr) = existing.as_array() {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                println!("    Previous: [{}]", parts.join(", "));
            }
            println!("    Re-add it manually after `parallax revert codex` if still needed.");
        }
    }

    let mut notify = Array::new();
    if host != DEFAULT_HOST || port != DEFAULT_PORT {
        notify.push("env");
        notify.push(format!("PARALLAX_URL=http://{}:{}/evaluate", host, port));
    }
    notify.push(&exe);
    notify.push("codex");
    notify.push("hook");
    notify.push("notification");
    doc["notify"] = value(notify);
    println!("  + notify hook");

    merge_hook(
        &mut doc,
        "PreToolUse",
        hook_command(&exe, host, port, "pre-tool-use"),
        "Parallax pre-tool-use check",
    );
    merge_hook(
        &mut doc,
        "PostToolUse",
        hook_command(&exe, host, port, "post-tool-use"),
        "Parallax post-tool-use review",
    );

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("  ERROR: Could not create {}: {}", parent.display(), e);
                std::process::exit(1);
            }
        }
    }

    if let Err(e) = std::fs::write(&path, doc.to_string()) {
        eprintln!("  ERROR: Failed to write {}: {}", path.display(), e);
        std::process::exit(1);
    }

    println!();
    println!("Done. Parallax hooks written to {}", path.display());
    println!();
    println!("Start the evaluation server with:");
    println!("  parallax serve -c config.yaml");
}

/// Remove the Parallax hooks from the Codex config (`~/.codex/config.toml`).
pub fn revert() {
    let path = config_path();

    if !path.exists() {
        println!("No Codex config found at {}", path.display());
        return;
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  ERROR: Failed to read {}: {}", path.display(), e);
            std::process::exit(1);
        }
    };

    let mut doc: DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("  ERROR: Failed to parse {}: {}", path.display(), e);
            std::process::exit(1);
        }
    };

    let mut removed = false;

    if matches!(doc.get("notify"), Some(n) if is_parallax_notify(n)) {
        doc.remove("notify");
        removed = true;
    }

    if remove_parallax_hooks(&mut doc) {
        removed = true;
    }

    if !removed {
        println!("No Parallax hooks found in {}", path.display());
        return;
    }

    if let Err(e) = std::fs::write(&path, doc.to_string()) {
        eprintln!("  ERROR: Failed to write {}: {}", path.display(), e);
        std::process::exit(1);
    }

    println!("Done. Parallax hooks removed from {}", path.display());
}

/// Build the shell command string for a `[[hooks.<event>]]` entry. Codex runs
/// the `command` through a shell, so a `PARALLAX_URL=...` prefix works for
/// non-default endpoints.
fn hook_command(exe: &str, host: &str, port: u16, event: &str) -> String {
    if host != DEFAULT_HOST || port != DEFAULT_PORT {
        format!(
            "PARALLAX_URL=http://{}:{}/evaluate \"{}\" codex hook {}",
            host, port, exe, event
        )
    } else {
        format!("\"{}\" codex hook {}", exe, event)
    }
}

/// Replace or append our entry for the given hook event, evicting stale Parallax
/// entries first while preserving any user-defined hooks.
fn merge_hook(doc: &mut DocumentMut, event: &str, command: String, status: &str) {
    println!("  + {} hook", event);

    let hooks = doc
        .entry("hooks")
        .or_insert_with(|| Item::Table(Table::new()));
    if let Some(tbl) = hooks.as_table_mut() {
        tbl.set_implicit(true);
    }
    let hooks = match hooks.as_table_mut() {
        Some(t) => t,
        None => return,
    };

    let mut rebuilt = ArrayOfTables::new();
    if let Some(existing) = hooks.get(event).and_then(Item::as_array_of_tables) {
        for entry in existing.iter() {
            if !is_parallax_hook_entry(entry) {
                rebuilt.push(entry.clone());
            }
        }
    }
    rebuilt.push(parallax_hook_entry(command, status));

    hooks.insert(event, Item::ArrayOfTables(rebuilt));
}

/// Build a single `[[hooks.<event>]]` table for Parallax.
fn parallax_hook_entry(command: String, status: &str) -> Table {
    let mut cmd = Table::new();
    cmd["type"] = value("command");
    cmd["command"] = value(command);
    cmd["timeout"] = value(HOOK_TIMEOUT_SECS);
    cmd["statusMessage"] = value(status);

    let mut inner = ArrayOfTables::new();
    inner.push(cmd);

    let mut entry = Table::new();
    entry["matcher"] = value(".*");
    entry.insert("hooks", Item::ArrayOfTables(inner));
    entry
}

/// Strip every Parallax hook entry from all `[[hooks.*]]` arrays, dropping any
/// arrays (and the `hooks` table) left empty.
fn remove_parallax_hooks(doc: &mut DocumentMut) -> bool {
    let hooks = match doc.get_mut("hooks").and_then(Item::as_table_mut) {
        Some(h) => h,
        None => return false,
    };

    let mut removed = false;
    let events: Vec<String> = hooks.iter().map(|(k, _)| k.to_string()).collect();

    for event in events {
        let Some(existing) = hooks.get(&event).and_then(Item::as_array_of_tables) else {
            continue;
        };
        let before = existing.len();
        let mut rebuilt = ArrayOfTables::new();
        for entry in existing.iter() {
            if !is_parallax_hook_entry(entry) {
                rebuilt.push(entry.clone());
            }
        }
        if rebuilt.len() == before {
            continue;
        }
        removed = true;
        if rebuilt.is_empty() {
            hooks.remove(&event);
        } else {
            hooks.insert(&event, Item::ArrayOfTables(rebuilt));
        }
    }

    if hooks.is_empty() {
        doc.remove("hooks");
    }

    removed
}

/// Returns true if the `notify` array references a Parallax binary.
fn is_parallax_notify(item: &Item) -> bool {
    item.as_array()
        .map(|arr| {
            arr.iter()
                .any(|v| v.as_str().map(|s| s.contains("parallax")).unwrap_or(false))
        })
        .unwrap_or(false)
}

/// Returns true if a `[[hooks.<event>]]` table is a Parallax-managed entry.
fn is_parallax_hook_entry(entry: &Table) -> bool {
    entry
        .get("hooks")
        .and_then(Item::as_array_of_tables)
        .map(|inner| {
            inner.iter().any(|cmd| {
                cmd.get("command")
                    .and_then(Item::as_str)
                    .map(|c| c.contains("parallax") && c.contains("codex hook"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(doc: &mut DocumentMut, exe: &str, host: &str, port: u16) {
        let mut notify = Array::new();
        if host != DEFAULT_HOST || port != DEFAULT_PORT {
            notify.push("env");
            notify.push(format!("PARALLAX_URL=http://{}:{}/evaluate", host, port));
        }
        notify.push(exe);
        notify.push("codex");
        notify.push("hook");
        notify.push("notification");
        doc["notify"] = value(notify);
        merge_hook(doc, "PreToolUse", hook_command(exe, host, port, "pre-tool-use"), "pre");
        merge_hook(doc, "PostToolUse", hook_command(exe, host, port, "post-tool-use"), "post");
    }

    #[test]
    fn setup_writes_valid_parseable_toml() {
        let mut doc = DocumentMut::new();
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);

        let rendered = doc.to_string();
        // Must round-trip as valid TOML.
        let reparsed: DocumentMut = rendered.parse().expect("valid toml");

        assert!(reparsed.get("notify").is_some());
        let pre = reparsed["hooks"]["PreToolUse"]
            .as_array_of_tables()
            .unwrap();
        assert_eq!(pre.len(), 1);
        let cmd = pre.get(0).unwrap()["hooks"]
            .as_array_of_tables()
            .unwrap()
            .get(0)
            .unwrap()["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains("codex hook pre-tool-use"));
        assert!(reparsed["hooks"]["PostToolUse"].is_array_of_tables());
    }

    #[test]
    fn notify_stays_before_hook_tables() {
        let mut doc = DocumentMut::new();
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);
        let rendered = doc.to_string();
        let notify_pos = rendered.find("notify =").expect("notify present");
        let hooks_pos = rendered.find("[[hooks").expect("hooks present");
        assert!(notify_pos < hooks_pos, "notify must precede hook tables:\n{rendered}");
    }

    #[test]
    fn custom_endpoint_injects_url_into_hook_command() {
        let mut doc = DocumentMut::new();
        install(&mut doc, "/usr/bin/parallax", "0.0.0.0", 9999);
        let rendered = doc.to_string();
        assert!(rendered.contains("PARALLAX_URL=http://0.0.0.0:9999/evaluate"));
    }

    #[test]
    fn setup_preserves_existing_user_hooks() {
        let existing = r#"
[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "/usr/bin/my-own-linter"
"#;
        let mut doc: DocumentMut = existing.parse().unwrap();
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);

        let pre = doc["hooks"]["PreToolUse"].as_array_of_tables().unwrap();
        assert_eq!(pre.len(), 2, "user hook + parallax hook");
        assert!(pre.iter().any(|t| !is_parallax_hook_entry(t)));
        assert!(pre.iter().any(is_parallax_hook_entry));
    }

    #[test]
    fn setup_is_idempotent() {
        let mut doc = DocumentMut::new();
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);
        let pre = doc["hooks"]["PreToolUse"].as_array_of_tables().unwrap();
        assert_eq!(pre.len(), 1, "re-running setup must not duplicate entries");
    }

    #[test]
    fn revert_removes_parallax_keeps_user_hooks() {
        let existing = r#"
[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "/usr/bin/my-own-linter"
"#;
        let mut doc: DocumentMut = existing.parse().unwrap();
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);

        assert!(matches!(doc.get("notify"), Some(n) if is_parallax_notify(n)));
        if matches!(doc.get("notify"), Some(n) if is_parallax_notify(n)) {
            doc.remove("notify");
        }
        assert!(remove_parallax_hooks(&mut doc));

        assert!(doc.get("notify").is_none());
        // PostToolUse was parallax-only → gone; PreToolUse keeps the user entry.
        assert!(doc.get("hooks").is_some());
        let pre = doc["hooks"]["PreToolUse"].as_array_of_tables().unwrap();
        assert_eq!(pre.len(), 1);
        assert!(!is_parallax_hook_entry(pre.get(0).unwrap()));
        assert!(doc["hooks"].get("PostToolUse").is_none());
    }

    #[test]
    fn revert_drops_empty_hooks_table() {
        let mut doc = DocumentMut::new();
        install(&mut doc, "/usr/bin/parallax", DEFAULT_HOST, DEFAULT_PORT);
        if matches!(doc.get("notify"), Some(n) if is_parallax_notify(n)) {
            doc.remove("notify");
        }
        assert!(remove_parallax_hooks(&mut doc));
        assert!(doc.get("hooks").is_none(), "empty hooks table should be removed");
    }
}
