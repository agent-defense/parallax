# Security Rules Reference

Parallax ships with **54 active rules across 13 threat categories** (plus one detect-only swap rule shipped commented-out), providing defense-in-depth for AI agent systems. Rules live under [`rules/`](../rules), grouped by evaluator engine (`regex/`, `pattern/`, `cel/`, `sigma/`, `sql/`), and are **auto-discovered at startup** — one evaluator per file, named after the filename stem. Every rule declares its own mandatory `stages:` array.

**Design principles:**

- **Cost-ordered evaluation** -- cheap evaluators (regex, pattern) run before expensive ones (SQL). The chain short-circuits on the first `block`, so most events are resolved in microseconds.
- **Layered defense** -- the same threat may be caught by multiple rule types. A dangerous `rm -rf /` is caught by both a regex rule and a CEL policy. This redundancy is intentional.
- **Tunable severity** -- rules use `block`, `redact`, or `detect` actions. Switch a rule from `block` to `detect` to monitor before enforcing.
- **Normalized metadata** -- all rule types share the same core metadata fields: `id`, `title`, and `description`. This enables consistent filtering, reporting, and auditing across evaluator engines.

---

## Rule Metadata

Every rule, regardless of evaluator engine, has three standard metadata fields:

| Field | Required | Description |
|-------|----------|-------------|
| `id` | Yes | Unique identifier for the rule, formatted `<category>-NNN` (e.g. `pe-001`, `sec-003`, `pi-001`) |
| `title` | Yes | Short human-readable name |
| `description` | Yes | Longer explanation of what the rule detects and why |

When a rule triggers, `id`, `title`, and `description` are included in the evaluation result metadata alongside engine-specific fields.

**ID conventions by category:**

Every rule ID uses the form `<category>-NNN`. The category prefix is the canonical key for backend grouping; the engine that implements the rule is implied by the directory under `rules/` and is _not_ encoded in the ID.

| Category | Prefix | Engine | Example |
|----------|--------|--------|---------|
| Secrets | `sec` | regex | `sec-001` |
| PII | `pii` | regex | `pii-003` |
| Data exfiltration | `exfil` | regex | `exfil-002` |
| Dangerous commands | `cmd` | regex | `cmd-001` |
| SQL injection | `sql` | pattern | `sql-001` |
| Supply chain | `sc` | pattern | `sc-002` |
| General policies | `pol` | cel | `pol-001` |
| Privilege escalation | `pe` | cel | `pe-003` |
| Model manipulation | `mm` | cel | `mm-002` |
| Rate limiting | `rl` | sql | `rl-001` |
| Dangerous tools | `dt` | sigma | `dt-001` |
| Prompt injection | `pi` | sigma | `pi-002` |
| Reconnaissance | `recon` | sigma | `recon-003` |
| Shadow IT | `shadow` | sigma | `shadow-001` |

---

## Prompt Injection

Detects attempts to extract system prompts, jailbreak the model, or override safety constraints.

**Engine:** Sigma | **File:** `rules/sigma/prompt-injection.yaml` | **Stage:** `message.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| pi-001 | System prompt extraction attempt | Detects messages attempting to extract the system prompt or initial instructions | Block |
| pi-002 | Jailbreak pattern detected | Detects common jailbreak and guardrail bypass attempts | Block |
| pi-003 | Role-play escape attempt | Detects attempts to redefine the agent role or bypass constraints via role-play | Block |

**False-positive notes:** Legitimate discussions about AI safety or prompt engineering may trigger pi-001/pi-002. Consider switching to `detect` in research/educational environments.

---

## Secret Scanning

Redacts or blocks common secret patterns before they leak through tool calls or responses.

**Engine:** Regex | **Evaluator:** `secrets-scanner` | **File:** `rules/regex/secrets.yaml` | **Stage:** `tool.before`, `tool.after`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| sec-001 | AWS Access Key | Detects AWS access key IDs starting with AKIA | Redact |
| sec-002 | AWS Secret Key | Detects AWS secret access keys in configuration or environment variables | Redact |
| sec-003 | GitHub Personal Access Token | Detects GitHub personal access tokens (classic format) | Redact |
| sec-004 | GitHub Fine-Grained Token | Detects GitHub fine-grained personal access tokens | Redact |
| sec-005 | Generic API Key | Detects generic API keys and secret keys in assignments | Redact |
| sec-006 | Private Key Block | Detects PEM-encoded private key blocks | Block |

**False-positive notes:** The generic API key pattern may match configuration documentation that contains placeholder keys. Use the `fields` option to restrict matching to specific fields if needed.

---

## PII Exposure

Redacts personally identifiable information from tool outputs before they reach the user or external systems.

**Engine:** Regex | **Evaluator:** `pii-scanner` | **File:** `rules/regex/pii.yaml` | **Stage:** `tool.after`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| pii-001 | Social Security Number | Detects US Social Security Numbers in NNN-NN-NNNN format | Redact |
| pii-002 | Credit Card - Visa | Detects Visa credit card numbers | Redact |
| pii-003 | Credit Card - Mastercard | Detects Mastercard credit card numbers | Redact |
| pii-004 | Credit Card - Amex | Detects American Express credit card numbers | Redact |
| pii-005 | US Phone Number | Detects US phone numbers in various formats | Redact |

**False-positive notes:** The SSN pattern will match any `NNN-NN-NNNN` format, including some date formats and version numbers. The phone number pattern may match numeric sequences in technical output.

---

## Data Exfiltration

Detects encoded data and indicators of data being prepared for exfiltration.

**Engine:** Regex | **Evaluator:** `data-exfiltration` | **File:** `rules/regex/data-exfiltration.yaml` | **Stage:** `tool.before`, `tool.after`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| exfil-001 | Base64-encoded secret indicator | Detects base64-encoded values following secret-like key names | Detect |
| exfil-002 | Long hex-encoded data block | Detects long hex-encoded strings that may indicate data exfiltration | Detect |
| exfil-003 | Data URI with base64 | Detects data URIs with large base64 payloads | Detect |

---

## Dangerous Commands

Blocks dangerous shell commands before they execute. Covered by multiple engines for defense-in-depth.

**Engine:** Regex | **Evaluator:** `dangerous-commands` | **File:** `rules/regex/dangerous-commands.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| cmd-001 | Recursive delete root | Blocks recursive deletion of root filesystem | Block |
| cmd-002 | Format disk | Blocks filesystem formatting commands | Block |
| cmd-003 | Disk overwrite | Blocks raw disk writes via dd to device files | Block |
| cmd-004 | Chmod 777 recursive | Blocks recursive permission changes to world-writable | Block |
| cmd-005 | Curl pipe to shell | Blocks piping curl output directly to a shell interpreter | Block |

**Engine:** CEL | **File:** `rules/cel/policies.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| pol-001 | Block recursive file deletion | Prevents recursive file deletion via rm -r commands | Block |
| pol-002 | Block world-writable permissions | Prevents setting chmod 777 which makes files world-writable | Block |
| pol-003 | Detect sudo usage | Detects elevated privilege execution via sudo | Detect |
| pol-004 | Detect environment variable dump | Detects environment variable dumps that may expose secrets | Detect |

---

## Privilege Escalation

Detects attempts to gain elevated permissions.

**Engine:** CEL | **File:** `rules/cel/privilege-escalation.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| pe-001 | Block sudo privilege escalation | Blocks privilege escalation via sudo command | _Disabled by default_ |
| pe-002 | Block user switching | Blocks switching to another user via su command | Block |
| pe-003 | Block pkexec privilege escalation | Blocks privilege escalation via pkexec | Block |
| pe-004 | Block doas privilege escalation | Blocks privilege escalation via doas command | Block |
| pe-005 | Block chown to root | Blocks changing file ownership to root | Block |
| pe-006 | Block setuid bit | Blocks setting the setuid bit on files | Block |
| pe-007 | Block sudoers modification | Blocks modifying the sudoers configuration | Block |

**Default sudo policy:** `pe-001` ships commented out so `sudo` is detect-only via `pol-003` (Dangerous Commands section). To enforce a hard block, uncomment `pe-001` in `rules/cel/privilege-escalation.yaml` and consider removing `pol-003` to avoid duplicate metadata in the audit log.

---

## Reconnaissance

Detects attempts to access credential files, system configuration, or cloud metadata.

**Engine:** Sigma | **File:** `rules/sigma/reconnaissance.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| recon-001 | Sensitive file read - credentials and keys | Detects tool calls reading credential files, SSH keys, or secret stores | Block |
| recon-002 | System file reconnaissance | Detects reads of system files commonly targeted for information gathering | Block |
| recon-003 | Cloud metadata endpoint access | Detects access to cloud instance metadata services used for credential theft | Block |
| recon-004 | Container and orchestration config access | Detects reads of Kubernetes, Docker, and container configuration files | Block |

---

## Sensitive File Writes

Detects tool calls that write to critical system directories.

**Engine:** Sigma | **File:** `rules/sigma/dangerous-tools.yaml` | **Stage:** `tool.before`, `tool.after`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| dt-001 | Suspicious file write outside workspace | Detects tool calls that write files to sensitive system directories | Block |
| dt-002 | Shell command with network exfiltration indicators | Detects exec tool calls that combine data access with network transfer | Block |
| dt-003 | Database credential access via tool | Detects tool calls that read database configuration or credential files | Block |

---

## Shadow IT

Detects agents creating infrastructure or modifying system resources without approval.

**Engine:** Sigma | **File:** `rules/sigma/shadow-it.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| shadow-001 | Container runtime operations | Detects agent creating, running, or building containers without approval | Block |
| shadow-002 | Kubernetes cluster operations | Detects agent deploying or modifying Kubernetes resources | Block |
| shadow-003 | Cloud infrastructure provisioning | Detects agent provisioning or modifying cloud infrastructure | Block |
| shadow-004 | User account management | Detects agent creating or modifying system user accounts | Block |

**False-positive notes:** DevOps agents that legitimately manage infrastructure should selectively disable shadow-001 through shadow-003 while keeping shadow-004 active.

---

## Model Manipulation

Detects attempts to tamper with model parameters or redefine tool behavior.

**Engine:** CEL | **File:** `rules/cel/model-manipulation.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| mm-001 | Block system prompt injection via tools | Detects attempts to modify the system prompt through tool calls | Block |
| mm-002 | Detect temperature override attempt | Detects attempts to modify the model temperature parameter | Detect |
| mm-003 | Detect tool definitions tampering | Detects attempts to modify or redefine available tool definitions | Detect |
| mm-004 | Detect max_tokens override attempt | Detects attempts to modify the max_tokens parameter | Detect |

---

## Supply Chain

Blocks untrusted package installs and dependency confusion attacks.

**Engine:** Pattern | **Evaluator:** `supply-chain` | **File:** `rules/pattern/supply-chain.yaml` | **Stage:** `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| sc-001 | Pip install from custom index | Blocks pip installs from non-default package indexes | Block |
| sc-002 | Npm install from custom registry | Blocks npm installs from non-default registries | Block |
| sc-003 | Wget pipe to shell | Blocks piping wget output directly to a shell interpreter | Block |
| sc-004 | Gem install from custom source | Blocks gem installs from non-default sources | Block |

---

## SQL Injection

Detects common SQL injection patterns in messages and tool arguments.

**Engine:** Pattern | **Evaluator:** `sql-keywords` | **File:** `rules/pattern/sql-keywords.yaml` | **Stage:** `message.before`, `tool.before`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| sql-001 | SQL destructive keywords | Detects common SQL injection and destructive query patterns | Detect |

**False-positive notes:** Database administration agents will regularly trigger this rule. Switch to `allow` for trusted database management workflows.

---

## Resource Abuse

Detects abnormal tool usage rates that may indicate automated abuse or infinite loops.

**Engine:** SQL | **Evaluator:** `rate-limits` | **File:** `rules/sql/rate-limits.yaml` | **Stage:** `tool.before`, `tool.after`

| ID | Title | Description | Action |
|----|-------|-------------|--------|
| rl-001 | High tool call rate | Detects unusually high tool call rates per session | Detect |
| rl-002 | Repeated tool abuse | Detects repeated calls to the same tool in a short window | Detect |

---

## Writing Custom Rules

All custom rules should include the three standard metadata fields: `id`, `title`, and `description`.

### Where rules live

Three loading paths exist; in order of preference:

| Source | Used by | Behavior |
|--------|---------|----------|
| `rules/` auto-discovery | `regex`, `pattern`, `cel`, `sql`, `sigma` | Default. The loader walks `./rules/<engine>/*.yaml` and registers one evaluator per file. Each file is a flat YAML list of rules; every rule has a mandatory `stages:` array. Evaluator name = filename stem; evaluator stages = union of its rules' stages. |
| `rules:` (inline) | all | Rules embedded under `evaluators:` in `parallax.yaml`. Used for the shipped starter set. |
| `rules_file: <path>`, `rules_dir: <path>` on an inline evaluator | `regex`, `pattern`, `cel`, `sql`, `sigma` | Explicit external references — appended to that evaluator's `rules:` list at load time. |

When an inline rule id collides with an id from the `rules/` tree, the rules-tree version wins (so the shipped starter rules in `parallax.yaml` are transparently upgraded when the curated tree is present).

To suppress a specific evaluator entirely, add its name (inline or auto-discovered) to the top-level `disabled:` list in `parallax.yaml`.

### Adding a Sigma rule

Drop a `.yaml` file in `rules/sigma/` — the `sigma-threats` evaluator auto-loads it:

```yaml
title: My custom rule
id: custom-001
description: What this rule detects and why it matters
stages: [tool.before]
detection:
  selection:
    tool_name: exec
  pattern:
    tool_args.command|contains:
      - "my-dangerous-command"
  condition: selection and pattern
action: block    # block, detect, redact, or allow
```

Non-Sigma rule files are flat YAML lists. Every rule must include `stages:`
alongside `id`, `title`, `description`, `action`, and `priority`.

### Adding a CEL rule

Append to one of the existing files in `rules/cel/`, or drop a new `rules/cel/<name>.yaml`. Pick a category prefix (`pol`, `pe`, `mm`, or your own) and a free numeric slot:

```yaml
- id: pol-099
  title: My custom CEL rule
  description: Description of what this rule detects
  stages: [tool.before]
  expr: 'tool_name == "exec" && tool_args_command.contains("dangerous")'
  action: block
  priority: high
  reason: Description of why this is blocked
```

### Adding a regex rule

Append to one of the existing files in `rules/regex/`, or drop a new `rules/regex/<name>.yaml`. Pick a category prefix (`sec`, `pii`, `exfil`, `cmd`, or your own) and a free numeric slot:

```yaml
- id: cmd-099
  title: My pattern
  description: Detects a dangerous regex pattern in tool arguments
  stages: [tool.before]
  pattern: "dangerous-regex-here"
  action: block               # block, redact, detect, allow
  priority: high
  fields: [tool_args.command] # optional: target specific fields
```

### Adding a pattern rule

Append to a file in `rules/pattern/`, or drop a new file. Pick a category prefix (`sql`, `sc`, or your own) and a free numeric slot:

```yaml
- id: sc-099
  title: My keyword rule
  description: Detects dangerous keywords in tool arguments
  stages: [tool.before]
  keywords: ["dangerous-keyword"]
  action: block
  priority: high
```

### Adding a SQL rule

Append to `rules/sql/rate-limits.yaml`, or drop a new `rules/sql/<name>.yaml`. Use the `rl` category prefix (or define a new one):

```yaml
- id: rl-099
  title: My aggregate rule
  description: Detects abnormal event patterns using SQL aggregation
  stages: [tool.before, tool.after]
  query: >
    SELECT COUNT(*) as cnt FROM events
    WHERE session_id = :session_id AND timestamp > :now - 120
  condition: "cnt > 10"
  action: detect
  priority: medium
  reason: "What this means"
```

Available SQL parameters: `:session_id`, `:user_id`, `:channel`, `:tool_name`, `:now`
