use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_URL: &str = "http://127.0.0.1:9920/evaluate";
const DEFAULT_TIMEOUT_MS: u64 = 3000;

#[derive(Debug, Deserialize, Serialize)]
pub struct Verdict {
    pub action: String,
    pub blocked: bool,
    pub reasons: Vec<String>,
}

/// Build the stdout JSON that tells Codex to deny a tool call.
///
/// Codex denies a `PreToolUse` tool call when the hook prints this object to
/// stdout and exits `0`. We use the structured `hookSpecificOutput` form rather
/// than exiting `2` (the stderr-only alternative): a non-zero exit with JSON on
/// stdout is treated as a hook error and the call is allowed through.
pub fn deny_output(verdict: &Verdict) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": verdict.reasons.join("; "),
        }
    })
    .to_string()
}

/// Evaluate a tool-before event. Returns the Parallax verdict.
/// Blocking is signalled by the caller via [`deny_output`] on stdout + exit `0`.
pub async fn pre_tool_use(hook: &Value) -> Verdict {
    evaluate(json!({
        "stage":      "tool.before",
        "session_id": codex_session(hook),
        "tool_name":  str_field(hook, "tool_name"),
        "tool_args":  hook.get("tool_input").cloned().unwrap_or_default(),
        "timestamp":  unix_ts(),
    }))
    .await
}

/// Evaluate a tool-after event. Fire-and-forget — verdict is informational.
pub async fn post_tool_use(hook: &Value) -> Verdict {
    let result = hook.get("tool_response").cloned().unwrap_or_default();
    let result_str = if result.is_string() {
        result.as_str().unwrap_or("").to_string()
    } else {
        result.to_string()
    };

    evaluate(json!({
        "stage":       "tool.after",
        "session_id":  codex_session(hook),
        "tool_name":   str_field(hook, "tool_name"),
        "tool_args":   hook.get("tool_input").cloned().unwrap_or_default(),
        "tool_result": result_str,
        "timestamp":   unix_ts(),
    })).await
}

/// Evaluate a Codex `notify` event. Fire-and-forget — verdict is informational.
///
/// Codex passes the event as a JSON object, e.g. for `agent-turn-complete`:
/// `{ "type", "turn-id", "input-messages": [..], "last-assistant-message" }`.
pub async fn notification(hook: &Value) -> Verdict {
    evaluate(json!({
        "stage":        "message.before",
        "session_id":   codex_session(hook),
        "message_text": codex_message(hook),
        "event_type":   str_field(hook, "type"),
        "timestamp":    unix_ts(),
    }))
    .await
}

async fn evaluate(event: Value) -> Verdict {
    let url = std::env::var("PARALLAX_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let timeout_ms = std::env::var("PARALLAX_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
    {
        Ok(c) => c,
        Err(_) => return allow(),
    };

    match client.post(&url).json(&event).send().await {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<Verdict>().await.unwrap_or_else(|_| allow())
        }
        _ => allow(),
    }
}

fn allow() -> Verdict {
    Verdict {
        action: "allow".to_string(),
        blocked: false,
        reasons: vec![],
    }
}

/// Extract the session/turn identifier from a Codex event, falling back to the
/// generic `session_id` field used by the other Parallax integrations.
fn codex_session(hook: &Value) -> String {
    let turn = str_field(hook, "turn-id");
    if !turn.is_empty() {
        return turn.to_string();
    }
    str_field(hook, "session_id").to_string()
}

/// Extract the message text from a Codex event. Prefers the assistant's last
/// message, then a generic `message`, then the joined user input messages.
fn codex_message(hook: &Value) -> String {
    let last = str_field(hook, "last-assistant-message");
    if !last.is_empty() {
        return last.to_string();
    }
    let msg = str_field(hook, "message");
    if !msg.is_empty() {
        return msg.to_string();
    }
    hook.get("input-messages")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn unix_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_prefers_last_assistant_message() {
        let event = json!({
            "type": "agent-turn-complete",
            "turn-id": "t-42",
            "input-messages": ["rename foo to bar"],
            "last-assistant-message": "Done!",
        });
        assert_eq!(codex_message(&event), "Done!");
        assert_eq!(codex_session(&event), "t-42");
    }

    #[test]
    fn message_falls_back_to_input_messages() {
        let event = json!({
            "input-messages": ["first", "second"],
        });
        assert_eq!(codex_message(&event), "first\nsecond");
    }

    #[test]
    fn session_falls_back_to_session_id() {
        let event = json!({ "session_id": "s-1" });
        assert_eq!(codex_session(&event), "s-1");
    }

    #[test]
    fn deny_output_uses_codex_permission_decision() {
        let verdict = Verdict {
            action: "block".to_string(),
            blocked: true,
            reasons: vec!["dangerous-commands".to_string(), "rm -rf".to_string()],
        };
        let parsed: Value = serde_json::from_str(&deny_output(&verdict)).unwrap();
        let out = &parsed["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PreToolUse");
        assert_eq!(out["permissionDecision"], "deny");
        assert_eq!(out["permissionDecisionReason"], "dangerous-commands; rm -rf");
    }
}
