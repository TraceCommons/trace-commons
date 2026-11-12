# Privacy filter

**Status: tiers 1 and 2 built; tier 3 not started.** This document is the plan,
the critique of the plan, and now the record of what was actually built.
`ROADMAP.md` M7 tracks the rest.

An optional, off-by-default filter that removes sensitive values from requests
on the way out and restores them on the way back, so a coding agent can run
against a provider the user does not fully trust with their data.

---

## 1. What this is, and what it is not

IronWire sits at the one place in a coding agent's life where every byte
destined for a model passes through code the user controls. That is the natural
home for a privacy filter, and it is the only place one can be added without
modifying the agent.

It is **not** a compliance control, and this document will not describe it as
one. A filter of this kind is a *risk reducer with a known false-negative rate*.
Sold as a guarantee it is actively harmful, because it changes what people are
willing to paste into an agent. The single most important product decision here
is that the UI never claims more than the mechanism delivers (§7).

---

## 2. Three tiers

Each tier is strictly more expensive and strictly less predictable than the one
before it. They compose: a later tier runs in addition to, not instead of, the
earlier ones.

| Tier | What it finds | Cost | Failure mode |
|---|---|---|---|
| **0 — off** (default) | nothing | zero | — |
| **1 — secrets** | API keys, tokens, private keys, connection strings — things with a *machine-checkable shape* | microseconds; Aho-Corasick + regex | misses a key format nobody has written a pattern for |
| **2 — named values** | exact strings the user nominated: their real name, their employer, a customer's domain | microseconds | misses inflections and encodings of those strings |
| **3 — inferred PII** | names, addresses, phone numbers, account numbers in free text, found by a local model | 10s–100s of ms per new block; a model in memory | both directions: misses real PII, and flags code as PII |

Tier 1 and 2 are deterministic and reviewable: given the same input they
produce the same output, and a user can be shown exactly what matched. **Tier 3
is neither**, which is why it is a separate tier and not "tier 1, but better".
A non-deterministic filter on the request path of an interactive tool needs a
much higher bar than a batch scrubber does.

### What ironclaw already provides

| Need | Crate | Verdict |
|---|---|---|
| Tier 1 patterns + severity/action model | `ironclaw_safety::{LeakDetector, LeakPattern, LeakMatch, LeakSeverity, LeakAction}` | **reuse directly.** ~15 maintained patterns (OpenAI, Anthropic, AWS, GitHub classic + fine-grained PAT, Stripe, Slack, Twilio, SendGrid, Google, PEM/OpenSSH private keys, bearer tokens), Aho-Corasick prefiltered. Exactly the boring, high-value layer we should not write twice. |
| Tier 2 exact-value scrubbing, including URL-encoded variants | `ironclaw_safety::{redact_exact_values, redaction_values_for_secret}` | **reuse.** The encoding-variant expansion is the part that is easy to get wrong. |
| Redacting the *ledger* by key name | `ironclaw_reborn_traces::redact_sensitive_json` | **reuse** for `capture.bodies`, which is a different problem from this one — see §8. |
| Tier 3 | — | nothing. |
| **Reversible** substitution | — | **nothing.** Every ironclaw path above is one-way: it replaces a match with `[REDACTED]` and the original is gone. |

That last row is the whole engineering problem. Redaction is solved and we
should not reinvent it. *Reversal* is unsolved, and it is where every hard case
in this document lives.

---

## 3. Why reversal, and what it costs

One-way redaction is useless on the request path of a coding agent. If the model
receives `[REDACTED]` where a hostname was, it writes code against
`[REDACTED]`. The value has to come back.

So: substitute on the way out, restore on the way back.

This breaks the native lane's central claim. `docs/PROTOCOL.md` §2 enumerates
every mutation IronWire makes, and byte-identity is pinned by
`tests/passthrough.rs`. **A privacy filter is, by construction, a mutation of
the request body and the response stream.** That is not a caveat to be
footnoted; it is a different operating mode:

- The filter is **off by default** and turning it on is an explicit, per-profile
  choice.
- With it on, `ironwire status` says so, on its own line, permanently — not a
  startup message that scrolls away.
- The ledger records, per exchange, whether the filter was active and how many
  substitutions it made. An exchange that was filtered is not comparable to one
  that was not, and the log must not imply otherwise.
- `tests/passthrough.rs` keeps asserting byte-identity with the filter **off**.
  A parallel suite asserts the *only* differences with it on are the
  substitutions the filter reports (§9).

---

## 4. The substitution map: derive it, never store it

The obvious design is a persistent per-conversation map from plaintext to
placeholder. It is the wrong one.

A stored map is a file containing every piece of PII the user has ever sent,
in plaintext, next to the token that indexes it. We would have built a
purpose-made PII database in service of *not* exposing PII. It is also a
correctness liability: it can drift from the conversation, it needs eviction,
and it survives a `--clear` the user thought was a clear.

**The map is derived fresh from each request and dropped when the request
ends.** This works because of a property of the whole product: coding agents
are stateless over HTTP and resend their entire history every turn. So every
turn, we see all the plaintext again and rebuild the same map.

For that to hold, substitution must be **deterministic**: the same plaintext, in
the same conversation, must always produce the same placeholder. Keyed as
`HMAC(conversation_salt, normalized_plaintext)`, truncated — with
`conversation_salt` random per conversation and held only in memory, so the same
email in two different conversations gets different placeholders and the token
carries no offline-crackable information about its plaintext.

Three consequences worth stating plainly:

1. **Restoration on the way back only ever un-substitutes tokens this request
   minted.** A model that invents a placeholder-shaped string gets it passed
   through untouched. We never map a token we did not issue.
2. **Restored PII is what the client stores.** The client's transcript keeps
   real values; only the provider ever sees placeholders. This is the right way
   round — the user's own machine is not the threat model — and it is what makes
   the map re-derivable next turn.
3. **Nothing persists.** Restart the daemon mid-conversation and the salt
   changes, so placeholders change. Harmless: the map is per-request, and the
   provider has no memory of the old tokens either. This is the one place where
   "it does not survive a restart" is a feature.

---

### `full`: a constraint on the destination, not on the content

Substitution has a false-negative rate nobody can measure on someone else's
data, so it cannot answer "this must not reach Anthropic". `mode = "full"` does,
by making the trusted set a routing *eligibility* rule: a backend the user did
not name stops being a candidate, and when none of the named ones can serve a
request IronWire refuses it and says so. A pin and an `X-IronWire-Route` header
cannot relax it — a per-request header able to override a config-level privacy
constraint would make the setting decorative.

What it does **not** do: the trusted backend still receives the request in full.
`full` means "only these destinations". It does not mean encrypted, anonymous,
or safe, and no string this feature prints says otherwise (`docs/TRUST.md` I7).
Substitution at the `pii` level still runs on the way there — trusted is not the
same as unfiltered.

There is no default trusted set. Shipping one would be IronWire asserting that
some operator is trustworthy for someone else's data; the daemon refuses to
start under `full` with an empty list instead.

### Measured cost

`credentials` costs ~1.5 ms per turn on a 333 KB history; `pii` costs ~8 ms —
three more deterministic passes over the same text. Both are invisible next to a
model's time to first token, which is why there is still no detection cache
(`crates/ironwire_privacy/tests/cost.rs` asserts the budget and prints the
figure). A tier-3 classifier would not be, and that is where the per-block cache
becomes necessary rather than optional.

## 5. Compaction, per harness

This is the hard case, and the one most likely to produce a permanent, visible
failure.

Every serious coding harness compacts: when the conversation approaches the
context limit, it asks the model to summarize the conversation so far and
restarts from that summary. The summary is then **permanent client-side
history** — it is resent on every subsequent turn, forever.

| Harness | Mechanism | Reaches us as |
|---|---|---|
| Claude Code | auto-compact near the limit, plus `/compact`; drives the trigger off `count_tokens` | `POST /v1/messages` |
| Codex | `/compact` and auto-compaction | `POST /v1/responses` |
| Aider | chat-history summarization above a threshold | `POST /v1/chat/completions` |
| Cline / Roo | "Condense context" | whichever façade is configured |

### Why it is dangerous

A normal turn is disposable: if reversal goes wrong, one response is wrong and
the user sees it immediately. A compaction response is not disposable. If a
placeholder survives unreversed into a summary:

- It is written into the client's permanent history.
- It is resent every turn from then on.
- Next turn we do *not* recognize it — the salt-derived map is rebuilt from the
  plaintext in the request, and a stale token is not in it — so it is never
  reversed. The corruption is permanent and self-perpetuating.

And the summarizing model has every incentive to mangle it. Summarization is
exactly the operation that paraphrases, truncates, reformats, and translates.
`⟦pii.email.7f3a⟧` is far more likely to come back altered from "summarize this
conversation" than from "fix this test".

### The design must not depend on detecting compaction

The tempting move is to fingerprint each harness's compaction request and
handle it specially. That fingerprint is precisely the kind of thing that
breaks silently on a client update — which is what the signed catalog channel
exists for (`docs/UPDATES.md`), and it is where any such fingerprint will live.

But recognition must be an **optimization, not a dependency**. Correctness comes
from a rule that needs no recognition:

> **Any response containing a placeholder this request minted must be fully
> reversed, or the exchange fails loudly.** A partially-reversed response is
> never forwarded.

"Fails loudly" means: surface a provider-shaped error the agent already knows
how to handle, log what could not be reversed with the plaintext elided, and
leave the client's transcript untouched. A failed turn the user can retry is
vastly better than a corrupted transcript they will not notice for a week.

**What that rule can and cannot catch, stated rather than implied.** A
*truncated* token is recognisable: its prefix matches one we minted and then the
stream diverges or ends, so it fails the exchange. A token whose *middle* was
rewritten — `⟦named. abc⟧` — is not distinguishable from one the model invented
out of nothing, and guessing would mean fuzzy-matching arbitrary model output
against our tokens, where a false positive fails a working session.

So a rewritten token is passed through untouched and **counted**. It is never
mapped to a wrong value; the user's real data does not appear. What they get is
a visible oddity in one response and a non-zero `passed_through` count in
`ironwire log` — which is the signal that something went sideways. That is the
honest boundary of this mechanism, and it is why §7 forbids the interface from
claiming more.

Recognition, when it works, buys us three things on top: raise the strictness
for that request, prefer format-preserving surrogates (§6) which survive
paraphrase better, and warn the user *before* a summary is committed.

### Cache invalidation

Detection over the whole history every turn is O(history), and this document
originally assumed that would need a per-block cache keyed by content hash.

**Measured instead** (`ironwire_privacy/tests/cost.rs`): tiers 1 and 2 cost
**~1.5 ms per turn on a 333 KB history** — the size at which a harness compacts.
Against a model's time-to-first-token of hundreds of milliseconds that is
invisible, so there is no cache, and the correctness hazard a cache would
introduce across a compaction boundary does not exist. The Aho-Corasick
prefilter in `ironclaw_safety::LeakDetector` is doing the work.

The test carries the budget as an assertion, so if a future tier pushes the cost
past ~50 ms it fails and says which paragraph of this document to come back to.

**Tier 3 will need the cache**, because a local model is orders of magnitude
slower than a regex sweep, and there the invalidation rule stated above applies:
compaction replaces the history rather than appending to it, so the cache is
invalidated wholesale — one expensive turn, then cheap again.

---

## 6. Opaque tokens vs. format-preserving surrogates

Two substitution styles, and the choice is not obvious.

**Opaque** — `alice@corp.com` → `⟦pii.email.7f3a⟧`. Unmistakable, trivially
found on the way back, cannot collide with real data.

**Format-preserving** — `alice@corp.com` → `u7f3a@example.invalid`. Looks like
what it replaced.

Opaque tokens have a failure mode that reversal cannot fix. The model does not
merely *copy* values; it *reasons about their structure*. Asked to write a
validator for an email, given `⟦pii.email.7f3a⟧`, it writes a validator for
that shape. Asked to sort addresses, it cannot. Reversal restores the value and
leaves the code wrong. **Substitution preserves referential identity and
destroys structure, and in a coding agent the structure is frequently the
point.**

Format-preserving surrogates keep the model able to reason, at the cost of being
harder to find on the way back and able to collide with genuine data.

The plan: **opaque by default; format-preserving per class where structure is
load-bearing** (emails, IPs, phone numbers, dates), selected by config. Both
reverse through the same map. Measured against the corpus in §9 — this is a
question to answer with data, not taste.

### The related trap: PII-shaped strings that are code

In a coding agent, a large share of what a PII detector flags is not PII:
`user@example.com` in a test assertion, `192.168.1.1` in a fixture, a phone
number in a validation test's expected output. Substituting these makes the
model write code that does not compile or tests that do not pass, and the user
blames the model.

Mitigations, in order of expected value: never substitute inside fenced code
blocks or `tool_result` content by default; never substitute reserved/example
ranges (`example.com`, `example.org`, RFC 5737 and RFC 1918 addresses, `555`
numbers); and require tier 3 to justify a match against surrounding context
rather than shape alone.

**The corpus test found one of these before a user did.** `high_entropy_hex`
matched every lockfile checksum and git SHA. Substituting one is not a small
annoyance: the model rewrites `Cargo.lock` around a placeholder, the build
breaks, and nothing in the failure points at IronWire. That pattern — and
`high_entropy_base64` — now require a *credential word* within 48 characters
before the match (`token`, `secret`, `Bearer`, …). The cost is stated rather
than hidden: a bare hex secret with no surrounding context is missed. In a
repository, that is the right side of the trade.

---

## 7. What the UI is allowed to claim

The mechanism has a false-negative rate that we cannot measure on the user's
actual data. So:

- The status line says what is **on**, never what is *safe*. "Privacy filter:
  secrets + named values" — not "your data is protected".
- `ironwire privacy check <file>` runs the configured filter over a file and
  shows exactly what it would and would not have caught, so the rate is
  something a user can see for themselves before trusting it.
- Every exchange's substitution count is in `ironwire log`. Zero substitutions
  on a turn full of customer data is the signal that the filter is not doing
  what the user assumed, and it should be visible without being asked for.
- No green checkmarks. No "protected" badge. This is the difference between a
  tool that reduces risk and a tool that manufactures confidence.

---

## 8. Relationship to trace capture

Distinct problems, and conflating them would be a mistake:

| | Privacy filter | `capture.bodies` redaction |
|---|---|---|
| Protects against | the **provider** | our own **ledger on disk**, and any later upload |
| Direction | must reverse | one-way; nothing reads it back |
| Failure cost | a corrupted transcript | a leaked log |
| Reuse | new code (§2) | `ironclaw_reborn_traces::redact_sensitive_json` and `ironclaw_trace_commons` |

They share the tier-1 detector and share nothing else. The ledger records
**post-substitution** bodies when the filter is on, so a filtered exchange
cannot leak through the log the filter was there to prevent — and records the
substitution *count* so the log is not silently misleading about what was sent.

---

## 9. Test plan

"Test carefully" is the requirement, so it is specified before the code.

**Round-trip properties** (property tests over generated documents)

1. With the filter off, the request body is byte-identical upstream and the
   response byte-identical downstream. This is `tests/passthrough.rs` and it
   does not change.
2. With the filter on, restoring the substituted request yields the original
   request exactly. Substitution is injective; no two distinct plaintexts share
   a placeholder within a conversation.
3. Determinism: the same plaintext in the same conversation yields the same
   placeholder across turns, across requests, and across a compaction boundary.
4. Substitution never straddles a JSON string boundary and never produces
   invalid JSON, including for values containing quotes, backslashes, newlines,
   and non-BMP characters.
5. Placeholders survive `serde_json` round-tripping with `preserve_order`.

**Streaming reversal**

6. A placeholder split across *any* SSE chunk boundary — tested at every byte
   offset — is reversed correctly. This is the most likely place for a
   silent bug: the response arrives in chunks that respect nothing.
7. Reversal never emits a partial placeholder, even if the stream ends
   mid-token; a truncated stream is reported as truncated.
8. The stream is not stalled by buffering: the reversal buffer holds at most one
   maximum-placeholder-length worth of bytes, asserted under the same
   cancellation and keepalive tests `resilience` already has.

**Compaction** (one case per harness in the §5 table)

9. Replay a real compaction request for each harness against a mock that echoes
   placeholders verbatim → the summary is fully reversed and the client's stored
   history contains no placeholder.
10. Same, against a mock that *mangles* the placeholder (case-changed, split by
    a newline, wrapped in backticks, paraphrased away) → the exchange fails
    loudly and the client's transcript is untouched. **Never a partial write.**
11. A stale placeholder from a previous salt appears in the history → it is
    passed through untouched and never reversed to the wrong value.
12. Cache invalidation across a compaction boundary is measured, not assumed.

**False positives that break code**

13. A corpus of real-looking source files — test fixtures with example emails,
    RFC 1918 addresses, `555` phone numbers, seeded UUIDs — passes through tier
    1 and 2 with zero substitutions.
14. Content inside fenced code blocks and `tool_result` blocks is not
    substituted under default config.

**Tier 3 specifically**

15. Measured precision and recall against a labelled corpus, published in the
    docs as a number. A tier that cannot state its error rate does not ship.
16. A bounded per-request latency budget, enforced: exceeding it degrades to
    tier 2 and says so, rather than stalling the user's agent.
17. The local model runs offline. A test asserts the tier-3 path makes no
    network call — a privacy feature that phones out is the worst possible
    outcome and exactly the kind of thing that arrives via a dependency.

**Adversarial**

18. Model output that echoes a placeholder inside a tool call, a diff, a regex,
    a URL-encoded string, and base64 — reversal is correct or the exchange fails.
19. Model output that invents placeholder-shaped strings never maps to a real
    value.

---

## 10. What we will not do

- **Claim compliance.** No GDPR/HIPAA language. The mechanism does not support
  it and saying so would be the most harmful thing on this page.
- **Filter by default.** A mutation of the request path is opt-in, always.
- **Send anything to a remote classifier.** Tier 3 is local-only, and I2 already
  makes the network shape of this unavailable to the catalog channel.
- **Persist the map.** §4.
- **Silently pass a partially-reversed response.** §5.
