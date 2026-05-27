# Codex CLI Integration

Parallax integrates with [Codex CLI](https://github.com/openai/codex) through Codex's hook system. It wires three hooks into `~/.codex/config.toml`: the `notify` program (agent-turn-complete) plus `PreToolUse` and `PostToolUse` lifecycle hooks. Each forwards its event to the Parallax evaluation server, which applies the configured rule chain.

The `PreToolUse` and `PostToolUse` hooks require a Codex version with the hooks engine (stable since Codex CLI v0.124.0).

The integration is a native Rust binary — no Node.js or Python runtime required.

## Setup

```bash
parallax serve -c config.yaml
parallax setup codex
```

`parallax setup codex` writes the hooks into `~/.codex/config.toml` (honoring `CODEX_HOME` if set), making them active for all Codex sessions on this machine.

The generated config looks like this:

```toml
notify = ["/path/to/parallax", "codex", "hook", "notification"]

[[hooks.PreToolUse]]
matcher = ".*"

[[hooks.PreToolUse.hooks]]
type = "command"
command = '"/path/to/parallax" codex hook pre-tool-use'
timeout = 30
statusMessage = "Parallax pre-tool-use check"

[[hooks.PostToolUse]]
matcher = ".*"

[[hooks.PostToolUse.hooks]]
type = "command"
command = '"/path/to/parallax" codex hook post-tool-use'
timeout = 30
statusMessage = "Parallax post-tool-use review"
```

Setup is idempotent and preserves any user-defined hooks: re-running it replaces only Parallax's own entries. `parallax revert codex` removes them, dropping empty hook arrays and the `notify` key.

For a non-default host or port, pass `--host` / `--port` to the setup command. Codex spawns `notify` directly (not via a shell), so its URL is injected through an `env` wrapper; the `PreToolUse`/`PostToolUse` commands run through a shell, so their URL is injected as a `PARALLAX_URL=...` prefix:

```bash
parallax setup codex --host 0.0.0.0 --port 9999
# notify: ["env", "PARALLAX_URL=http://0.0.0.0:9999/evaluate", "/path/to/parallax", "codex", "hook", "notification"]
# hook command: PARALLAX_URL=http://0.0.0.0:9999/evaluate "/path/to/parallax" codex hook pre-tool-use
```

| Variable | Default | Description |
|----------|---------|-------------|
| `PARALLAX_URL` | `http://127.0.0.1:9920/evaluate` | Evaluation endpoint |
| `PARALLAX_TIMEOUT` | `3000` | Request timeout in ms |
| `CODEX_HOME` | `~/.codex` | Codex config directory |

## What the integration evaluates

The setup wires all three hooks:

| Hook | Command | Stage | Behavior |
|------|---------|-------|----------|
| `notification` | `parallax codex hook notification` | `message.before` | Fire-and-forget — Codex's `notify` cannot block a turn |
| `pre-tool-use` | `parallax codex hook pre-tool-use` | `tool.before` | Blocks the tool call if the verdict is `block` (exits `2`) |
| `post-tool-use` | `parallax codex hook post-tool-use` | `tool.after` | Fire-and-forget — logs but doesn't block |

The `PreToolUse` hook reads the Codex event from stdin (`tool_name`, `tool_input`, `session_id`, `turn_id`) and blocks the call on a `block` verdict by exiting `2`. The standalone binary supports the same commands for wrappers that feed tool events manually.

A Codex notify payload is mapped as follows:

| Codex field | Parallax field |
|-------------|----------------|
| `turn-id` | `session_id` |
| `last-assistant-message` (or joined `input-messages`) | `message_text` |
| `type` | `event_type` |

## Event input

Codex passes the event JSON as a trailing argument to the `notify` program:

```bash
parallax codex hook notification '{"type":"agent-turn-complete","turn-id":"t-1","last-assistant-message":"Done!"}'
```

When no argument is supplied, the hook reads the event from stdin instead — useful for testing and for callers that pipe events.

## Fails open

If Parallax is unreachable or returns an error, all hooks exit `0` and allow the turn to proceed. This matches the behavior of the Claude Code and OpenClaw integrations.

## Standalone binary

The `integrations/codex` crate also produces a standalone `parallax-codex-hooks` binary that implements the same logic without the rest of the parallax CLI:

```bash
cargo build --release --manifest-path integrations/codex/Cargo.toml
# binary: integrations/codex/target/release/parallax-codex-hooks

parallax-codex-hooks notification '{"type":"agent-turn-complete","last-assistant-message":"Done!"}'
parallax-codex-hooks pre-tool-use
parallax-codex-hooks post-tool-use
```

This is useful when you want to install only the hook binary without the full `parallax` binary.
