/*!
 * Parallax Security — Codex CLI hook binary.
 *
 * Thin CLI wrapper around the `parallax_codex_hooks` library. Reads the hook
 * event either from a trailing JSON argument (how Codex's `notify` program is
 * invoked) or from stdin, delegates to the library, and exits with code 2
 * (+ block JSON to stdout) when `pre-tool-use` is blocked.
 *
 * Usage:
 *   parallax-codex-hooks notification [json]   — Codex notify (fire-and-forget)
 *   parallax-codex-hooks pre-tool-use  [json]  — tool-before (sequential, can block)
 *   parallax-codex-hooks post-tool-use [json]  — tool-after (fire-and-forget)
 *
 * Env vars:
 *   PARALLAX_URL     — evaluation endpoint (default: http://127.0.0.1:9920/evaluate)
 *   PARALLAX_TIMEOUT — request timeout in ms (default: 3000)
 */

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();

    match cmd.as_str() {
        "pre-tool-use" | "post-tool-use" | "notification" => {}
        _ => {
            eprintln!(
                "Usage: parallax-codex-hooks <pre-tool-use|post-tool-use|notification> [json]"
            );
            std::process::exit(1);
        }
    }

    let hook = read_event(args.next()).await;

    match cmd.as_str() {
        "pre-tool-use" => {
            let verdict = parallax_codex_hooks::pre_tool_use(&hook).await;
            if verdict.blocked {
                print!(
                    "{}",
                    json!({
                        "decision": "block",
                        "reason": verdict.reasons.join("; "),
                    })
                );
                std::process::exit(2);
            }
        }
        "post-tool-use" => {
            parallax_codex_hooks::post_tool_use(&hook).await;
        }
        "notification" => {
            parallax_codex_hooks::notification(&hook).await;
        }
        _ => unreachable!(),
    }
}

/// Read the event JSON from a trailing argument (Codex `notify` style) when
/// present, otherwise from stdin. Exits 0 on missing/invalid input (fail open).
async fn read_event(payload: Option<String>) -> Value {
    if let Some(raw) = payload {
        return serde_json::from_str(&raw).unwrap_or_else(|_| std::process::exit(0));
    }

    let mut input = String::new();
    if tokio::io::stdin().read_to_string(&mut input).await.is_err() {
        std::process::exit(0);
    }
    serde_json::from_str(&input).unwrap_or_else(|_| std::process::exit(0))
}
