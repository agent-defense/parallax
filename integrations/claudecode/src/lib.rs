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

/// Evaluate a `PreToolUse` hook event. Returns the Parallax verdict.
/// Exit code logic (2 = block) is left to the caller.
pub async fn pre_tool_use(hook: &Value) -> Verdict {
    evaluate(json!({
        "stage": "tool.before",
        "session_id": str_field(hook, "session_id"),
        "tool_name":  str_field(hook, "tool_name"),
        "tool_args":  hook.get("tool_input").cloned().unwrap_or_default(),
        "timestamp":  unix_ts(),
    }))
    .await
}

/// Evaluate a `PostToolUse` hook event. Fire-and-forget — verdict is informational.
pub async fn post_tool_use(hook: &Value) -> Verdict {
    let result = hook.get("tool_response").cloned().unwrap_or_default();
    let result_str = if result.is_string() {
        result.as_str().unwrap_or("").to_string()
    } else {
        result.to_string()
    };

    evaluate(json!({
        "stage":       "tool.after",
        "session_id":  str_field(hook, "session_id"),
        "tool_name":   str_field(hook, "tool_name"),
        "tool_args":   hook.get("tool_input").cloned().unwrap_or_default(),
        "tool_result": result_str,
        "timestamp":   unix_ts(),
    }))
    .await
}

/// Evaluate a `Notification` hook event. Fire-and-forget — verdict is informational.
pub async fn notification(hook: &Value) -> Verdict {
    evaluate(json!({
        "stage":        "message.before",
        "session_id":   str_field(hook, "session_id"),
        "message_text": str_field(hook, "message"),
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

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn unix_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
