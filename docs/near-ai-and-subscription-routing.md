# Routing between NEAR AI and subscriptions

What this repository knows, as of 2026-11-12 (`main` at `8fcb9e6fb`), about
how a coding tool's model calls reach either NEAR AI or the user's own Claude
or ChatGPT subscription: the IronWire design, the code that implements it, the
two ways the contributor uses it, how Claude Code labels subagent requests on
the wire, and what a "NEAR AI main agent, subscription subagents" router would
need. Citations are to files on `main` at that commit; line numbers drift, so
the section names are the stable part.

This is a survey, not a spec. Where it says something is decided, the
citation is the decision; where it analyses, it says so.

## 1. The two paths in the contributor

The contributor reaches IronWire in two different ways, with different
routing rules. Most confusion about "where did this call go" comes from mixing
them up.

| | Shared proxy (the Private AI switch) | Managed NEAR AI session |
|---|---|---|
| Started by | `private_inference` switch, off by default, never started by discovery (`crates/trace-commons-contributor/src/daemon/private_inference.rs:1-10`) | Launching a tool session with the NEAR AI connection (`crates/trace-commons-contributor/src/managed/route.rs`) |
| Proxy instances | One, shared with the `ironwire` CLI, home `$IRONWIRE_HOME` or `~/.ironwire` | One per session |
| Privacy mode | Not `full`; the daemon never writes `config.toml` except a staging seed (`private_inference.rs:15-26`) | `mode = "full"`, `trusted_backends = ["nearai"]`, `receipts = true` (`route.rs:126-138`) |
| Credentials | With a NEAR AI key it answers `NEARAI_API_KEY` **and** keeps `CredentialFiles::Discover`, so existing Claude Code / Codex logins remain destinations beside NEAR AI (`private_inference.rs:449-488`) | Answers only `NEARAI_API_KEY`; no discovery (`route.rs:157-162`) |
| Pin | None | `POST /_ironwire/pin {"backend":"nearai"}`, backend only, no model (`route.rs:201-209`) |
| Can a call reach a subscription? | Yes, if the user granted subscription consent (embedded starts never grant it, `docs/ironwire/EMBEDDING.md:3-7`) | No. `full` is a refusal, never a fallback (`crates/ironwire_core/src/policy.rs:205-218`) |

Subscription and API-key connections in managed sessions do not go through
IronWire at all: the supervisor launches the native tool against its provider
directly (`crates/trace-commons-contributor/src/managed/supervisor.rs:252-277`).
Routing a subscription through IronWire "requires account-aware credential and
routing capabilities the pinned integration does not currently expose", and
there is no cross-provider fallback
(`docs/superpowers/specs/2026-10-05-saved-model-accounts-design.md:59-66`).

## 2. How IronWire chooses a backend

### Conversation, not request

IronWire routes per conversation (`docs/ironwire/DESIGN.md` §3).

- Conversation key with a client session:
  `H(protocol, client session, client login)`; without one:
  `H(protocol family, system_preamble[..512 bytes], sorted tool names, client login)`.
  Keys are memory-only, TTL-evicted, never stored with content.
- In code: `ConversationKey::for_session(protocol, session, login)` when a
  session header is present, otherwise derived from content
  (`crates/ironwire_proxy/src/facade/anthropic.rs:199-210`).
- Each conversation holds a sticky `Affinity { backend, model, rung, since,
  client_login, ... }`, reused only for the same login and forgotten when the
  backend fails. The Responses replay origin is kept separately and outlives
  it.

### Session headers

`docs/ironwire/PROTOCOL.md` "Session headers";
`crates/ironwire_proxy/src/facade/mod.rs:229-275`;
`crates/ironwire_upstream/src/headers.rs`.

| Header | Read on | Forwarded upstream? |
|---|---|---|
| `x-ironwire-session-id` | any facade; wins when present | no (stripped) |
| `x-claude-code-session-id` | Anthropic | yes |
| `session-id` | OpenAI | yes |
| `session_id` | OpenAI Responses, legacy Codex | yes |

Values must be 1-200 characters of `[A-Za-z0-9-_:.]`. IronWire reads no other
Claude Code header for routing (see section 4 for `x-claude-code-agent-id`).

### Ranking and the fidelity ladder

- Cost rank: Local 0, Subscription 1, Credits (NEAR AI) 2, ApiKey 3
  (`crates/ironwire_core/src/protocol.rs:183-220`). Only Subscription needs
  consent.
- Candidate sort key: pressured, local beyond tier, cost, another login, no
  model; same-wire candidates before cross-family ones
  (`policy.rs:1294-1332`).
- The model string is a tier hint (frontier / balanced / fast) for choosing
  between backends; once a backend is chosen, a model it lists is served as
  named (`policy.rs:1801-1830`; `DESIGN.md` §3). A model name never picks the
  backend.
- Ladder (`DESIGN.md` §3; `Rung` in `policy.rs:29-56`):

| Rung | Transition | Cache | Reasoning state | User told? |
|---|---|---|---|---|
| 0 | stay | warm | intact | no |
| 1 | same account, smaller model | mostly warm | intact | no |
| 2 | same wire, other credential: metered key, or another of your subscriptions (opt-in) | cold | intact, except ChatGPT's | no |
| 3 | different family, translated | cold | dropped | yes |

- Descent debounce 20 s, climb-back debounce 5 min, affinity TTL 24 h, at most
  512 affinities and 4,096 reasoning origins (`policy.rs:468-501`).
- Cross-family switches happen only at a turn boundary, never inside a tool
  loop (`DESIGN.md` §2).
- NEAR AI is reached over Chat Completions; a Claude Code conversation moved
  to NEAR AI is a rung-3 translation that drops signed thinking and cache
  breakpoints (`docs/ironwire/TRANSLATION.md` cost matrix).
- Overrides: `ironwire pin`, the `X-IronWire-Route: backend[:model]` header
  (read and stripped, `pipeline.rs:192-225`), and `ironwire/*` model slugs.
  Pins and overrides obey the same eligibility gates.
- `NoRoute` outcomes (`crates/ironwire_core/src/policy.rs:184-244`):
  `NoBackendsConfigured`, `AllExhausted`, `NoneUsable`,
  `RequiresClientIdentity`, `NoTrustedBackendAvailable` (privacy `full`),
  `SpendCapReached`, `UnknownRoute`.

### Subscription eligibility

`DESIGN.md` §5; `docs/ironwire/TRUST.md`.

- A subscription serves only requests that already carry its product's
  identity: only actual Claude Code or Codex. Anything else on the Anthropic
  facade goes to an API key, NEAR AI or local.
- The login is learned only from the URL (`/anthropic/account/{id}`,
  `/openai/account/{id}/v1`), never the body. An unknown id is a 404
  `unknown_login`.
- Consent is off until explicitly granted, per login and per directory.
- `cross_account_fallback` (off by default) lets a client under pressure
  (at least 90% of plan used as the provider reports it, or spent) be served
  by another of the user's subscriptions to the same product. It relaxes the
  login half of eligibility, never the product half. Why it is off
  (`TRUST.md` §2): provider terms on rate limits and account sharing, and the
  provider can see it, because Claude Code's `metadata.user_id` carries the
  login's account UUID and device id and IronWire does not rewrite it.
- ChatGPT encrypted reasoning never changes account
  (`Ineligible::ReasoningBoundToAccount`). Whether Anthropic signed thinking
  survives an account change is unverified.

### Privacy, receipts and tiers

- Only NEAR AI answers can be checked: they are `pending` and later
  `verified` / `gateway_only` / `unattested` / `unavailable` / `failed`;
  everything else is `outside` (`DESIGN.md` §8). `verified` needs a
  `provider_tee` receipt bound by a verified TDX quote; gateway receipts alone
  are not model evidence.
- The model picker's privacy tiers come from `owned_by`
  (`crates/trace-commons-contributor/src/managed/models.rs:49-79`): NearAi
  (`nearai`, confidential TEE on NEAR AI's GPUs), External (`Attested 3P`),
  Incognito (a vendor model through NEAR AI's gateway).
- A background model must be of the main model's tier or a more private one
  (`models.rs:333-391`).
- Spend caps count only metered money; subscription cost is excluded
  (`crates/ironwire_proxy/src/spend.rs:1-12`).

## 3. Subagents today

IronWire has no subagent-aware routing. What exists lives in the
contributor's managed sessions, and all of it pushes subagents toward
**more** privacy, never less:

- Claude Code: `CLAUDE_CODE_SUBAGENT_MODEL`,
  `CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL`, `CLAUDE_CODE_AUTO_MODE_MODEL` and
  `CLAUDE_CODE_BG_CLASSIFIER_MODEL` are removed from the environment, and the
  family aliases are pinned to the session's tier or a more private one, so
  "no alias sends ... background or subagent requests to a less private
  model" (`models.rs:513-519, 558-561, 583-602`).
- A project subagent (`.claude/agents`) that names a model by id makes a NEAR
  AI session refuse to start (`ManagedError::ProjectChoosesModel`); aliases
  (`inherit`, `sonnet`, `haiku`) are allowed (`profiles.rs:465-540`).
- Codex: `agents.default_subagent_model`, `review_model` and the memory
  models are forced to the model in use (`codex.rs:28-39`).

## 4. How Claude Code labels subagent requests (measured)

Measured 2026-11-12 with Claude Code 2.1.322: two headless runs through a
logging pass-through proxy (`ANTHROPIC_BASE_URL`), one with a single subagent
and one with two parallel subagents that each ran a shell command. Headers
were recorded with credentials redacted; account values were hashed.

| Signal | Main thread | Subagent |
|---|---|---|
| `x-claude-code-session-id` | session UUID | **the same UUID** |
| `x-claude-code-agent-id` | absent | `a` + 16 hex digits |
| System block 1 (billing line) | `cc_version=...; cc_entrypoint=sdk-cli;` | adds `cc_is_subagent=true;` |
| System block 3 | main agent prompt | "You are an agent for Claude Code, ..." |
| Tools | includes `ScheduleWakeup`, `Workflow` | drops those, adds `SubagentHandback` |
| `metadata.user_id`, model, `anthropic-beta` | identical | identical |

- The agent id was stable across one subagent's turns and distinct between
  the two parallel subagents. The main thread never carried it.
- A subagent's first user message includes the same injected instructions
  block as the main thread (CLAUDE.md and user instructions), not only the
  task the main agent wrote.
- Consequence for IronWire as it is: a subagent and its parent share one
  conversation key, because the key uses the session header and IronWire does
  not read `x-claude-code-agent-id`. Every subagent therefore inherits the
  main thread's route and affinity.

Codex has not been measured.

## 5. A NEAR AI main agent with subscription subagents

The question: a NEAR AI model is the main agent in a Claude Code or Codex
session, and subagents the main agent judges not privacy-sensitive run on the
user's subscription. This section is analysis, not a decision.

**What already exists:** per-conversation routing, Claude Code and Codex on
NEAR AI via translation, native subscription passthrough per login, and a
ladder that can move a conversation across families at a turn boundary.

**What is missing:**

1. **Separating subagents.** Mix `x-claude-code-agent-id` into the
   conversation key (section 4), so each subagent gets its own affinity from
   its first request. `cc_is_subagent=true` is a second signal.
2. **A policy rule.** Main thread to NEAR AI; a subagent to the subscription
   only when cleared; anything unclassified to NEAR AI. Today the model name
   cannot select a backend, so this is new policy, not configuration.
3. **The shared proxy, not the managed one.** A managed NEAR AI session runs
   privacy `full` with a NEAR AI pin and no subscription credentials; it
   cannot do this by design. The router would sit on the shared proxy, or a
   new managed mode.

**Who decides "not a privacy issue":**

- Not the main agent's word alone. It has read the user's files and tool
  output, so injected content can talk it into calling something safe, and the
  task prompt it writes is a summary of private context.
- A subagent reads files and runs tools as it goes, so every later turn sends
  those results upstream. A decision at spawn covers none of them. Each
  outbound subagent request needs the privacy check, and section 4 shows even
  the first one carries the instructions files.
- Sketch: the NEAR AI agent proposes; IronWire classifies every outbound
  subagent request; a failure moves that subagent to NEAR AI for the rest of
  its life (a rung-3 move, reasoning dropped at the turn boundary) and never
  back. Results flowing from the subagent back to the main agent are not an
  exposure.

**Conflicts with current decisions:**

- Managed policy deliberately keeps subagent and background traffic at the
  main model's tier or more private (section 3). A subscription is direct to
  the provider: less private than every NEAR AI tier and unattestable. This
  reverses that rule and needs an owner decision.
- Provider terms: `TRUST.md` §2 covers rate-limit and account-sharing terms
  for driving subscriptions through IronWire; a subscription driven from a
  session whose main agent is another provider's model is a further step to
  check.
- Product copy: the destination is "Private AI"; a session that sends part of
  its work to a subscription has to say so.
- Subscription consent is per login and per directory, and embedded starts
  never grant it.

## 6. Stale or contradictory documents

Found while assembling this; not fixed here.

- `docs/multi-subscription-routing.md` says IronWire is a git dependency
  pinned at `a2ae5d97` with nothing vendored. The `ironwire_*` crates are in
  this repository since 2026-11-10 (`docs/ironwire/README.md`).
- `docs/managed-sessions.md` says Claude Code's default is the newest
  `anthropic/claude-sonnet-*`; `models.rs:24-30` defaults to the most private
  tier short of Incognito.
- `docs/managed-sessions.md` says managed Codex uses the Responses-to-Chat
  compatibility route; `route.rs` asserts its config does not contain
  `responses_via_chat`.
- `docs/managed-sessions.md` lists only `CLAUDE_CODE_SUBAGENT_MODEL` as
  removed; the code removes four variables.
- `docs/ironwire/DESIGN.md` cites `docs/TRANSLATION.md` and
  `docs/PROTOCOL.md`; they now live under `docs/ironwire/`.
- `docs/ironwire/CRITIQUE.md` lists Bedrock/Vertex as rung-2 targets;
  `ROADMAP.md` has them under "Later".
- Untested in the live world (`ROADMAP.md`): cross-account behaviour with two
  real accounts, and descent under a real rate limit (mock only).
