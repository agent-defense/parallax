# Claude Code Integration

Parallax integrates with [Claude Code](https://claude.ai/code) via its lifecycle hooks system. Hook scripts forward events to the Parallax evaluation server, which applies the configured rule chain and returns a verdict.

## Setup

```bash
# 1. Start Parallax in server mode
parallax serve -c config.yaml

# 2. Install hook dependencies
cd integrations/claudecode && npm install

# 3. Copy the settings snippet into your Claude Code settings
#    Project-level:  .claude/settings.json
#    User-level:     ~/.claude/settings.json
cp integrations/claudecode/settings.json .claude/settings.json

# 4. Update the script paths in .claude/settings.json to match your Parallax install location
```

## Requirements

The hooks run via [tsx](https://tsx.is/) — a zero-config TypeScript runner. It is listed as a dependency in `integrations/claudecode/package.json` and installed by `npm install`. Alternatively, `tsx` can be installed globally:

```bash
npm install -g tsx
```

## Configuration

Replace `/path/to/parallax` in `.claude/settings.json` with the actual path to your Parallax directory, or use the absolute path from `pwd`.

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
