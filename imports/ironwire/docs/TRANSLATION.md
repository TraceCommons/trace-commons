# Translation: the pivot IR

How a request that arrived on one wire is re-expressed on another, and what that
costs. `docs/PROTOCOL.md` §6 states the *policy* — when a cross-family route is
allowed and when it is refused. This document is the *mechanism*.

---

## 1. Why this exists

IronWire speaks three wires:

| Short | Protocol | Spoken by |
|---|---|---|
| **A** | `anthropic.messages` | Claude Code, the Anthropic façade |
| **R** | `openai.responses` | Codex, the ChatGPT subscription, OpenAI keys |
| **C** | `openai.chat` | Aider, Cline, NEAR AI, local servers |

Any of them can arrive; any of them can be the only capacity left. That is six
ordered pairs, and each pair needs three mappings — the request, the
non-streaming response, and the stream. **Eighteen mappings**, of which this
codebase had three (A→C request, C→A response, C→A stream).

Writing the other fifteen pairwise is the obvious approach and the wrong one.
The streaming half is where it becomes untenable: each pair needs its own
buffering state machine, its own tool-call accumulator, its own framing limits,
and its own answer to "what does a half-finished tool call look like". Six
hand-written SSE state machines will not agree with each other, and the ways
they disagree are exactly the ways this product fails silently — a dropped tool
call, a `stop_reason` that says `end_turn` when a call is pending, an id that
does not round trip.

So: **pivot on a canonical intermediate representation.** Three parsers in,
three emitters out, per layer. Nine mappings become six, and — far more
importantly — the hard parts (tool-call buffering, SSE framing, id identity,
usage accounting) exist once instead of six times.

```
        parse                         emit
  A ─────────┐                   ┌───────── A
  R ─────────┼──▶  IR  ────────▶─┼───────── R
  C ─────────┘                   └───────── C
```

This is the shape LiteLLM, OpenRouter and the Vercel AI SDK all converged on,
for the same reason.

### The IR is a superset, never one of the wires

The tempting shortcut is to pivot on Chat Completions, since a translator to it
already exists. That would be a mistake: C is the *weakest* of the three. It has
no typed content blocks, no reasoning items, no notion of a response id to
resume from. Pivoting there would silently degrade every A→R route — the one
that matters most, because it is what lets a Claude Code session fall back onto
a ChatGPT subscription — by routing it through a format that cannot express what
either endpoint supports.

The IR is therefore its own type, and it is a superset: it can hold anything any
of the three wires can express, plus the things this build does not recognise.

---

## 2. Parse is lossless; emit reports the loss

This inverts the current design, and it is the single most important decision
here.

Today `anthropic_to_chat_completions` parses and emits in one pass, and reports
what it dropped along the way. That conflates two different questions:

- *What did the client send?* — a fact.
- *What can this target carry?* — a fact about a **specific** target.

With one function per pair they are the same question. With a pivot they cannot
be: the same parsed conversation is emitted to three different targets, and the
losses differ per target.

So:

- **`parse` is total and lossless.** Anything not modelled becomes
  `Block::Unknown { kind, raw }`, holding the original JSON. Nothing is
  discarded and nothing is judged.
- **`emit` returns `Dropped`.** It names what the target could not express, and
  the caller decides whether that is tolerable (`thinking` on a foreign
  provider: yes) or disqualifying (an unrecognised block: no — see §6).

The practical gain is that a same-protocol emit is provably lossless, which
gives every parser a free round-trip property to test against: parse A, emit A,
and the result must carry everything the input did. That test does not exist
today and could not, because there was no A emitter.

---

## 3. The request IR

```rust
pub struct Conversation {
    pub system: Vec<SystemChunk>,
    pub turns: Vec<Turn>,
    pub tools: Vec<ToolDef>,
    pub unexpressible_tools: Vec<String>,
    pub tool_choice: Option<ToolChoice>,
    pub params: Params,
}

pub struct Turn { pub role: Role, pub blocks: Vec<Block> }

pub enum Block {
    Text(String),
    Image(Image),
    ToolUse { id: ToolCallId, name: String, input: Value },
    ToolResult { id: ToolCallId, content: String, is_error: bool },
    Reasoning(Reasoning),
    Unknown { kind: String, raw: Value },
}
```

Three notes on shape.

**Tool results are blocks, not a role.** Anthropic packs them into a user turn;
Chat Completions gives them their own `role: "tool"` message; Responses uses a
`function_call_output` item. Modelling them as a block and letting each emitter
place them is the only version that survives all three.

**`Reasoning` carries its origin.**

```rust
pub struct Reasoning {
    pub origin: Protocol,
    pub summary: Option<String>,
    pub opaque: Value,
}
```

Signed Anthropic `thinking` and encrypted OpenAI reasoning items are
provider-private: replayable *only* to the provider that minted them, and
inert-but-harmless everywhere else (`docs/PROTOCOL.md` §6). Recording the origin
lets an emitter make that call correctly instead of guessing — replay the opaque
part when emitting back to `origin`, and drop it, counted, otherwise. The
human-readable `summary` is not provider-private and survives everywhere, which
is a small fidelity win the current lane does not take.

**`Unknown` keeps the raw JSON.** A block this build has never heard of is
carried whole, so a same-protocol round trip preserves it and a cross-protocol
emit can name it precisely.

---

## 4. The response IR

```rust
pub struct Completion {
    pub id: String,
    pub blocks: Vec<Block>,
    pub stop: StopReason,
    pub usage: Usage,
}

pub enum StopReason { EndTurn, MaxTokens, ToolUse, Refusal, StopSequence(String), Unrecognised(String) }

pub struct Usage { pub input: u64, pub cached_input: u64, pub output: u64, pub reasoning: u64 }
```

`Block` is reused deliberately: an assistant turn in a *request* and an
assistant turn in a *response* are the same thing, and the client will replay
the second as the first. One type means the round trip cannot drift.

`StopReason::ToolUse` is the value worth being careful about. Getting it wrong —
reporting `end_turn` on a turn that issued a call — stops the agent instead of
letting it execute the call it was just handed. It is the highest-consequence
single field in this whole document.

---

## 5. The stream IR

The layer that justifies the pivot.

```rust
pub enum Delta {
    Start { id: String, model: String },
    Text(String),
    ReasoningText(String),
    ToolCall { index: usize, id: ToolCallId, name: String, arguments: String },
    Stop { reason: StopReason, usage: Usage },
}
```

**Tool calls are buffered until complete**, in the IR rather than in any one
translator. Chat Completions streams arguments as fragments of a JSON string;
Anthropic's `tool_use` carries a parsed object; Responses emits typed
`function_call_arguments.delta` events. There is nothing meaningful any target
can emit until the arguments are whole, and buffering costs no perceived
latency because no agent can act on half a call.

Everything hostile-input-shaped lives here once: the frame boundary search, the
1 MiB frame ceiling, the resync-after-oversized-frame rule, the 256-call and
4 MiB argument caps. Those exist today inside `ChatToAnthropicStream` and would
otherwise be copy-pasted into six state machines, where five copies would drift.

---

## 6. What each pair costs

Lossless unless stated. "Dropped" means named in `Dropped` and reported, never
silent.

| | → A | → R | → C |
|---|---|---|---|
| **A →** | native | thinking dropped¹, cache breakpoints dropped | thinking dropped¹, cache breakpoints dropped, system flattened to one string |
| **R →** | encrypted reasoning dropped¹, non-function tools dropped² | native | encrypted reasoning dropped¹, typed items flattened, non-function tools dropped² |
| **C →** | — | — | native |

¹ The summary text survives; only the provider-private blob is dropped. The
receiving provider never validates a foreign blob, and the API that minted it
drops rather than rejects it (`docs/PROTOCOL.md` §6).

² Responses lets a client declare tools that are not functions: a provider
built-in (`{"type": "web_search"}`, no `name` at all) and a `namespace`
holding a nested tool list, one per connected MCP server. Neither has an
equivalent on the other wires. They are left out of `tools` and named in
`unexpressible_tools`, which `emit` copies into `Dropped::tools`. The
alternative this replaced was worse than a drop: a built-in became a function
named `""`, which every OpenAI-compatible server rejects, so one built-in
anywhere in a Codex tool list made the whole translated request a 400.

**What is still a hard refusal**, unchanged by any of this: an unrecognised
content block. A `document` a user asked a question about is indistinguishable
from a decorative one, and answering about content the model never received is
the silent degradation the whole product refuses. The native lane carries it
perfectly, so the cost of refusing is waiting for same-family capacity.

The other hard refusals — tools against a backend with none, used parallel calls
against a serial backend, images against a text-only model, a prompt past the
context window — are capability-gate questions and stay in
`ironwire_core::capability`. Translation does not get a vote on them.

---

## 7. Tool-call identity, generalised

`toolu_xw_<original>` becomes one case of a general rule: an id must be **valid
in the target's namespace** and **reversible**, with no shared state, because a
map that can be lost mints ids that cannot be recovered
(`docs/PROTOCOL.md` §6).

| Target | Rule |
|---|---|
| A | prefix `toolu_xw_` — Anthropic requires the `toolu_` shape |
| R, C | pass through — both accept arbitrary strings |

The bijection property and its tests carry over unchanged. The generalisation is
that the encoding is now chosen by the *target protocol* rather than hardcoded
to Anthropic.

---

## 8. What does not change

- **The native lane is byte-identical.** Same wire in and out means no IR, no
  parse, no emit. A request that needs no translation must not be re-serialised,
  and `crates/ironwire_proxy/tests/passthrough.rs` and `codex_on_nearai.rs` both
  assert this at the byte level.
- **The turn-boundary rule.** Families change at a turn boundary, never mid tool
  loop. That is `ironwire_core::capability`, not this crate.
- **Refusal beats degradation.** Nothing here makes a lossy route silently
  acceptable; it makes the loss precise enough to report.
- **The model the client asked for is the model reported back.** Substituting
  the served slug makes the client's own bookkeeping incoherent.

---

## 9. Staging

1. The IR types, and `tool_ids` generalised.
2. Requests: parse and emit for A, R, C. The A→C emitter must reproduce the
   existing translator's output, so the existing tests are the regression suite.
3. Responses (non-streaming): parse and emit for all three.
4. Streams: one framing driver, three parsers, three emitters.
5. `Protocol::translates_to` opens to every pair; the pipeline picks the emitter
   from the chosen backend's primary wire rather than assuming Chat Completions.
6. End-to-end conformance for the newly reachable lanes — A→R first, since a
   Claude Code session falling back onto a ChatGPT subscription is the route
   users are most likely to want and this build could not offer.
