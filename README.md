<div align="center">

# 🛡️ Parallax 🛡️

### Runtime security for AI agents — block prompt injection, data exfiltration, and dangerous tool calls

**One binary · One YAML · Microsecond decisions · Any framework, any LLM**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Rust 1.70+](https://img.shields.io/badge/rust-1.70%2B-orange.svg?logo=rust)](https://www.rust-lang.org)
[![Release](https://img.shields.io/github/v/release/agent-defense/parallax?color=brightgreen)](https://github.com/agent-defense/parallax/releases)
[![Stars](https://img.shields.io/github/stars/agent-defense/parallax?style=social)](https://github.com/agent-defense/parallax/stargazers)

[**Quick Start**](#-quick-start) · [**Docs**](docs/) · [**Architecture**](docs/ARCHITECTURE.md) · [**Rules**](docs/RULES.md) · [**Roadmap**](#-roadmap)

</div>

---

## 🛡️ Why Parallax

- **Single binary, zero runtime dependencies** -- `cargo build --release` produces one static executable. No Python, no JVM, no containers required.
- **Microsecond evaluation** -- the evaluator chain runs in cost order and short-circuits on the first `block`. Typical decisions complete in under 0.2 ms.
- **Framework-agnostic** -- works with any agent system that can make HTTP calls. First-class integrations for OpenClaw and Claude Code; LangChain, CrewAI, and OpenAI Agents SDK are on the roadmap.
- **54 rules out of the box** -- ships with rules covering 13 threat categories: prompt injection, reconnaissance, privilege escalation, PII leakage, supply chain attacks, data exfiltration, and more.
- **Five evaluator engines** -- regex, keyword pattern, Sigma, CEL expressions, and SQL-based temporal analysis. Mix and match for layered defense.

## ⚙️ How It Works

![How Parallax processes an event: agent event flows through the evaluator chain (regex, pattern, sigma, cel, sql) to a decision (allow, detect, redact, block) and finally to the audit log and webhook.](docs/assets/how-it-works.svg)

Each event carries a lifecycle stage (`message.before`, `tool.before`, `tool.after`, `params.before`). Evaluators run cheapest-first and short-circuit on the first `block`; otherwise results are aggregated by severity (`block` > `redact` > `detect` > `allow`).

## 🚀 Quick Start

### 1. Get the binary

**Option A — Download a release** (fastest)

Grab a pre-built binary for your platform from [GitHub Releases](https://github.com/agent-defense/parallax/releases/latest).

**Option B — Build from source**

```bash
git clone https://github.com/agent-defense/parallax
cd parallax
cargo build --release
```

Requires [Rust](https://rustup.rs/) 1.70+. No other dependencies.

### 2. Start the server

```bash
./parallax serve
```

This auto-discovers `parallax.yaml` in the current directory. If a `rules/` directory sits next to it, the full curated rule set under [`rules/`](rules/) is auto-discovered too (one evaluator per file, each declaring its own `stages:`). Drop `rules/` and only the inline starter rules in `parallax.yaml` load — useful for a stripped-down install.

To point at a config in another location:

```bash
./parallax serve -c /path/to/parallax.yaml
```

### 3. Test it

```bash
curl http://127.0.0.1:9920/health

curl -X POST http://127.0.0.1:9920/evaluate \
  -H 'Content-Type: application/json' \
  -d '{"stage":"tool.before","tool_name":"exec","tool_args":{"command":"rm -rf /"}}'
# → {"action":"block","blocked":true,"reasons":["Regex match: Recursive delete"]}
```

Your agent calls `POST /evaluate` before and after each tool execution and acts on the decision.

### 4. Connect to an agent framework (optional)

Use `parallax setup <framework>` and `parallax revert <framework>` for framework-specific configuration. OpenClaw supports proxy and server modes; Claude Code uses lifecycle hooks. See [Agent Framework Integrations](#agent-framework-integrations) for the commands and links to the detailed setup guides.

## 🎯 Supported Threat Categories

| Category | Evaluator | Coverage |
|----------|-----------|----------|
| Prompt injection & jailbreak | Sigma | System prompt extraction, DAN mode, role-play escape |
| Secret leakage | Regex | AWS keys, GitHub tokens, private keys, generic API keys |
| PII exposure | Regex | SSN, credit cards, phone numbers |
| Data exfiltration | Regex | Base64-encoded secrets, hex payloads, data URIs |
| Dangerous commands | Regex + CEL | `rm -rf`, disk format, curl-pipe-bash |
| Privilege escalation | CEL | sudo, su, pkexec, setuid, sudoers |
| Reconnaissance | Sigma | Credential files, cloud metadata endpoints, container configs |
| Shadow IT | Sigma | Docker, Kubernetes, Terraform, cloud CLI |
| Supply chain attacks | Pattern | Custom package indexes, registry hijacking |
| SQL injection | Pattern | DROP TABLE, DELETE FROM, TRUNCATE |
| Model manipulation | CEL | System prompt tampering, temperature override, tool redefinition |
| Resource abuse | SQL | Rate limiting, repeated tool abuse |
| Sensitive file writes | Sigma | Writes to /etc, /usr, .ssh |

See [docs/RULES.md](docs/RULES.md) for the full reference.

## 📝 Configuration

One YAML file, four sections.

### Server

```yaml
server:
  host: "127.0.0.1"
  port: 9920
```

### Reporting

```yaml
reporting:
  log_file: ./logs/audit.jsonl          # Append-only JSONL audit trail
  webhook_url: https://siem.example.com # POST decisions to external systems
  webhook_events: [block, redact]       # Filter which decisions to send
```

### Evaluators (inline starter rules)

Inline evaluators are short, hand-picked rules that ship in [`parallax.yaml`](parallax.yaml) so a bare `parallax serve` (no rules tree) still blocks the obvious. Each has a `name`, `type`, the `stages` it applies to, and inline `rules`:

```yaml
evaluators:
  - name: starter-dangerous-commands
    type: regex
    stages: [tool.before]
    rules:
      - id: cmd-001
        title: Recursive delete root
        description: Blocks recursive deletion of root filesystem
        pattern: "rm\\s+-[a-zA-Z]*r[a-zA-Z]*f[a-zA-Z]*\\s+/"
        action: block
        fields: [tool_args.command]
```

### Rules tree (auto-discovered)

If a `rules/` directory sits next to `parallax.yaml` (or you point `rules_dir:` at one), every file under `<rules_dir>/<engine>/*.yaml` is auto-loaded as its own evaluator. Each rule file declares a mandatory root-level `stages:` array followed by a flat `rules:` list:

```yaml
stages: [tool.before]

rules:
  - id: sc-001
    title: Custom PyPI index
    description: Detects pip installs from non-default package indexes
    keywords: ["--index-url ", "--extra-index-url "]
    action: detect
    priority: medium
```

Every rule carries the mandatory fields `id`, `title`, `description`, `action`, `priority` (plus engine-specific fields like `pattern`, `keywords`, `expr`, or `query`). A file missing the root-level `stages:` array is rejected at load time.

When an inline rule id (in `evaluators:`) collides with a discovered rule id (in `rules/`), the discovered version wins — so dropping a `rules/` tree in cleanly upgrades the inline starter to the full curated set.

### Disabling evaluators

```yaml
disabled:
  - pii                    # auto-discovered evaluator name (filename stem)
```

See [parallax.yaml](parallax.yaml) for the shipped config and [`rules/`](rules/) for the curated rule library.

## Evaluator Types

| Type | Description | Rule sources |
|------|-------------|-------------|
| **regex** | Compiled regex patterns with AND/OR, negation, field targeting, redaction | inline `rules`, `rules_file`, or `rules_dir` |
| **pattern** | Keyword substring matching, case-insensitive | inline `rules`, `rules_file`, or `rules_dir` |
| **sigma** | Sigma-format YAML threat detection with field modifiers and complex conditions | `rules_dir` of multi-document Sigma YAML |
| **cel** | CEL-like expressions (`==`, `!=`, `&&`, `.contains()`, `.startsWith()`, `.matches()`) | inline `rules`, `rules_file`, or `rules_dir` |
| **sql** | In-memory SQLite for rate limiting, frequency analysis, temporal patterns | inline `rules`, `rules_file`, or `rules_dir` |

Evaluators run in cost order (cheapest first) and short-circuit on block.

## Decisions

| Action | Behavior |
|--------|----------|
| `block` | Reject the event |
| `redact` | Replace matched content with `[REDACTED]`, then allow |
| `detect` | Log and alert, but allow |
| `allow` | Pass through |

## Stages

| Stage | When | Can block? |
|-------|------|------------|
| `message.before` | User message received | Yes |
| `tool.before` | Before tool execution | Yes |
| `tool.after` | After tool execution | Yes |
| `params.before` | Before model parameter forwarding | Yes |

## 🌐 Two Modes

### Server Mode (default)

Exposes a `/evaluate` HTTP endpoint. Your agent calls it at each lifecycle stage and acts on the decision.

```bash
parallax serve            # auto-discovers ./parallax.yaml + ./rules/
parallax serve -c /etc/parallax/parallax.yaml
```

**POST /evaluate**

```json
// Request
{ "stage": "tool.before", "session_id": "s-123", "tool_name": "exec", "tool_args": {"command": "rm -rf /"} }

// Response
{ "action": "block", "blocked": true, "reasons": ["Regex match: Recursive delete"], "elapsed_ms": 0.1 }
```

**GET /health**

```json
{ "status": "ok", "mode": "server", "evaluators": 3, "version": "0.2.0" }
```

### Proxy Mode

Acts as a reverse proxy between your agent and the LLM API. All traffic is automatically evaluated -- no integration code needed.

```bash
parallax serve --mode proxy
```

```
  Agent ──> POST /anthropic/v1/messages ──> Parallax ──> Anthropic API
                                               │
                                    ┌──────────┼──────────┐
                                    │          │          │
                              message.before tool.after tool.before
                                    │          │          │
                               Block before  Scan tool  Intercept tool_use
                               forwarding    results    in SSE stream
```

The proxy:
- Evaluates user messages before forwarding (`message.before`)
- Evaluates tool results in the request (`tool.after`)
- Buffers and evaluates `tool_use` blocks in streaming responses (`tool.before`)
- Replaces blocked tool calls with text explanations
- Passes through non-messages endpoints transparently

## 🔌 Agent Framework Integrations

### Any agent system (HTTP API)

Parallax works with any agent that can make HTTP requests. POST to `/evaluate`:

| Field | Required | Description |
|-------|----------|-------------|
| `stage` | Yes | `message.before`, `tool.before`, `tool.after`, or `params.before` |
| `session_id` | No | Session identifier |
| `tool_name` | No | Tool being called |
| `tool_args` | No | Tool arguments |
| `tool_result` | No | Tool output (for `tool.after`) |
| `message_text` | No | Message content (for `message.before`) |

Check `blocked` in the response to decide whether to proceed.

### OpenClaw

Parallax includes a dedicated integration for [OpenClaw](https://openclaw.ai) agent systems. Proxy mode routes OpenClaw traffic through Parallax; server mode uses the plugin under `./integrations/openclaw`. See [docs/integrations/openclaw.md](docs/integrations/openclaw.md) for full setup instructions.

### Claude Code

Parallax includes a dedicated integration for [Claude Code](https://claude.ai/code) agent systems. It writes lifecycle hooks into `.claude/settings.json` and can be installed per-project or copied to a user-level Claude config. See [docs/integrations/claudecode.md](docs/integrations/claudecode.md) for full setup instructions.

### Codex CLI

Parallax includes a dedicated integration for [Codex CLI](https://github.com/openai/codex). It writes `notify`, `PreToolUse`, and `PostToolUse` hooks into `~/.codex/config.toml` that forward agent events to the Parallax evaluation server — `PreToolUse` can block a tool call before it runs. See [docs/integrations/codex.md](docs/integrations/codex.md) for full setup instructions.

## CLI Reference

```
parallax serve [OPTIONS]
  -c, --config <PATH>       Config file path
      --host <HOST>         Override host
      --port <PORT>         Override port
      --mode <MODE>         server or proxy [default: server]
      --log-level <LEVEL>   Log level [default: info]

parallax setup <COMMAND>
  openclaw   Configure OpenClaw to route through Parallax
  claudecode Configure Claude Code hooks to route through Parallax
  codex      Configure Codex CLI hooks to route through Parallax

parallax setup openclaw [OPTIONS]
      --host <HOST>         Proxy host [default: 127.0.0.1]
      --port <PORT>         Proxy port [default: 9920]
      --model <MODEL>       Model ID [default: claude-sonnet-4-20250514]

parallax setup claudecode [OPTIONS]
      --host <HOST>         Proxy host [default: 127.0.0.1]
      --port <PORT>         Proxy port [default: 9920]

parallax setup codex [OPTIONS]
      --host <HOST>         Proxy host [default: 127.0.0.1]
      --port <PORT>         Proxy port [default: 9920]

parallax revert <COMMAND>
  openclaw   Revert OpenClaw to use Anthropic directly
  claudecode Revert Claude Code hooks
  codex      Revert Codex CLI hooks

parallax revert openclaw [OPTIONS]
      --model <MODEL>       Model ID [default: claude-sonnet-4-20250514]

parallax revert claudecode

parallax revert codex
```

Supported frameworks: `openclaw`, `claudecode`, `codex`.

## 🗺️ Roadmap

### -- Multi-Framework & Multi-Provider Support
- Generic `parallax setup <name>` for LangChain, CrewAI, OpenAI Agents SDK
- Integration directory structure for framework integrations
- OpenAI-compatible proxy mode (`/v1/chat/completions`) covering OpenAI, Azure OpenAI, and local models (Ollama, LM Studio)
- Configurable upstream provider in `parallax.yaml`

### -- Advanced Evaluators
- Embedding-based semantic prompt injection detection
- Tool argument JSON Schema validation
- Multi-turn escalation detection across conversation history
- Token budget enforcement per session/user

### -- Extended Lifecycle Stages
- `response.after` -- evaluate LLM responses before returning to the user
- `memory.before` -- evaluate before writing to agent memory/context
- RAG pipeline stages (`retrieval.before`, `retrieval.after`)
- Rule hot-reload -- watch config file for changes without restart

### -- SDKs and Ecosystem
- Python client library (`pip install parallax-client`) with LangChain/CrewAI decorators
- TypeScript client library (`npm install @parallax/client`)
- Webhook integrations -- Slack, PagerDuty, and SIEM connectors
- Dashboard UI for rule management and audit log visualization

## 🏗️ Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for details on the evaluator chain, short-circuit logic, and cost-ordered execution.

## Development

```bash
cargo build            # Dev build
cargo test             # Run tests (45 tests)
cargo build --release  # Optimized release build
RUST_LOG=debug cargo run -- serve
```

## 📄 License

Apache 2.0

---

<div align="center">

Made with 🦀 in Rust · [Report a bug](https://github.com/agent-defense/parallax/issues) · [Request a feature](https://github.com/agent-defense/parallax/issues/new)

</div>
