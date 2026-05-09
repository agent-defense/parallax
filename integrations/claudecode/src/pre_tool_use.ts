#!/usr/bin/env tsx
/**
 * Parallax Security — Claude Code PreToolUse hook.
 *
 * Reads hook input from stdin, forwards to the Parallax evaluation server,
 * and blocks the tool call when the verdict says so (exit 2 + JSON to stdout).
 *
 * Hooks:
 *   PreToolUse  (sequential — can block)  →  stage: tool.before
 *
 * Env vars:
 *   PARALLAX_URL      — evaluation endpoint (default: http://127.0.0.1:9920/evaluate)
 *   PARALLAX_TIMEOUT  — request timeout in ms (default: 3000)
 */

const DEFAULT_URL = "http://127.0.0.1:9920/evaluate";

interface Verdict {
  action: "allow" | "block" | "detect" | "redact";
  blocked: boolean;
  reasons: string[];
}

async function evaluate(event: Record<string, unknown>): Promise<Verdict> {
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
    if (!resp.ok) return { action: "allow", blocked: false, reasons: [] };
    return (await resp.json()) as Verdict;
  } catch {
    return { action: "allow", blocked: false, reasons: [] };
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

  const verdict = await evaluate({
    stage: "tool.before",
    session_id: (hook.session_id as string) || "",
    tool_name: (hook.tool_name as string) || "",
    tool_args: (hook.tool_input as Record<string, unknown>) || {},
    timestamp: Date.now() / 1000,
  });

  if (verdict.blocked) {
    process.stdout.write(
      JSON.stringify({
        decision: "block",
        reason: verdict.reasons.join("; ") || "Blocked by Parallax",
      }),
    );
    process.exit(2);
  }
}

main().catch(() => process.exit(0));
