/*!
 * Parallax Security — Claude Code hook binary.
 *
 * Thin CLI wrapper around the `parallax_hooks` library. Reads the hook
 * event from stdin as JSON, delegates to the library, and exits with
 * code 2 (+ block JSON to stdout) when `pre-tool-use` is blocked.
 *
 * Usage:
 *   parallax-hooks pre-tool-use   — PreToolUse  (sequential, can block)
 *   parallax-hooks post-tool-use  — PostToolUse (fire-and-forget)
 *   parallax-hooks notification   — Notification (fire-and-forget)
 *
 * Env vars:
 *   PARALLAX_URL     — evaluation endpoint (default: http://127.0.0.1:9920/evaluate)
 *   PARALLAX_TIMEOUT — request timeout in ms (default: 3000)
 */

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();

    match cmd.as_str() {
        "pre-tool-use" | "post-tool-use" | "notification" => {}
        _ => {
            eprintln!("Usage: parallax-hooks <pre-tool-use|post-tool-use|notification>");
            std::process::exit(1);
        }
    }

    let hook = read_stdin().await;

    match cmd.as_str() {
        "pre-tool-use" => {
            let verdict = parallax_hooks::pre_tool_use(&hook).await;
            let val = json!({
                "decision": "block",
                "reason": verdict.reasons.join("; "),
            });
            if verdict.blocked {
                print!("{val}");
                std::process::exit(2);
            }
        }
        "post-tool-use" => {
            parallax_hooks::post_tool_use(&hook).await;
        }
        "notification" => {
            parallax_hooks::notification(&hook).await;
        }
        _ => unreachable!(),
    }
}

async fn read_stdin() -> Value {
    let mut input = String::new();
    if tokio::io::stdin()
        .read_to_string(&mut input)
        .await
        .is_err()
    {
        std::process::exit(0);
    }
    serde_json::from_str(&input).unwrap_or_else(|_| std::process::exit(0))
}
