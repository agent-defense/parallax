#!/usr/bin/env tsx
/**
 * Parallax Security — Claude Code Notification hook (fire-and-forget).
 *
 * Forwards notification events to Parallax for message-level monitoring.
 * Never blocks — exits 0 regardless of verdict.
 *
 * Hooks:
 *   Notification  (fire-and-forget)  →  stage: message.before
 *
 * Env vars:
 *   PARALLAX_URL      — evaluation endpoint (default: http://127.0.0.1:9920/evaluate)
 *   PARALLAX_TIMEOUT  — request timeout in ms (default: 3000)
 */

const DEFAULT_URL = "http://127.0.0.1:9920/evaluate";

async function evaluate(event: Record<string, unknown>): Promise<void> {
  const url = process.env.PARALLAX_URL || DEFAULT_URL;
  const timeout = parseInt(process.env.PARALLAX_TIMEOUT || "3000", 10);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);

  try {
    const resp = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(event),
      signal: controller.signal,
    });
    if (!resp.ok) return;
    await resp.json();
  } catch {
    // fire-and-forget: ignore errors
  } finally {
    clearTimeout(timer);
  }
}

async function main(): Promise<void> {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;

  let hook: Record<string, unknown>;
  try {
    hook = JSON.parse(input);
  } catch {
    process.exit(0);
  }

  await evaluate({
    stage: "message.before",
    session_id: (hook.session_id as string) || "",
    message_text: (hook.message as string) || "",
    timestamp: Date.now() / 1000,
  });
}

main().catch(() => process.exit(0));
