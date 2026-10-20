# Protocol fidelity

The native lane's promise is that a request is indistinguishable, upstream, from
one the client sent directly — and that the response is indistinguishable,
downstream, from one the provider sent directly. This document is what that
costs.

---

## 1. Façade surface

### Anthropic façade — `http://127.0.0.1:8463/anthropic`

Set `ANTHROPIC_BASE_URL` to that URL. Claude Code appends `/v1/...` itself.

| Route | Required by | Notes |
|---|---|---|
| `POST /v1/messages` | everything | streaming and non-streaming |
| `POST /v1/messages/count_tokens` | **Claude Code** | drives its context budget and compaction trigger. A 404 here silently breaks context accounting. |
| `GET /v1/models` | model pickers | synthesized from the backends that are actually eligible |

Headers that must survive untouched: `anthropic-version`, `anthropic-beta` (all
values, comma-joined), `x-api-key`/`authorization` (replaced, not forwarded),
and any `anthropic-*` we do not recognize.

### OpenAI façade — `http://127.0.0.1:8463/openai`

| Route | Wire API | Used by |
|---|---|---|
| `POST /v1/responses` | Responses | Codex (`wire_api = "responses"`) |
| `POST /v1/chat/completions` | Chat Completions | Aider, Cline, most third parties |
| `GET /v1/models` | — | pickers |

### Login-bound URLs — `…/anthropic/account/{id}`, `…/openai/account/{id}`

Every route above also exists under an account prefix, for a client bound to
one of the user's logins (`DESIGN.md` §5):

| Client | Pointed at | So it requests |
|---|---|---|
| Claude Code with `CLAUDE_CONFIG_DIR=~/.claude-work` | `ANTHROPIC_BASE_URL=http://127.0.0.1:8463/anthropic/account/claude-work` | `…/account/claude-work/v1/messages`, `…/v1/messages/count_tokens`, `…/v1/models` |
| Codex with `CODEX_HOME=~/.codex-work` | `base_url = "http://127.0.0.1:8463/openai/account/codex-work/v1"` | `…/v1/responses`, `…/v1/chat/completions`, `…/v1/models` |

- **The segment is IronWire's.** It says whose request this is and is consumed:
  the upstream URL is the backend's base URL plus `/v1/...`, exactly as from
  the plain façade, and the request that leaves differs from the unbound one
  only in which login's credential is attached
  (`a_bound_url_changes_which_login_pays_and_nothing_else_on_the_wire`,
  `a_bound_codex_request_carries_its_own_homes_bearer_and_account`).
- **`{id}` must name a registered login of the façade's product** — a Claude
  login under `/anthropic`, a ChatGPT login under `/openai`. The default logins
  qualify too (`/anthropic/account/claude-sub`), and a moved default is reached
  no other way (below). Anything else is a 404 in the façade's own envelope —
  Anthropic `not_found_error`, OpenAI `invalid_request_error` — naming the
  logins that do exist, their URLs, and the command that adds one. Nothing is
  sent and no ledger row is written. It is never served as the plain façade
  instead: that would put one account's request on another account's token. An
  `{id}` that is not a plain identifier is described in the message rather than
  echoed back to a terminal. That includes an empty one, and one that is not
  UTF-8 once percent-decoded (`/anthropic/account/%FF/…`) — which the router
  would otherwise have answered with its own plain-text 400 before the façade
  saw it (`a_bound_url_naming_no_such_login_is_refused_naming_the_fix`).
- **The model lists are the login's.** A bound `GET /v1/models` offers what that
  login's client could be served: its own subscription, another login's only
  when cross-account fallback is on, and everything that is not a login.
  Codex's catalogue document is its own login's, because two plans can carry
  different models.
- **The plain façade is the default login's, in its default place.** Nothing
  about it changed for anyone who has not moved the default. A default moved
  with `config_dir` reads a directory that a different client uses — the
  Claude Code or Codex still on the plain URL kept the old one — so it is
  served only at its bound URL, `ironwire connect` wires it with that, and the
  plain façade's model lists stop offering it. A request still at the plain URL
  is served by no subscription of that product: the ladder without one, or the
  refusal below (`a_moved_default_login_serves_its_bound_url_and_never_the_plain_one`,
  `a_moved_default_codex_login_serves_only_its_bound_url`).
- **A client with no login of its own is told how to get one.** A Claude Code
  left on the plain façade while the only Claude login is another one gets a
  503 naming that login and its real `ironwire connect … --config-dir`
  command — not a 429, which its back-off would retry forever. A transient
  refusal beside it does not change that, and there always is one: every
  daemon registers NEAR AI, which reads as unhealthy without a key
  (`a_client_on_the_wrong_url_is_told_the_command_that_binds_it`). A client
  whose own login is registered but spent, or has its circuit open, is still
  told to wait, as a 429, whatever the other logins' state
  (`a_login_whose_circuit_is_open_is_still_its_clients_and_the_client_is_told_to_wait`).

---

## 2. What "passthrough" means precisely

For a native-lane request, IronWire performs exactly these mutations:

1. **URL**: façade path → backend base URL + same suffix. On a login-bound URL
   the `/account/{id}` segment is part of the façade prefix, not the suffix, so
   it never reaches the provider (§1).
2. **Auth headers**: strip inbound `authorization` / `x-api-key` / `api-key`
   and the account-bound `chatgpt-account-id` / `x-openai-fedramp`; attach the
   chosen backend's credential. For a ChatGPT subscription that credential
   includes `chatgpt-account-id`, read from the `chatgpt_account_id` claim of
   the very token being presented and attached to every request made with it
   — inference, the auth probe and the `/models` fetch alike. The header names
   the workspace a bearer belongs to, so a client's copy sent beside another
   login's bearer would be credential drift (`AGENTS.md` rule 4); it is
   dropped even when it happens to match. `x-openai-fedramp` is not
   re-attached, because no credential IronWire reads states it. Codex sends
   neither header to a custom provider, so stripping them costs a Codex
   client nothing.
3. **Host-ish headers**: strip hop-by-hop headers (`connection`,
   `transfer-encoding`, `keep-alive`, `upgrade`, `proxy-*`), set `host`.
4. **`model`**: rewritten *only* when policy selected a different model, via a
   targeted JSON edit of that one key — never a full re-serialize.
5. **Headers addressed to IronWire**: strip `x-ironwire-session-id`. It names
   the client's session to IronWire itself (§3), not to the provider; the
   provider never asked for it, and forwarding one would announce that there
   is a proxy in the path.
6. **Explicit admission binding**: only while an authenticated local client has
   confirmed an ephemeral binding for the exact client session and selected
   OpenAI Chat backend, insert `metadata.trace_commons_admission`. This happens
   after translation/model/privacy edits and before body capture, so captured
   and upstream bytes agree. The insertion preserves all other bytes, including
   unknown fields and whitespace. An identical existing value is unchanged;
   conflicting or ambiguous metadata refuses before sending. Other sessions are
   byte-identical; a bound session routed to another backend/protocol or past
   expiry is refused, never silently sent without its binding.
7. **Responses `tools` entries the backend rejects**: removed, and only the
   `type`s that backend is *known* to reject
   (`Capabilities::unsupported_responses_tools`, empty for almost everything).
   This is the one place a body edit is made on the client's behalf rather than
   the user's, and it is narrow on purpose. NEAR AI deserialises `tools`
   strictly and answers a `namespace` entry -- which Codex emits once per
   connected MCP server -- with `unknown variant `namespace`` and refuses the
   whole turn; forwarding it faithfully means the request cannot be served at
   all. Nothing is filtered to a lowest common set: `web_search` is on NEAR AI's
   own accepted list and goes through untouched, because dropping a tool the
   provider would have taken is a capability the user silently loses. The
   removal is logged with what went and how much.
8. **Provider opt-ins**: a backend may carry request headers of its own,
   declared once where that backend is built rather than in the shared request
   path. There is exactly one today, and it is **off unless an operator turns
   it on**. With `refuse_model_aliases = true` the NEAR AI backend sends
   `x-no-aliasing`, on both wires it speaks, and the gateway then answers 400
   for a model name that is an alias rather than serving the canonical model
   under it. A client's own copy of such a header is dropped rather than
   forwarded, so it is sent once and with our value -- cloud-api reads an
   explicit `false` or `0` as "alias away", and that is the operator's decision
   to make, not the caller's. No other backend sends it, and by default no
   backend sends it at all: see §3 for what happens instead.
9. **Explicit token-distribution capture**: only for an opted-in, exact
   backend/model pair, add missing Chat Completions `logprobs`/`top_logprobs`
   fields before body capture. Existing caller fields win, including false.
   No unrelated bytes are reserialized. Ambiguous JSON is not augmented.
   Responses and other protocols are not augmented by this initial adapter.
10. **Nothing else.** The body is otherwise the bytes the client sent.

`GET /_ironwire/admission-binding` requires the control bearer and returns
capability/limits only, never registered session or challenge values.
`POST /_ironwire/admission-binding` requires the control bearer and
`{session_id, backend, binding, confirmed:true}`. Binding spelling is
`tcad1:<64 lowercase hex account anchor>:<64 lowercase hex nonce>:<Unix expiry>`.
The named backend must already exist and speak OpenAI Chat without a
subscription identity requirement. Expiry must be in the future and no more
than 900 seconds away; an upstream challenge with a longer lifetime is refused,
not shortened or extended. At most 32 session bindings are held in memory,
never persisted. `DELETE /_ironwire/admission-binding` with `{session_id}` revokes
one. Restart clears all bindings. Expired bindings remain bounded refusal
records until removed or replaced; they do not silently resume unbound sends.
Neither endpoint changes routing policy, provider credentials, capture.enabled,
or capture.bodies. Binding is metadata, not admission or funding proof. The
account binding/nonce never appears in status or log messages.

> **The one documented exception, and it is opt-in.** The privacy filter
> (`docs/PRIVACY.md`) substitutes values on the way out and restores them on the
> way back. That is a mutation of both the body and the response stream, so it
> is off by default, announced in `ironwire status` while it is on, and marked
> per exchange in the ledger. This enumeration — and `tests/passthrough.rs` —
> describe the filter-off path, which is the only path that makes the
> byte-identity claim.

Everything IronWire needs for policy comes from a **peek**: a bounded,
non-consuming scan of the parsed body for `model`, `stream`, `system[0]`,
message count, `tools[].name`, and the presence of `cache_control`, `thinking`,
image blocks and `reasoning` — including whether a replayed Responses
`reasoning` item carries `encrypted_content` (§6). The peek result never
re-encodes the body. Which login a request belongs to is not in the peek's
reach: the façade sets it from the URL, and the client's session id from its
headers (§6).

### Streaming

The response body is forwarded as a byte stream, not re-framed. A tee reads a
copy for observation. Consequences we accept:

- Backpressure is the client's; a slow reader slows the upstream read.
- The observation tee must never be able to stall or fail the forward path. It
  runs on a bounded channel and **drops observations under pressure** rather
  than blocking bytes.
- `text/event-stream` responses set `Cache-Control: no-cache`,
  `X-Accel-Buffering: no`, and no compression is negotiated on our hop.

---

## 3. Observation

Usage and quota come off the wire, never from a model of our own (DESIGN §4).

**Anthropic (SSE):**
- `message_start` → `message.usage` (input, cache creation, cache read tokens)
- `message_delta` → `usage.output_tokens` (cumulative)
- `message_stop` → finalize
- headers: `anthropic-ratelimit-unified-*`, `retry-after`

**OpenAI Responses (SSE):**
- `response.completed` → `response.usage`
- headers: rate-limit windows returned by the ChatGPT backend

**Anthropic/OpenAI non-streaming:** `usage` object in the body.

An exchange whose usage could not be observed is recorded with
`usage: unknown`, not with an estimate.

### Model substitution

A gateway may serve a model other than the one the request named. NEAR AI does
this for catalogue aliases, and announces it in a response header --
`x-model-alias-resolved: <requested> -> <canonical>` -- on every aliased
response, alongside a top-level `warning` field it inserts into the body.

That header is read on both facades and recorded verbatim on the exchange as
`model_alias_resolved`. `served_model` cannot answer the same question: it comes
out of the response body and names the canonical model whether that model was
asked for or reached through an alias, so the substituted case and the ordinary
one are indistinguishable there.

It is worth its own column because of what the body rewrite costs. By
cloud-api's own account (`inject_warning_field`) the rewritten body "no longer
byte-matches what the backend TD signed, so response-hash verification will not
pass for aliased responses". A row carrying a value here is therefore a row
whose `response_sha256` will not verify against the provider's receipt for
`upstream_id` -- better learned from the row than from a failed verification.

Observing costs the caller nothing and refuses nothing, so it is unconditional.
Refusing is the separate, opt-in decision in §2.8.

### The client's session id, and what it is safe to join on

Each ledger row carries `client_session_id`: **the client's own identifier,
stored verbatim**, read from a request header the client sent.

| Façade | Header read |
|---|---|
| Any | `x-ironwire-session-id`, when present (takes precedence) |
| Anthropic (`Protocol::AnthropicMessages`) | `x-claude-code-session-id` |
| OpenAI (`Protocol::OpenAiResponses`, `Protocol::OpenAiChat`) | `session-id` |
| OpenAI Responses, when `session-id` is absent | `session_id`, older Codex's spelling |

`x-ironwire-session-id` is addressed to IronWire and is **not forwarded** to
the provider. The native headers are forwarded untouched, as before. The older
spelling is read because the same value keys where a session's reasoning lives
(§6), not only the ledger row (`codexs_older_session_header_is_read_after_the_current_one`). A
client that sends no native session header -- Aider, Cline, Roo -- can set
the neutral one wherever its provider settings allow an extra request
header, and its rows become attributable without an IronWire release.

The value has to be an identifier: **at most 200 characters, made only of
`A-Za-z0-9`, `-`, `_`, `:` and `.`**. Anything else -- a space, a `#`, an
emoji -- is recorded as nothing at all, silently, and the row comes back
unattributed. `my project #3` is not a session id; `my-project-3` is.

`None` when the client sent no such header, which is the honest answer and not
an error: a client that names no session cannot have its rows attributed to
one.

The charset above is why: this is client-supplied text written to a local
database that `ironwire log` renders to a terminal, and a row addressed by
nothing beats a row that can smuggle control characters onto someone's screen.

**Three properties a consumer joining on this must know**, because each has
already caused a wrong assumption:

- **It is verbatim, not normalised.** IronWire does not lowercase it, strip a
  prefix, or parse a UUID out of it. Whatever the client sent is what a row
  carries. A consumer holding the same session under a different spelling --
  a filename stem, a path, a prefixed form -- must do that mapping itself.
  IronWire cannot, because it never sees the other spelling.
- **It is not `conversation`.** That column is a hashed routing-affinity key.
  With a valid session header it uses protocol, session and login; without
  one it falls back to protocol family, preamble head and tool list, which may
  collide across sessions with the same tools. It is right for stickiness and
  wrong for direct attribution.
- **It is not unique to a machine or an install.** It identifies a session as
  its client named it, nothing more.

A consumer that cannot verify the two spellings agree should say so rather than
assume: a join that silently matches nothing is indistinguishable from a client
that ran nothing, and both render as an empty result.

---

## 4. Cancellation

Client disconnect must abort upstream. In axum this means holding the upstream
request's `AbortHandle` in a guard tied to the response body's `Drop`. Abandoned
requests otherwise keep generating — burning exactly the scarce quota IronWire
exists to protect.

Proved by `crates/ironwire_proxy/tests/cancellation.rs`: an upstream that streams
forever, a client that reads two frames and walks away, and an assertion that the
upstream's next writes fail. The same test pins the other half — the observation
tee flushes on `Drop`, so an abandoned request still records the tokens the
provider had already reported. Cancellation that loses the accounting would hide
exactly the spend the user needs to see.

---

## 5. Retry, failover, and keeping a stream alive

### The point of no return is the first *content* byte

An earlier revision of this section said retry ends at the first byte of the
response body. That was too strict, and it gave away the most recoverable
failure there is.

`message_start` and `ping` carry no content — they are an envelope and a
heartbeat. A stream that has emitted only those has told the client nothing it
would later have to un-tell. Anthropic's shape is `message_start`, then often a
long silence while the model thinks, then content; failures land in that gap
constantly, and under the old rule every one of them was unrecoverable for no
reason.

So the boundary moved:

```
request received
   │
   ├─ [retryable] connect · send · response head · message_start · ping
   │      429 / 5xx / connect error / reset / silence
   │      → retry the same backend, or descend the ladder
   │      `message_start` is HELD, so a restart is invisible
   │
   ├─ FIRST CONTENT EVENT ──────────── point of no return
   │
   └─ [committed] any failure becomes a terminal `error` event on the open
          stream. Never retried: replaying would duplicate text the client has
          already written into its transcript.
```

`ironwire_proxy::resilience` implements this; `tests/stalled_stream.rs` pins it.

### Keeping a quiet stream alive

The other half of "Response stalled mid-stream" is not a failure at all — the
upstream is thinking, and the client's patience runs out first. IronWire emits
`ping` events after `resilience.keepalive_secs` of upstream silence. A ping is
part of the Anthropic event stream, carries nothing, and states something true:
still connected, still waiting.

Two rules keep that honest rather than a way of hiding a hang:

- **Upstream pings are forwarded immediately**, never held. A delayed heartbeat
  is not a heartbeat, and holding them would grow the pre-commit buffer without
  bound.
- **We give up too.** After `resilience.stall_timeout_secs` of continuous
  silence the stream ends with a stated `error` rather than pinging into the
  void. A user waiting forever is a worse outcome than a user told to retry.

### Ending a dead stream honestly

When an upstream dies mid-response the old behaviour was to drop the connection,
leaving the client to infer a stall — which is literally the message the user
sees. Now the stream ends with a terminal `error` event in the shape Claude Code
already handles, saying whether the response was truncated and whether retrying
is worth it.

### Same-backend retry before descending

A 529 or a reset is usually momentary. Descending the fidelity ladder over one
would move a warm conversation off its subscription and onto a metered key the
user pays for. So a transient failure retries the **same** backend up to
`MAX_SAME_BACKEND_RETRIES` times with exponential backoff first, and only then
descends.

A 429 that carries a real `retry-after` is the exception: the provider told us
how long, and sleeping through it while other capacity sits idle would stall the
agent for no reason. Those descend immediately.

---

## 6. Where translation is lossy, and what we do about it

> The *mechanism* — the pivot IR, the parsers and emitters, and the cost of each
> ordered pair — is `docs/TRANSLATION.md`. This section is the policy: when a
> translated route is allowed at all, and what it costs when it is taken.

### The rule: switch families at a turn boundary, never mid tool loop

This section originally claimed that a signed `thinking` block made a
cross-family route permanently ineligible. **That was wrong**, and the error was
expensive: it deleted an entire capacity pool for the product's flagship use
case, because Claude Code runs with thinking on and therefore carries signed
blocks from turn one.

What actually happens:

| Direction | Behaviour |
|---|---|
| Anthropic thinking blocks → a foreign provider | The provider never validates them. We drop them during translation. Nothing errors. |
| Foreign reasoning state → Anthropic | Dropped by us, before it is sent. Anthropic would drop it from the prompt anyway — silently and unbilled — but relying on that is relying on somebody else's leniency. |
| Anthropic thinking blocks → Anthropic | Must be replayed **unchanged**. Modifying one is rejected; *removing* one can trigger an ordering/signature 400. |

The third row is the whole constraint, and it bites on the **return** path. A
turn served by a foreign provider produces an assistant turn with a `tool_use`
and no thinking block. Replay that to Anthropic mid-loop — where the block is
expected — and you get a 400 the user cannot diagnose. At a turn boundary
(the last message is a fresh user turn, not a replayed `tool_result`) there is
no in-flight state to be missing.

So the gate is **one boolean**, `RequestRequirements::mid_tool_loop`:

```rust
if cross_family && req.mid_tool_loop {
    return Err(Ineligible::MidToolLoop);
}
```

It **defers** a switch to the next clean turn rather than disqualifying the
conversation. In a coding session that is at most one turn of waiting.

The rule is about *switching*, so it does not apply to the backend a
conversation is already on. Continuing on the foreign backend mid-loop is where
that loop's state lives; treating the incumbent as ineligible inverted the rule,
because the conversation then fell through to a fresh selection and was dragged
back to the native family — mid tool loop, which is the move the rule exists to
prevent.

### Climbing back

The ladder is descended *and* climbed. A conversation that fell to rung 3
returns to a better rung once one has been continuously available for
`PROMOTION_DEBOUNCE` — five minutes, against twenty seconds for a descent, and
the asymmetry is deliberate. Descending is urgent, because the alternative is a
failed turn. Promoting is not, because the conversation is working; and
`Headroom::is_pressured` is a step function at 90% with no hysteresis of its
own, so without a long debounce a provider hovering at that line would move the
conversation — discarding a warm prompt cache — every twenty seconds. That is
strictly worse than never promoting.

Leaving a foreign family is gated by exactly the same turn boundary as entering
it, for exactly the same reason: the recent assistant turns carry none of the
native family's signed reasoning state. A blocked promotion waits for a clean
turn without losing its accumulated recovery, so a busy tool loop postpones the
climb rather than cancelling it.

### The account twin: ChatGPT reasoning stays where it was minted

The family rule has a twin one level down. OpenAI binds Responses encrypted
reasoning to the organization that produced it and rejects it anywhere else —
`invalid_encrypted_content`, "Encrypted content organization_id did not match
the target organization" — and Codex asks for encrypted reasoning on every turn
and replays it on the next. So for a Codex conversation, two ChatGPT logins are
not interchangeable credentials on one wire: from the second turn on, any
account but the one that minted the reasoning answers with a 400. This matters
only once a conversation can change account at all, which takes cross-account
fallback (`TRUST.md` §2); without it, another login is never usable to begin
with.

The gate lives in `eligible()`, fed by the policy's recorded origin:

- `RequestRequirements::encrypted_reasoning` is set when the input replays a
  `reasoning` item with non-empty `encrypted_content`. Empty or null content
  carries no private state, so a first turn can start on any eligible login.
- Native Responses replay requires the recorded backend and its known scope
  to match the current candidate. Subscriptions use a domain-separated hash
  of the provider's `chatgpt_account_id`; absent identity stays unknown.
  Metered keys use a different domain-separated hash of the actual key. That
  identifies the credential, without claiming to know its API organization.
  A changed key requires a new origin even if its organization might be the
  same. Missing origin, unknown subscription identity or mismatch yields
  `Ineligible::ReasoningBoundToAccount`.
- The Responses sender recomputes the scope from the credential it actually
  read and refuses before HTTP if routing selected a different one. A relogin
  between status and send cannot put an old account's state on a new bearer.

**The record is not the affinity.** An affinity says where a conversation
should go next and is forgotten when a backend fails. Its replay origin
survives 429, 5xx, stale credentials, retries and translated detours. Native
Responses selections establish the origin, for metered keys as well as
subscriptions; translated routes do not invent one. Scope hashes stay in
memory and are never a substitute for provider quota observations.

A valid client session header keys affinity by protocol, session and client
login, so changing instructions or tools during one session does not move its
route and a new session does not inherit a sibling's descent. Origin records
also include the client session and login. With no header, the content-derived
conversation key is the fallback and cannot distinguish identical concurrent
conversations. Tests include
`a_session_that_moves_to_the_sibling_does_not_take_another_sessions_reasoning_with_it`
and `one_codex_session_moving_to_another_login_takes_no_other_sessions_reasoning_with_it`.

The recorded origin remains binding whether its backend is healthy, absent
from the candidates, or currently rate limited. If it cannot serve, the request
may cross families at a turn boundary, where translation drops encrypted
reasoning, or it is refused. Moving to another native Responses login or a
metered key is refused before the provider can reject that state. When the
origin recovers, it can serve again. Tests:
`a_failing_sibling_keeps_the_reasoning_it_minted_from_the_clients_own_login`,
`a_failing_sibling_never_hands_its_reasoning_to_the_clients_own_login`, and
`encrypted_reasoning_minted_by_a_metered_key_stays_on_that_key`.

Three edges:

- **Pins and `X-IronWire-Route` obey the same gate.** They override preference,
  never semantic eligibility, consent or login ownership. A pin may start a
  new origin on a plaintext turn but cannot move encrypted replay to another
  account or key (`a_pin_can_start_reasoning_on_an_account_but_cannot_move_a_replay`,
  `a_route_override_cannot_move_another_accounts_encrypted_reasoning`). A
  user-named same-wire pin still records rung 0 rather than automatic rung 2.
- **Origins are memory-only and separately bounded.** The route cache holds
  at most 512 affinities; replay origins hold at most 4,096 entries, with
  24-hour idle retention. Expiry, capacity eviction or daemon restart loses
  knowledge, so native encrypted continuation refuses instead of assuming
  the client's own login (`with_no_record_encrypted_reasoning_is_refused_rather_than_assigned_an_account`,
  `an_expired_origin_refuses_a_replay_even_when_the_same_account_is_pinned`,
  `evicting_an_origin_at_capacity_refuses_instead_of_guessing_an_account`).
  Starting a new session or restoring authenticated origin evidence is needed
  for native continuation; persistent origin recovery is not implemented.
- **Provider behavior remains a live acceptance gap.** These local refusal
  gates prevent an unchecked subscription-to-key descent. They do not prove
  which real ChatGPT accounts share an organization or reproduce the reported
  `invalid_encrypted_content` response. Two-account and API-key live checks
  remain in `ROADMAP.md`.

Anthropic has no rule here because none is documented. Signed thinking is
"compatible across platforms"; whether another account accepts it is not
stated, and a Claude conversation moved by cross-account fallback carries its
blocks unchanged. That needs a live check with two accounts (`ROADMAP.md`).

### What crossing a family actually costs

None of these are refusals. They are what the rung-3 announcement is for:

| Lost | Consequence |
|---|---|
| Signed thinking / encrypted reasoning | No reasoning continuity across the switch. Quality drop, no error. |
| Prompt cache | Cold start. Refused only when the warm prefix exceeds `CACHE_SACRIFICE_THRESHOLD_TOKENS` — an economic call, not a semantic one. A prefix is warm only when the conversation's current backend caches prompts and is still connected; a cache-control breakpoint on a first turn is not a warm cache. |
| Client-specific system-prompt tuning | The prompt was written for Claude; another model follows it less precisely. |

### What is still a hard refusal

| Request carries | Why it breaks, not degrades |
|---|---|
| tools, backend has none | The agent waits forever for a call that cannot come |
| **used** parallel tool calls, backend is serial | All but one call is silently dropped |
| images, backend is text-only | The model cannot see the input it is being asked about |
| strict JSON schema, backend cannot guarantee one | The client fails to parse the answer |
| a prompt larger than the context window | — |

Note the second row: the gate fires on *history that already issued several
calls in one turn*, not on the permissive Anthropic default. Reading "permitted"
as "required" would refuse every serial backend for every request — the same
class of over-strictness as the original reasoning rule.

### Tool-call identity

A translated route needs ids to survive both directions, because the client
replays whatever id we returned, forever.

The obvious design is a conversation-lifetime map. IronWire deliberately does
**not** use one: a map is state that can be lost (restart, eviction), and losing
it mints *invalid ids* — undiagnosable and unrecoverable. Instead the mapping is
a stateless reversible encoding (`ironwire_translate::tool_ids`), chosen by the
**target** wire: Anthropic wants the `toolu_` shape, so a foreign id becomes
`toolu_xw_<original>` and the prefix comes off on the way back; both OpenAI
wires take arbitrary strings and need no encoding at all. Two processes agree
without sharing anything, and a restart changes nothing.

An id already native to the target goes back **unchanged**, which is why
`ToolCallId` carries where it came from: the string alone cannot distinguish an
Anthropic id going home from a foreign one arriving, and prefixing the first
produces `toolu_xw_toolu_01ABC` — an id the client has never seen and cannot
match to anything it is holding.

### Not yet done: switching mid-loop

The turn-boundary rule is the conservative choice, and it leaves capacity on the
table: a session that spends most of its life inside tool loops can wait a while
for a boundary. Lifting it means synthesizing the missing state on the return
path — either an empty/placeholder thinking block on the foreign assistant turn,
or stripping the whole turn's `tool_use`/`tool_result` pair and re-issuing it
natively.

Both need to be validated against the live API before shipping, since the exact
tolerance for a missing block at each position is not documented. Until then,
mid-loop switching is a **known limitation, not a design position**. It is
tracked in `ROADMAP.md` under M3.

### Content this build does not model

The IR models `text`, reasoning, `image`, tool calls and tool results on every
wire. Anything else — a `document`, a `search_result`, whatever a provider ships
next — is **carried whole** through parsing, and then, if the target cannot
express it, **named in `Dropped::unknown_blocks`, which makes the route
ineligible.**

Carried rather than judged on the way in, because whether a block can travel
depends entirely on where it is going: a block goes back to its own wire
untouched, and that round trip is what makes the parsers testable.

Refusing rather than dropping, because the two are indistinguishable from here.
A `document` block a user asked a question about looks exactly like one that was
decorative, and translating it away produces an answer about content the model
never received — silent degradation, which §6 exists to refuse. The native lane
carries it perfectly, so the cost is waiting for same-family capacity.

An earlier version let these fall through a catch-all arm and vanish without
appearing in the dropped report at all, which made this document's own promise
false for exactly the case where it mattered most.

### Admission metadata compatibility

An explicitly registered session requires its registered backend and an OpenAI
Chat output wire. This is a request requirement checked by `eligible()`: normal
policy can skip a preferred mismatched candidate and choose the matching eligible
backend, while pins, retries and failover can never select an unbound route.
Normal tool-loop, privacy, consent and capability constraints still apply.
Privacy substitution and any translation run before metadata insertion and
capture; captured bytes are the exact buffer forwarded upstream.

The selected provider must accept OpenAI Chat `metadata`. A strict provider
that rejects that object produces its ordinary upstream error; IronWire never
retries without the binding to make such a provider accept the request. Configure
a compatible backend, renew the binding, or explicitly revoke it. The privacy
cost and repeated-use semantics are recorded in `TRUST.md` §9.

## 7. Conformance testing

Fidelity claims are worthless without a harness that proves them.

1. **Golden-pair corpus.** Capture real Claude Code and Codex sessions
   (`IRONWIRE_RECORD=1`) into fixtures: exact request bytes and exact upstream
   response bytes.
2. **Passthrough identity test.** Replay each fixture through the native lane
   against a mock upstream; assert the bytes the mock received differ from the
   original *only* in the header/URL/model mutations enumerated in §2, and that
   the bytes the client received are byte-identical to the recorded upstream
   response.
3. **Stream-shape test.** Assert SSE event order, event names and frame
   boundaries are preserved exactly.
4. **Live smoke.** `ironwire doctor` hits every connected backend for real and
   reports per-backend latency.

   The probe must not claim another product's identity (`TRUST.md` §3), which
   rules out the obvious design. A synthetic 1-token *message* against the
   Claude subscription would have to carry Claude Code's system preamble to be
   accepted — i.e. IronWire pretending to be Claude Code, which is the one thing
   the architecture refuses. So subscription backends are probed with an
   auth-only call (`GET /v1/models`), which validates the credential and costs
   nothing. Metered backends may use a real 1-token request.

   `doctor` also skips any backend that is authenticated but not consented:
   probing it would use the very credential the user has not authorised.
5. **Cross-family acceptance.** `tests/claude_code_on_nearai.rs` runs a Claude
   Code-shaped session against an exhausted Anthropic backend and a NEAR AI
   stand-in: it asserts the fallback happens, that the client receives a
   well-formed Anthropic stream, that a NEAR AI tool call arrives as a valid
   `tool_use` block whose id round-trips, that a mid-loop conversation does
   **not** switch, and that the same conversation does switch one turn later.
6. **Agent-level acceptance.** A scripted Claude Code task (edit a file, run a
   test, fix the failure) must complete through IronWire with the same
   turn count as direct. This is the only test that catches subtle behavioral
   regressions from a mis-mapped field.

---

## 8. Compaction

Every serious harness compacts: as a conversation approaches the context limit
it asks the model to summarize what has happened so far, then continues from
that summary. IronWire sees the compaction turn as an ordinary request, which
is convenient and also the source of the problem — **it is not an ordinary
request, and treating it as one is wrong in three separate ways.**

| Harness | Trigger | Reaches us as |
|---|---|---|
| Claude Code | auto near the limit, plus `/compact`; the trigger is computed from `POST /v1/messages/count_tokens` | `POST /v1/messages` |
| Codex | auto, plus `/compact` | `POST /v1/responses` |
| Aider | chat-history summarization above a threshold | `POST /v1/chat/completions` |
| Cline / Roo | "Condense context" | whichever façade is configured |

### What makes it different

**Its output is permanent.** An ordinary turn is disposable — a bad answer is
visible and retryable. A compaction summary *becomes the conversation*: it is
written into the client's history and resent on every subsequent turn for the
rest of the session. A degraded summary degrades everything after it, and the
user has no way to tell that is what happened.

**It is the largest request of the session and the slowest.** The whole history
goes up; a long summary comes back after a long think. It is therefore the turn
most likely to hit a stall, an overload, or a read timeout — precisely where
`resilience` (§5) matters most, and precisely where a same-backend retry is
worth more than descending the ladder.

**Our own turn-boundary rule permits a family switch here.** `mid_tool_loop` is
false during compaction — it is a clean boundary by construction. So the
cross-family gate in `capability::eligible` *allows* a switch at the one moment
in the session when a switch is most expensive. The gate is not wrong; it is
answering a different question (is a switch *correct*) than the one that matters
here (is a switch *wise*).

### Consequences

1. **Fidelity should dominate cost on a compaction turn.** Descending a rung to
   save money buys one cheaper request and pays for it on every turn afterwards.
   Not yet implemented — tracked in `ROADMAP.md`.
2. **Recognition must stay optional.** Fingerprinting each harness's compaction
   request is the obvious approach and is exactly the kind of client-shape
   detail that breaks silently on a client update. Any fingerprint belongs in
   the signed catalog channel (`docs/UPDATES.md`), and nothing may depend on it
   being right — behaviour without recognition must be correct, just less
   tuned.
3. **`count_tokens` must keep working and must be routed like the turn it
   predicts.** Claude Code computes its compaction trigger from it (§1). It
   already goes through the same routing so the count comes from the model that
   will serve the turn; if that ever stops being true, the client will compact
   at the wrong moment in both directions.
4. **Affinity survives, and should.** The conversation key is derived from the
   system preamble and tool names (`DESIGN.md` §3), which compaction does not
   change. A compacted conversation keeps its sticky route — which is what we
   want, since the provider-side prompt cache is about to be rebuilt either way.
5. **For the privacy filter, this is the hardest case in the product.** An
   unreversed placeholder in a summary is permanent, self-perpetuating
   corruption of the user's transcript. See `docs/PRIVACY.md` §5.

---

## Codex clients

The CLI and the desktop app read the same `~/.codex/config.toml` and the same
`~/.codex/auth.json`, so `ironwire connect codex` configures both with one edit
and neither needs a separate mechanism. Two consequences follow, and both are
stated by that command before it writes anything:

- "Restart Codex" means different things. A CLI session picks the change up on
  its next start; the desktop app has to be quit and relaunched, because
  reopening a window keeps the old config.
- Codex offers no UI for changing the model on a custom provider
  ([openai/codex#15364](https://github.com/openai/codex/issues/15364)), so a
  desktop thread keeps the model it was created with. That is Codex's
  limitation, but pointing it at IronWire is what makes a user meet it.

**More than one `CODEX_HOME`.** Codex keeps everything per home — `auth.json`,
`config.toml`, its installation id — so a second ChatGPT login is a second home,
and IronWire reads it as a second backend (`DESIGN.md` §6). `ironwire connect
codex --config-dir ~/.codex-work` writes the `[[backends]]` entry and points
*that* home's `config.toml` at the login-bound URL,
`http://127.0.0.1:8463/openai/account/codex-work/v1` (§1); the default home is
left as it was. Each home's requests carry its own bearer and its own
`chatgpt-account-id` (§2.2), and are served by its own subscription — by
another only with cross-account fallback on, and then never mid-conversation
(§6).

Two rules about homes are Codex's, not IronWire's, and the not-logged-in
message names them. A `CODEX_HOME` that does not exist is refused by
Codex, so `mkdir -p` comes before `CODEX_HOME=~/.codex-work codex login`. A home
in keyring mode (`cli_auth_credentials_store = "keyring"` or `"auto"`) has no
`auth.json`, and IronWire does not read Codex's keyring item; set it to `"file"`
in that home's `config.toml` and log in again.

The default login is `~/.codex`, or `$CODEX_HOME` when the daemon's own
environment sets it — unchanged, because existing setups rely on it. A home you
use from a shell belongs in `config_dir` instead: a daemon under launchd or
systemd does not see your shell's variables. Whether the desktop app can be
pointed at a home other than `~/.codex` at all has not been checked.

**What has not been verified.** The client-identity gate (`docs/TRUST.md` §3)
recognises the CLI by its `originator` header and its `instructions` preamble.
Neither has been observed from the desktop app, which is an Electron client with
a different HTTP stack — so it is not known whether the app reaches the ChatGPT
subscription through IronWire or is refused as an unrecognised client. That
needs a capture from a running app, and until there is one this section states
the uncertainty rather than implying coverage. The markers live in the signed
catalog channel precisely so the answer can ship in minutes rather than a
release.

The admission capability also reports `body_capture_ready`: true only when the
separate capture flags are enabled and both local ledger and body storage opened.
It reports readiness without enabling capture or exposing stored content.
