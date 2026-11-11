# Review of the draft IronWire specification

Status: accepted with changes. This document records what the draft got right,
what has to change before it becomes buildable, and why. `DESIGN.md` is the
finalized design that results; read this one only if you want the reasoning.

The draft's core thesis — *sit at the inference boundary, let the coding agents
keep their own loops, aggregate the user's subscriptions* — is correct and is
kept unchanged. Everything below is about the mechanism.

---

## 1. Per-request cross-family routing is the load-bearing idea, and it is unsound

The draft's headline illustration:

```
request 1 → Claude Max / Opus
request 3 → Claude Max / Sonnet
request 4 → NEAR AI
request 6 → Codex subscription
```

described as "invisible" to the user. Four things break.

**Prompt cache destruction.** Claude Code sends a large stable prefix (system
prompt, tool schemas, file context, conversation history) with `cache_control`
breakpoints. On a warm cache that prefix bills at ~10% of input rate and adds
almost nothing to TTFT. Every switch away and back is two cold caches. On a
100k-token prefix that is not a rounding error: it is roughly a 10× cost swing
on the input side and multiple seconds of added latency, repeatedly. A router
that "saves quota" by hopping models can easily *consume more quota* than one
that stays put.

**Reasoning continuity.** Anthropic returns `thinking` blocks carrying a
`signature`; the OpenAI Responses API carries `reasoning` items with
`encrypted_content`. Neither survives a detour through another provider, so a
family change costs reasoning continuity and therefore quality.

> **Correction.** An earlier revision of this document claimed signed thinking
> made cross-family routing *permanently impossible*. That was wrong, and it was
> the most expensive error in the review: it would have deleted an entire
> capacity pool for the flagship use case. A foreign provider never validates an
> Anthropic signature, and Anthropic **drops** foreign reasoning state rather
> than rejecting it. The genuine constraint is narrower — replaying an assistant
> turn whose `tool_use` lacks its expected thinking block can be rejected — so
> the rule is *switch at a turn boundary, never mid tool loop*, which defers a
> switch rather than forbidding one. See `docs/PROTOCOL.md` §6.

**Tool-call ID identity.** `toolu_*` / `call_*` / `fc_*` are not
interchangeable, and the client persists whatever we returned and replays it
forever. Any cross-family route commits us to maintaining a stable bidirectional
ID map for the life of the session — not per request.

### Change

Routing granularity is **per conversation, not per request**, with sticky
affinity and hysteresis. Fallback is a *session-level state transition*, taken
under pressure, along an explicit fidelity ladder:

| Rung | Route | Cache | Thinking/reasoning | Translation |
|---|---|---|---|---|
| 0 | Same account, same model | warm | intact | none |
| 1 | Same account, smaller model (Opus→Sonnet) | warm-ish | intact | none |
| 2 | Same wire format, different credential (Claude sub → Anthropic API key / Bedrock / Vertex, or — opt-in — another of the user's own Claude subscriptions) | cold | intact — except across two of the user's accounts: unverified for Claude (ROADMAP), and ChatGPT's encrypted reasoning is rejected by any account but its own, so a Codex conversation carrying it never changes ChatGPT account (PROTOCOL §6) | none |
| 3 | Different family (Anthropic façade → Codex/NEAR) | cold | dropped | full |

Rungs 0–2 are silent. **Rung 3 is announced** — the user's agent will behave
differently and pretending otherwise is how we lose their trust the first time
it matters. It is also the only rung that needs the translation layer, which is
why the translation layer is not on the V0 critical path.

The pitch survives the change intact, and gets more honest:

> Your coding agent never dies at the rate limit.

not

> You will never notice which model you are talking to.

---

## 2. "Subscription backends invoke the official executable" does not work

The draft proposes avoiding credential replay by having subscription backends
call "the official Codex executable/app-server/SDK".

A gateway needs an **inference endpoint**. `claude -p` and `codex exec` are
**agent loops**. Handing one an arbitrary Messages API request means discarding
the client's system prompt, its tool schemas, its cache breakpoints, its
thinking budget and its sampling parameters, then wrapping the whole thing in a
*second* agent's system prompt and a *second* tool loop. Streaming event
fidelity, `stop_reason`, per-block usage accounting and tool-call IDs are all
lost. Running Claude Code's requests through Claude Code is not a fidelity
compromise; it is a different product.

There is also no version of it that is meaningfully more permitted. It is the
same credential doing the same work; a subprocess boundary is not a legal
boundary.

### Change

Subscription backends replay the user's own credential against the provider's
own endpoint, exactly as the official client does. `ironclaw_llm` already ships
this and it is proven in production:

- `codex_auth.rs` — reads `~/.codex/auth.json`, ChatGPT OAuth mode against
  `chatgpt.com/backend-api/codex`, refresh-on-401 with token persistence.
- `anthropic_oauth.rs` — reads the macOS Keychain item `Claude Code-credentials`
  or `~/.claude/.credentials.json`, `Authorization: Bearer` +
  `anthropic-beta: oauth-2025-04-20`, refresh on 401.

The draft's risk instinct is right; it just picked the wrong control. The real
controls are structural, and they are now design invariants (`TRUST.md`):

- **Loopback only.** Bind 127.0.0.1. No remote mode, no tunnel, no `--host`.
- **Single user, single machine.** Credentials never leave the box. There is no
  hosted IronWire that holds anyone's subscription token, ever — that is the
  line between "using what I pay for" and "reselling capacity", and it is not a
  line we approach.
- **Consent is explicit, per backend, default-off**, in plain language naming
  whose account bears the risk. `ironclaw_llm` emits a `tracing::warn!` here;
  a warning in a log is not consent.
- **Identity is not forged.** See §5.

---

## 3. The canonical IR is drawn at the center; it belongs at the edge

The draft says "avoid the lowest common denominator" and then draws every
request flowing through canonical IR. Those are in tension, and the diagram
wins by default in implementation.

Measured against real usage, the overwhelming majority of traffic is
**Anthropic-in → Anthropic-out** (Claude Code → Claude subscription) or
**Responses-in → Responses-out** (Codex → ChatGPT backend). For those, parsing
into an IR and re-serializing can only lose information. Every unknown field,
every new beta header, every content-block type we have not modeled yet is a
silent fidelity bug that ships the day the provider changes something.

### Change

Two lanes, with the native lane as the default and the overwhelming majority:

- **Native lane.** Validate, account, rewrite auth, forward. We do not rebuild
  the body; unknown fields survive because we never look at them. SSE is
  forwarded frame-for-frame. Fidelity is 1.0 by construction, including for
  provider features that did not exist when we shipped.
- **Translated lane.** IR-mediated, used only for cross-family fallback and for
  backends that speak a different protocol. **Capability-gated: a route whose
  target cannot preserve the request's semantics is ineligible, not
  best-effort.**

This inverts the engineering risk. "Build a perfect universal translator" is
not achievable and would be on the critical path. "Build a perfect pipe, plus a
narrow honest translator" is achievable in weeks, and the pipe is the part users
actually depend on.

A consequence worth stating: since we mostly do not construct the body, we must
**observe** usage rather than compute it — parse `message_delta.usage` off the
wire, read `x-ratelimit-*` headers, honor `retry-after`. Never estimate what the
provider is willing to tell us.

---

## 4. "Estimated quota: 72%" is a credibility bomb

Inferring subscription headroom from token counts is hopeless: the windows are
rolling, opaque, model-weighted, and change without notice. One confidently
wrong percentage and the user stops believing the entire status surface —
including the parts that are correct.

### Change

Quota is **observed, never modeled**:

- Anthropic returns unified rate-limit headers on every response.
- The ChatGPT/Codex backend returns primary/secondary rate-limit window usage.
- 429s carry `retry-after`.

Display last-observed value with an age ("82% used, observed 40s ago, resets in
2h14m"). Where nothing is observable, display `unknown`. Never synthesize a
number, and never let a synthesized number drive a routing decision that the
user will attribute to us.

---

## 5. `model = ironwire/auto` does not match how the clients behave

Claude Code does not let the user type an arbitrary model slug in the normal
flow; it sends `claude-opus-4-6` and friends from a fixed picker. So `auto` as a
*model string the user selects* mostly will not happen.

### Change

The client's model string is a **quality hint**, not a selection. Policy applies
to whatever was requested:

- `requested_model` → normalized to a **tier** (frontier / balanced / fast).
- Policy picks a concrete backend+model that satisfies that tier, subject to the
  fidelity ladder and observed quota.
- Escape hatches: `ironwire pin <model>` (sticky override),
  `X-IronWire-Route:` request header, and the `ironwire/*` slug family for
  clients that *do* accept free-form model strings (Aider, Cline, Codex).

---

## 6. Hard requirements the draft omits

These are not polish; each one breaks a client on day one.

- **`POST /v1/messages/count_tokens`.** Claude Code calls it to drive its
  context-budget math and compaction trigger. If the Anthropic façade 404s it,
  Claude Code's context accounting goes wrong.
- **Retry is not free at the proxy.** Once a byte of SSE has reached the client
  we cannot transparently retry — replaying would corrupt the stream. Rule:
  **retry and failover only before first byte.** After first byte the only
  honest options are propagate the upstream error or end the stream with an
  `error` event. The draft's "retries/failover" box hides this entirely.
- **Cancellation must propagate.** Coding agents abandon requests constantly
  (user hits Esc). If we do not abort upstream on client disconnect we burn the
  scarce quota we exist to protect.
- **OAuth-mode Anthropic requires the Claude Code system preamble.** Requests
  authenticated with a subscription token are expected to carry Claude Code's
  identifying first system block. This creates a fork:
  - *Inject it for any client* → we are dressing Aider up as Claude Code. That
    is the impersonation the draft rightly wanted to avoid, just moved.
  - *Require it* → the subscription backend is eligible only for requests that
    already carry Claude Code's identity, i.e. actual Claude Code. Everything
    else routes to an API key or another backend.

  **Take the second.** It is defensible, it is simple, and it falls out of the
  architecture rather than being bolted on. Same rule for Codex: the ChatGPT
  backend is eligible for requests that arrive as Codex.
- **Single-daemon semantics.** Port collision, lockfile, `ironwire status`
  talking to the *running* daemon rather than reading config off disk.
- **Long-lived streams.** 10+ minute responses, idle-timeout tuning, keep-alive,
  and no buffering anywhere in the path.

---

## 7. Trace capture: right instinct, thin consent story, and one thing that must be said out loud

IronWire sits where 100% of the user's proprietary source code flows. That is
the highest-trust position in a developer's toolchain, higher than the coding
agent itself, because the agent at least has a UI showing what it is doing.

### Changes

- **Local capture on by default; upload off by default.** Local traces must be
  useful *to the user first* — `ironwire log`, `ironwire replay`, "why did this
  request cost $4", "what did the agent actually send". That way the feature
  earns its place even for the user who never shares anything, and sharing is a
  separate, later, explicit decision rather than a checkbox in onboarding.
- **Reuse `ironclaw_trace_commons`** rather than reinventing. It already has
  deterministic redaction, consent policy, an on-disk queue with holds,
  manual-review gating, classification, credit/claims, and device-key
  onboarding — i.e. the entire NEAR AI credit loop the draft sketches. Its
  `capture_conversation_trace(scope, messages, task_failed)` entry point takes a
  scope string and a plain `ConversationMessage` list, which is close to exactly
  what a proxy can produce.
- **Say the training question out loud.** Both major providers' terms restrict
  using their outputs to train competing models. Traces of a user's own
  subscription sessions, uploaded by that user's explicit choice, are the user's
  call and their terms to honor — but IronWire should not be built such that its
  economics *require* that reading to hold. Concretely: the trace corpus is
  justified first by evaluation, routing quality and user-facing analytics; the
  training-corpus upside is upside. If it goes away, the product still works.

---

## 8. Scope: V0 is still too wide

The draft's V0 — "Claude Code + Codex → Claude sub + Codex sub + NEAR AI" — is
four connections, and every pairing except the two diagonal ones needs the
translation layer. Translation is where the schedule dies.

### Change

Split it. M1 is one native lane end to end:

> **Claude Code → IronWire → Claude subscription, falling back to Anthropic API key.**

Zero translation. It proves the pipe, streaming fidelity, observed quota, the
failover UX, cancellation, and local trace capture. It is testable on a laptop
that already has `~/.claude/.credentials.json`.

M2 adds the second native lane (Codex → ChatGPT backend). Only M3 introduces the
first translation pair. Full sequencing in `ROADMAP.md`.

---

## 9. Smaller notes

- **Fidelity budget matters for distribution.** `brew`/`npx`/`apt`/`pip` all
  want a small self-contained binary. `ironclaw_llm` pulls `rig-core` and
  `ironclaw_trace_commons` pulls `ironclaw_llm`. Neither is needed for the
  native lane. Reuse them behind off-by-default features rather than in the
  default build — see `DESIGN.md` §7.
- **The Mac menu bar app must be a viewer.** The daemon owns all state and
  exposes a local control API; the menu bar is a thin SwiftUI client over it. No
  routing logic in Swift, or CLI and GUI diverge within a month.
- **`ironclaw_llm::auth` exposes Codex credentials but not Claude Code's** (the
  Claude reader is a private fn inside `anthropic_oauth.rs`). Upstream gap to
  close: add `CredentialSource::ClaudeCode`. Until then IronWire carries its own
  small reader.
- **`smart_routing.rs`'s 13-dimension complexity scorer** is a useful *signal*
  for tier selection but must not be the router. Complexity of the latest user
  message is weakly correlated with what a coding agent actually needs on turn
  40 of a session.
