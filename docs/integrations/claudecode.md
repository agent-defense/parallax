# Claude Code Integration

Parallax integrates with [Claude Code](https://claude.ai/code) via its lifecycle hooks system. Hook scripts forward events to the Parallax evaluation server, which applies the configured rule chain and returns a verdict.

## Setup

```bash
parallax serve -c config.yaml
cd integrations/claudecode && npm install
parallax setup claudecode
```

The hooks run via [tsx](https://tsx.is/) and are installed through `npm install` in `integrations/claudecode`. `parallax setup claudecode` writes the project hook entries into `.claude/settings.json`; copy the same entries to `~/.claude/settings.json` if you want user-level hooks across projects.

| Variable | Default | Description |
|----------|---------|-------------|
| `PARALLAX_URL` | `http://127.0.0.1:9920/evaluate` | Evaluation endpoint |
| `PARALLAX_TIMEOUT` | `3000` | Request timeout in ms |

## What the integration evaluates

| Hook | Stage | Behavior |
|------|-------|----------|
| `PreToolUse` | `tool.before` | Sequential — blocks the tool call if verdict is `block` |
| `PostToolUse` | `tool.after` | Fire-and-forget — logs but doesn't block |
| `Notification` | `message.before` | Fire-and-forget — logs but doesn't block |

## Blocking behavior

When `PreToolUse` receives a `block` verdict from Parallax, the hook exits with code `2` and writes a JSON decision to stdout:

```json
{"decision": "block", "reason": "Matched rule: dangerous-command"}
```

Claude Code intercepts this and surfaces the reason to the user instead of executing the tool call.

## Fails open

If Parallax is unreachable or returns an error, all hooks exit `0` and allow the tool call to proceed. This matches the server-mode behavior of the OpenClaw integration.
