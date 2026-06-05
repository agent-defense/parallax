/**
 * Parallax Security — OpenClaw server-mode integration.
 *
 * Registers lifecycle hooks and forwards events to the Parallax evaluation
 * server. The server runs the evaluator chain and returns a verdict.
 *
 * Hooks:
 *   - before_tool_call  (sequential — can block)
 *   - after_tool_call   (fire-and-forget)
 *   - message_received  (fire-and-forget)
 *
 * Env vars:
 *   PARALLAX_URL      — evaluation endpoint (default: http://127.0.0.1:9920/evaluate)
 *   PARALLAX_TIMEOUT  — request timeout in ms (default: 3000)
 */
/*
* # Set a single env var
openclaw config set env.PARALLAX_URL "http://127.0.0.1:47549/ai_defender/evaluate"

# Also works via the nested vars object
openclaw config set env.vars.MY_WEBHOOK_URL "ws://localhost:8765"

# Set multiple env vars at once (batch)
openclaw config set env '{
  "LOG_LEVEL": "debug",
  "MY_PLUGIN_DEBUG": "true",
  "LOG_SOCKET_URL": "ws://127.0.0.1:9000"
}'
* */
// Config is resolved once at module load — keeps env reads away from network calls.
const config = {
  url: process.env.PARALLAX_URL || "http://127.0.0.1:9920/evaluate",
  timeout: parseInt(process.env.PARALLAX_TIMEOUT || "3000", 10),
};

interface Verdict {
  action: "allow" | "block" | "detect" | "redact";
  blocked: boolean;
  reasons: string[];
}

async function evaluate(event: Record<string, unknown>): Promise<Verdict> {
  const { url, timeout } = config;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);

  try {
    const resp = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(event),
      signal: controller.signal,
    });
    if (!resp.ok) return { action: "allow", blocked: false, reasons: [] };
    return (await resp.json()) as Verdict;
  } catch {
    return { action: "allow", blocked: false, reasons: [] };
  } finally {
    clearTimeout(timer);
  }
}

export default function register(api: any) {
  api.on(
      "before_tool_call",
      async (event: any, ctx: any) => {
        const verdict = await evaluate({
          stage: "tool.before",
          session_id: ctx?.sessionId || "",
          tool_name: event.toolName || "",
          tool_args: event.params || {},
          timestamp: Date.now() / 1000,
        });
        if (verdict.blocked) {
          return { block: true, blockReason: verdict.reasons.join("; ") || "Blocked by Parallax" };
        }
        return {};
      },
      { priority: 1000 },
  );

  api.on(
      "after_tool_call",
      async (event: any, ctx: any) => {
        await evaluate({
          stage: "tool.after",
          session_id: ctx?.sessionId || "",
          tool_name: event.toolName || "",
          tool_args: event.params || {},
          tool_result: typeof event.result === "string" ? event.result : JSON.stringify(event.result),
          timestamp: Date.now() / 1000,
        });
      },
      { priority: 1000 },
  );

  api.on(
      "message_received",
      async (event: any, ctx: any) => {
        await evaluate({
          stage: "message.before",
          session_id: ctx?.sessionId || "",
          message_text: event.text || event.content || "",
          channel: event.channel || "",
          user_id: event.userId || "",
          timestamp: Date.now() / 1000,
        });
      },
      { priority: 1000 },
  );
}
