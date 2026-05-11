use std::path::PathBuf;

use serde_json::{json, Value};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9920;

fn global_settings_path() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".claude")
        .join("settings.json")
}

/// Install Parallax hook commands into the global Claude Code settings (~/.claude/settings.json).
pub fn setup(host: &str, port: u16) {
    let settings_path = global_settings_path();

    println!("Installing Claude Code hooks");
    println!();

    let exe = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("parallax"))
        .display()
        .to_string();

    let make_cmd = |event: &str| -> String {
        if host != DEFAULT_HOST || port != DEFAULT_PORT {
            format!(
                "PARALLAX_URL=http://{}:{}/evaluate {} claudecode hook {}",
                host, port, exe, event
            )
        } else {
            format!("{} claudecode hook {}", exe, event)
        }
    };

    let mut settings: Value = if settings_path.exists() {
        std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };

    if settings.get("hooks").is_none() {
        settings["hooks"] = json!({});
    }

    let hooks = settings["hooks"].as_object_mut().expect("hooks is object");

    merge_hook(
        hooks,
        "PreToolUse",
        json!({
            "matcher": ".*",
            "hooks": [{"type": "command", "command": make_cmd("pre-tool-use")}]
        }),
    );
    merge_hook(
        hooks,
        "PostToolUse",
        json!({
            "matcher": ".*",
            "hooks": [{"type": "command", "command": make_cmd("post-tool-use")}]
        }),
    );
    merge_hook(
        hooks,
        "Notification",
        json!({
            "hooks": [{"type": "command", "command": make_cmd("notification")}]
        }),
    );

    if let Some(parent) = settings_path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("  ERROR: Could not create {}: {}", parent.display(), e);
                std::process::exit(1);
            }
        }
    }

    let json_str = match serde_json::to_string_pretty(&settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ERROR: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = std::fs::write(&settings_path, format!("{}\n", json_str)) {
        eprintln!("  ERROR: Failed to write {}: {}", settings_path.display(), e);
        std::process::exit(1);
    }

    println!("Done. Parallax hooks written to {}", settings_path.display());
    println!();
    println!("Start the evaluation server with:");
    println!("  parallax serve -c config.yaml");
}

/// Remove Parallax hook entries from the global Claude Code settings (~/.claude/settings.json).
pub fn revert() {
    let settings_path = global_settings_path();

    if !settings_path.exists() {
        println!("No Claude Code settings found at {}", settings_path.display());
        return;
    }

    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  ERROR: Failed to read {}: {}", settings_path.display(), e);
            std::process::exit(1);
        }
    };

    let mut settings: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  ERROR: Failed to parse {}: {}", settings_path.display(), e);
            std::process::exit(1);
        }
    };

    if !remove_parallax_hooks(&mut settings) {
        println!("No Parallax hooks found in {}", settings_path.display());
        return;
    }

    let json_str = match serde_json::to_string_pretty(&settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ERROR: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = std::fs::write(&settings_path, format!("{}\n", json_str)) {
        eprintln!("  ERROR: Failed to write {}: {}", settings_path.display(), e);
        std::process::exit(1);
    }

    println!("Done. Parallax hooks removed from {}", settings_path.display());
}

/// Replace or append our entry for the given hook type, evicting stale Parallax entries first.
fn merge_hook(hooks: &mut serde_json::Map<String, Value>, hook_type: &str, entry: Value) {
    println!("  + {} hook", hook_type);

    let existing = hooks
        .get(hook_type)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut filtered: Vec<Value> = existing
        .into_iter()
        .filter(|e| !is_parallax_entry(e))
        .collect();

    filtered.push(entry);
    hooks.insert(hook_type.to_string(), Value::Array(filtered));
}

fn remove_parallax_hooks(settings: &mut Value) -> bool {
    let hooks = match settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        Some(h) => h,
        None => return false,
    };

    let mut removed = false;
    for entries in hooks.values_mut() {
        if let Some(arr) = entries.as_array_mut() {
            let before = arr.len();
            arr.retain(|e| !is_parallax_entry(e));
            if arr.len() < before {
                removed = true;
            }
        }
    }

    hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));

    removed
}

/// Returns true if any inner hook command is a Parallax claudecode hook entry.
fn is_parallax_entry(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| {
                        c.contains("parallax claudecode hook")
                            || c.contains("claudecode/src/")
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
