use std::path::PathBuf;

use toml_edit::{value, Array, DocumentMut, Item};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9920;

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

/// Install the Parallax `notify` hook into the Codex config (`~/.codex/config.toml`).
///
/// Codex invokes the `notify` program with the event JSON as a trailing argument
/// when an agent turn completes. The hook forwards it to the Parallax server.
pub fn setup(host: &str, port: u16) {
    let path = config_path();

    println!("Installing Codex notify hook");
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
    notify.push(exe);
    notify.push("codex");
    notify.push("hook");
    notify.push("notification");
    doc["notify"] = value(notify);

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

    println!("  + notify hook");
    println!();
    println!("Done. Parallax notify hook written to {}", path.display());
    println!();
    println!("Start the evaluation server with:");
    println!("  parallax serve -c config.yaml");
}

/// Remove the Parallax `notify` hook from the Codex config (`~/.codex/config.toml`).
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

    match doc.get("notify") {
        Some(n) if is_parallax_notify(n) => {
            doc.remove("notify");
        }
        _ => {
            println!("No Parallax notify hook found in {}", path.display());
            return;
        }
    }

    if let Err(e) = std::fs::write(&path, doc.to_string()) {
        eprintln!("  ERROR: Failed to write {}: {}", path.display(), e);
        std::process::exit(1);
    }

    println!("Done. Parallax notify hook removed from {}", path.display());
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
