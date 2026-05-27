# Codex CLI Integration

Parallax integrates with [Codex CLI](https://github.com/openai/codex) via its `notify` hook. Codex spawns the configured `notify` program when an agent turn completes, passing the event as a JSON argument. The hook forwards the event to the Parallax evaluation server, which applies the configured rule chain.

The integration is a native Rust binary — no Node.js or Python runtime required.

## Setup

```bash
parallax serve -c config.yaml
parallax setup codex
```

`parallax setup codex` writes a `notify` entry into `~/.codex/config.toml` (honoring `CODEX_HOME` if set), making it active for all Codex sessions on this machine.

The generated entry looks like this:

```toml
notify = ["/path/to/parallax", "codex", "hook", "notification"]
```

For a non-default host or port, pass `--host` / `--port` to the setup command. Because Codex spawns `notify` directly (not via a shell), the URL is injected through an `env` wrapper:

```bash
parallax setup codex --host 0.0.0.0 --port 9999
# writes: notify = ["env", "PARALLAX_URL=http://0.0.0.0:9999/evaluate", "/path/to/parallax", "codex", "hook", "notification"]
```

| Variable | Default | Description |
|----------|---------|-------------|
| `PARALLAX_URL` | `http://127.0.0.1:9920/evaluate` | Evaluation endpoint |
| `PARALLAX_TIMEOUT` | `3000` | Request timeout in ms |
| `CODEX_HOME` | `~/.codex` | Codex config directory |

## What the integration evaluates

Codex's `notify` program is **fire-and-forget** — it cannot block a turn. The setup wires the `notification` hook, which forwards Codex notify events (e.g. `agent-turn-complete`) to Parallax:

| Hook | Command | Stage | Behavior |
|------|---------|-------|----------|
| `notification` | `parallax codex hook notification` | `message.before` | Fire-and-forget — logs but doesn't block |
| `pre-tool-use` | `parallax codex hook pre-tool-use` | `tool.before` | Sequential — blocks if verdict is `block` (exits `2`) |
| `post-tool-use` | `parallax codex hook post-tool-use` | `tool.after` | Fire-and-forget — logs but doesn't block |

The `pre-tool-use` / `post-tool-use` hooks are exposed for the standalone binary and for wrappers that can feed tool events; Codex's native `notify` only drives `notification`.

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
