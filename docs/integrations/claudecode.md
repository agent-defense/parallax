# Claude Code Integration

Parallax integrates with [Claude Code](https://claude.ai/code) via its lifecycle hooks system. Hook commands forward events to the Parallax evaluation server, which applies the configured rule chain and returns a verdict.

The integration is a native Rust binary — no Node.js or TypeScript runtime required.

## Setup

```bash
parallax serve -c config.yaml
parallax setup claudecode
```

`parallax setup claudecode` writes the hook entries into `~/.claude/settings.json`, making them active for all projects on this machine.

The generated entries look like this:

```json
{
  "hooks": {
    "PreToolUse": [{"matcher": ".*", "hooks": [{"type": "command", "command": "parallax claudecode hook pre-tool-use"}]}],
    "PostToolUse": [{"matcher": ".*", "hooks": [{"type": "command", "command": "parallax claudecode hook post-tool-use"}]}],
    "Notification": [{"hooks": [{"type": "command", "command": "parallax claudecode hook notification"}]}]
  }
}
```

For a non-default host or port, pass `--host` / `--port` to the setup command and the env var will be prepended automatically:

```bash
parallax setup claudecode --host 0.0.0.0 --port 9999
# writes: PARALLAX_URL=http://0.0.0.0:9999/evaluate parallax claudecode hook pre-tool-use
```

| Variable | Default | Description |
|----------|---------|-------------|
| `PARALLAX_URL` | `http://127.0.0.1:9920/evaluate` | Evaluation endpoint |
| `PARALLAX_TIMEOUT` | `3000` | Request timeout in ms |

## What the integration evaluates

Each hook reads the Claude Code event JSON from stdin and forwards it to Parallax:

| Hook | Command | Stage | Behavior |
|------|---------|-------|----------|
| `PreToolUse` | `parallax claudecode hook pre-tool-use` | `tool.before` | Sequential — blocks the tool call if verdict is `block` |
| `PostToolUse` | `parallax claudecode hook post-tool-use` | `tool.after` | Fire-and-forget — logs but doesn't block |
| `Notification` | `parallax claudecode hook notification` | `message.before` | Fire-and-forget — logs but doesn't block |

## Blocking behavior

When `PreToolUse` receives a `block` verdict from Parallax, the hook exits with code `2` and writes a JSON decision to stdout:

```json
{"decision": "block", "reason": "Matched rule: dangerous-command"}
```

Claude Code intercepts this and surfaces the reason to the user instead of executing the tool call.

## Fails open

If Parallax is unreachable or returns an error, all hooks exit `0` and allow the tool call to proceed. This matches the server-mode behavior of the OpenClaw integration.

## Standalone binary

The `integrations/claudecode` crate also produces a standalone `parallax-hooks` binary that implements the same logic without the rest of the parallax CLI:

```bash
cargo build --release --manifest-path integrations/claudecode/Cargo.toml
# binary: integrations/claudecode/target/release/parallax-hooks

parallax-hooks pre-tool-use
parallax-hooks post-tool-use
parallax-hooks notification
```

This is useful when you want to install only the hook binary without the full `parallax` binary.
