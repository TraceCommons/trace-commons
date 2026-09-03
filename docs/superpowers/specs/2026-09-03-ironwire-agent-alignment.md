# IronWire and trace acquisition: agent coverage and alignment

Survey date: 2026-09-03. IronWire at `nearai/ironwire` commit 4024619
(2026-09-02). Trace Commons at main `be9bee3b`.

This note answers three questions: which agents IronWire can sit in front
of, which agents the contributor daemon can acquire traces from, and what
it would take to make those two sets the same set.

## 1. What IronWire supports

IronWire is a loopback inference proxy. An "agent" is supported when its
traffic can be pointed at one of the proxy's facades. There are three tiers.

### Tier A: shipped, auto-wired, session-joinable

| Agent | Facade | Config IronWire edits | Session header recorded |
|---|---|---|---|
| Claude Code | `/anthropic` (Messages) | `~/.claude/settings.json` | `x-claude-code-session-id` |
| Codex CLI | `/openai` (Responses) | `~/.codex/config.toml` | `session-id` |

Source: `crates/ironwire_agents/src/tools.rs` (only `built_in_claude` and
`built_in_codex`), `crates/ironwire_upstream/src/headers.rs:55-60`
(`client_session_header`), `crates/ironwire_ledger/src/lib.rs:83`
(`client_session_id` column, landed; the memory note saying PR #17 was
still open is stale).

The Codex desktop app shares the CLI's config and credentials but not its
HTTP stack; ROADMAP lists verifying it as open (M2).

### Tier B: wire-compatible, hand-wired

IronWire's docs name Aider (Chat Completions), Cline and Roo (either
facade), Zed, Amp, and "custom". `docs/PROTOCOL.md:31` says
`/v1/chat/completions` is "Aider, Cline, most third parties". None of
these gets a compiled-in `connect`; the user sets the base URL by hand.
`client_session_header` maps `OpenAiChat` to `session-id`, so a Chat
Completions client is joinable only if it happens to send that header.
Aider and Cline are not known to. Unverified in this survey.

### Tier C: catalog-described (inert today)

`ironwire_catalog::schema::AgentEntry` lets a signed catalog introduce a
new tool as `{id, name, detect, config: {dir, file}, settings: [{key,
facade}]}` for JSON or TOML configs. `ironwire_agents::catalog` fills an
empty key, refuses an occupied one, and removes only its own writes.
ROADMAP row "Real catalog signing key": the compiled-in key is a
placeholder, so the channel verifies nothing and is inert. In practice
Tier C is empty.

### Not supported at all

There is no Google facade. `grep -rli gemini crates docs` in IronWire
returns only the privacy corpus and PRIVACY.md. Gemini CLI, Antigravity,
and anything else that speaks `generateContent` cannot route through
IronWire.

## 2. What trace acquisition supports

The contributor daemon's `TraceSource` implementations
(`crates/trace-commons-contributor/src/source/`):

| Source id | Reads | `conversation_id` set | Settings/UI row |
|---|---|---|---|
| `claude-code` | `~/.claude/projects/**/*.jsonl` | yes (transcript stem) | `claude` |
| `codex` | `~/.codex/sessions/**/rollout-*.jsonl` | yes (UUID suffix of filename) | `codex` |
| `gemini-cli` | `~/.gemini/tmp/<project>/chats/session-*.json` | yes (`sessionId`) | `gemini` |
| `trajectory` | Letta Trajectory v1 files, staged by the contributor | yes (record field) | none |

Plus one import-only path: `antigravity/` discovers the Antigravity IDE's
local language-server API, converts conversations to Trajectory v1, and
stages them through `trajectory`. The daemon never watches Antigravity
directly.

`SourceTool` in `source_copy.rs` (the settings screen) knows exactly
`Claude`, `Codex`, `Gemini`.

IronWire enrichment already exists on our side: `routing/ironwire.rs`
polls `GET /_ironwire/log`, and `routing/enriched.rs` joins each loaded
transcript's `conversation_id` against the ledger's `client_session_id`
(equality, plus a UUID-suffix rule for Codex). Every scraper sets
`conversation_id`, so the join is wired for all four; only two of them
will ever find a row.

## 3. The alignment matrix

| Agent | IronWire routes it | IronWire auto-wires | Sends a session id | We scrape it | Enrichment joins |
|---|---|---|---|---|---|
| Claude Code | yes | yes | yes | yes | yes |
| Codex CLI | yes | yes | yes | yes | yes |
| Codex desktop | yes (shared config) | partial | unverified | same rollout dir | probably |
| Gemini CLI | no facade | no | n/a | yes | never |
| Antigravity | no facade | no | n/a | import only | never |
| Letta trajectory | n/a (file format) | no | none | yes | never |
| Aider | Chat Completions | manual | no (unverified) | no | no |
| Cline / Roo | either facade | manual | no (unverified) | no | no |
| Zed / Amp / Copilot | mentioned in docs | no | unverified | no | no |
| Ironclaw (TEE) | not a client | n/a | n/a | server-side recording path | n/a |

The intersection where everything lines up is two agents: Claude Code and
Codex. Everything else is lopsided in one direction or the other.

## 4. How to maximally align them

Ordered by value over cost. Items 1 and 2 are ours alone; 3 and 4 are
shared; 5 is upstream only.

### 4.1 Prove the intersection live (ours, small)

The join code is built but has only been tested against an in-memory
fake ledger. Run a real Claude Code and a real Codex session through a
real IronWire on one machine, load them through the daemon, and record
the join rate: fraction of loaded sessions with at least one
`RoutingDecision` event. Surface that number in daemon status so a
contributor can see whether enrichment is working. No design change.

### 4.2 Scrape the agents IronWire already routes (ours, medium)

IronWire covers Aider, Cline and Roo on the wire; we have no scraper for
any of them. Adding scrapers closes the gap from our side with zero
upstream dependency.

- **Cline first.** Its session store is structured JSON with tool calls
  intact. Closest to what the gate scores on today. Roo is a fork of the
  pre-SDK Cline and still uses the legacy `tasks/<id>/` layout; current
  Cline does not (see 4.7).
- **Aider second, conditionally.** `.aider.chat.history.md` is lossy
  markdown; `.aider.llm.history` (opt-in) is the full LLM transcript.
  Only worth a scraper for the latter.
- Zed, Amp, Copilot: no evidence of demand; skip.

These traces will not join to IronWire rows, because those agents send no
session header. That is acceptable: an unenriched trace is what every
trace was before 2026-09-01.

### 4.3 A client-agnostic session header (upstream, small)

`client_session_header` hard-codes one header per protocol. Proposal:
IronWire also honours a neutral header, say `x-ironwire-session-id`, on
every facade, preferring it when present. Any agent or wrapper that can
add a request header then becomes joinable without IronWire knowing it
exists. Cline exposes custom headers for OpenAI-compatible providers;
Aider has an `extra_headers` model setting. Both unverified here. This is
a one-function upstream change plus a ledger test, and it is what turns
Tier B from "routable" into "routable and joinable".

### 4.4 One signed agent registry for both products (shared, larger)

The structural move. IronWire's `AgentEntry` already describes an agent as
id, display name, detect names, and config location. Our daemon needs the
same agent described as id, display name, and session-store location. Two
products maintaining two hand-written lists of the same tools is the drift
`ironwire_agents` was created to prevent inside one product.

Proposal: extend `AgentEntry` with optional `sessions: ConfigLocation`
(where the agent keeps transcripts) and `session_header: Option<String>`.
Our daemon fetches and verifies the same signed catalog (Ed25519, we
already ship ring) and uses it for source discovery; IronWire ignores the
fields it does not use. Ids converge: our settings keys are already
`claude` and `codex`, matching IronWire's; `gemini` becomes an entry with
`sessions` and no `settings`, which is exactly the "we scrape it, IronWire
does not route it" state, expressed in data.

Blocked on IronWire's real catalog signing key (ROADMAP M4). Until then
the schema change can land upstream and be exercised by tests only.

Naming drift to fix regardless: `SOURCE_CLAUDE_CODE` is `"claude-code"`
and `SOURCE_GEMINI_CLI` is `"gemini-cli"` on the envelope, while the
settings keys are `claude` and `gemini`. Pick the IronWire spelling for
anything that crosses to their side; leave the envelope strings alone
(they are pinned by digests).

### 4.5 Gemini (upstream, large, low return)

Gemini CLI is our third first-class scraper and is invisible to IronWire.
Closing that means a `generateContent` facade and a third translation
wire in a codebase with no Gemini code. Gemini CLI users are mostly on
Code Assist subscriptions that do not route through API keys anyway, so
the routing value is weak. Recommendation: file the demand signal upstream
and accept Gemini, Antigravity and Letta trajectories as never-enriched.

### 4.6 Ironclaw

Not an IronWire client and not a contributor-daemon source; its traces
arrive through the server-side recording path. No alignment action.

## 4.7 Execution notes (2026-09-03)

- **4.1, done as far as it can be without touching user configs.** An
  ad-hoc `ironwire serve` (0.1.0, commit 4024619) with an isolated
  `IRONWIRE_HOME` and a fake local backend recorded one Anthropic Messages
  exchange carrying `x-claude-code-session-id` and one Chat Completions
  exchange carrying `session-id`. The page it served is checked in as
  `crates/trace-commons-contributor/tests/fixtures/ironwire/log-page-2026-09-03.json`
  and two tests pin that our client parses it and that the join attaches
  each row to the right spelling of its session. Not yet done: a real
  Claude Code or Codex session through a connected IronWire, because
  `ironwire init` rewrites `~/.claude/settings.json` and
  `~/.codex/config.toml` and routes this machine's own traffic; that is
  the operator's call. Also observed: IronWire did not forward
  `x-claude-code-session-id` to the fake local backend on the translated
  Anthropic-to-Chat path, despite `headers.rs` saying native session
  headers are forwarded. Not investigated.
- **4.3, built upstream on a local branch.** `neutral-session-header` in
  `/Users/zakimanian/code/ironwire`, two commits, workspace tests and
  clippy green. Not pushed. `x-ironwire-session-id` is read first on every
  facade and is not forwarded to the provider.
- **4.2, in progress.** Research against `cline/cline` main changed the
  target: the current release (extension 4.1.17) no longer writes the VS
  Code global-storage `tasks/<id>/` files at all. It persists
  `~/.cline/data/sessions/<id>/<id>.messages.json` with real `tool_use`
  blocks, per-message `ts`, `modelInfo` and token `metrics`, plus a
  sibling manifest carrying `cwd` and `model`. The scraper targets that
  format only. Fixtures are transcribed from upstream source, not captured
  from an install, and the plan says so. Cline sends no session header to
  a direct Anthropic or OpenAI-compatible endpoint; its `X-Task-ID` goes
  only to Cline's own gateway. Custom headers are configurable only for
  the OpenAI-compatible provider (`openAiHeaders`), which is where a user
  would set `x-ironwire-session-id`.
- **4.4, deferred.** IronWire's `ConfigLocation` is deliberately
  constrained to a dotdir plus a `.json` or `.toml` file, and widening it
  to describe a session store is a `TRUST.md` change on their side. A
  shared registry needs its own design, not a field on `AgentEntry`.

## 5. Sequencing

1. 4.1 live join proof (this week; evidence gate for everything below).
2. 4.3 upstream PR for the neutral header, in parallel with 4.2 Cline
   scraper spec.
3. 4.4 schema extension upstream, tests only, so it is ready when the
   signing key exists.
4. 4.5 deferred with an upstream issue.

## Open questions to verify before building

- Does Codex desktop write to the same `~/.codex/sessions` rollout
  directory as the CLI?
- Do Cline or Aider send any per-session header today, and can each be
  configured to send one?
- Where does Cline keep its task store on Linux and Windows (macOS path
  known; others assumed to follow VS Code's global storage layout).
