# Enriching envelopes with IronWire's routing ledger

`nearai/ironwire` is a loopback proxy that sits at the inference boundary:
Claude Code and Codex point at `127.0.0.1:8463`, and it routes each turn to a
subscription, an API key, or NEAR AI. It keeps a local SQLite ledger of every
exchange it routes, including the cost and token counts our own scrapers cannot
see.

This spec covers reading that ledger from the contributor daemon and attaching
it to the traces we already build. It does not cover contributing *through*
IronWire, and it deliberately forecloses that.

## Why now rather than later

IronWire's own docs commit its upload path to `ironclaw_trace_commons`
(nearai/ironclaw, `crates/domains/`) in four places — `DESIGN.md` §7,
`TRUST.md` §4, `CRITIQUE.md` §7, `ROADMAP.md` M6. That is a second contributor
implementation whose capture entry point takes a flat
`ConversationMessage { id, role, content: String, created_at }`: no tool calls,
no structured events, no failure modes. An envelope built through it loses
exactly the structure our gate scores on.

It is not wired yet. IronWire's `Cargo.toml` carries `ironclaw_llm`,
`ironclaw_common` and `ironclaw_safety`, and not `ironclaw_trace_commons`; M6 is
unbuilt. The seam is open now and closes when someone builds M6.

## What the ledger actually holds

`crates/ironwire_ledger/src/lib.rs:186-217`, per exchange: `started_at`,
`ttfb_ms`, `total_ms`, `facade`, `path`, `conversation`, `backend`,
`requested_model`, `served_model`, `rung`, `attempts`, `input_tokens`,
`cache_read_tokens`, `cache_write_tokens`, `output_tokens`, `cost_usd`,
`substitutions`, `status`, `error`.

Three corrections to what the documentation implies:

- **`capture.bodies` is a dead flag.** It is declared at
  `ironwire_core/src/config.rs:122-124` and referenced in exactly two places in
  the tree — that declaration's test assertion (`config.rs:1240`) and a doc
  comment (`ironwire_ledger/src/lib.rs:12`). Nothing reads it and nothing writes
  a body anywhere. `docs/PACKAGING.md:103` documents a `$IRONWIRE_HOME/bodies/`
  store that does not exist. The ledger is metadata-only, unconditionally.
- **`ttfb_ms` is always NULL.** One writer, `ironwire_proxy/src/pipeline.rs:895`,
  hardcoded `None`. Do not design around time-to-first-byte.
- **`cost_usd` is priced, not billed.** It is computed from observed tokens
  against a price table (`ironwire_ledger/src/lib.rs:29-35`). Work served on a
  subscription is priced at what it *would* have cost on the meter. Nothing
  downstream may present it as money the contributor spent.

The first of these is load-bearing for the whole design: IronWire *cannot*
supply message content even if someone wanted it to. Our scrapers remain the
sole source of trace bodies, and there is no version of this where the proxy
becomes the capture path.

## The join key: `conversation` cannot do it

`ConversationKey::derive` (`ironwire_core/src/policy.rs:69-84`) hashes three
things with `std::hash::DefaultHasher`: the protocol family, the first 512 bytes
of the system preamble, and the sorted tool-name list. Messages are excluded
deliberately — the comment at `policy.rs:65-67` reads "a key that changes every
turn is not affinity, it is noise" — and a test asserts turn 1 and turn 40 hash
identically.

That is correct for its purpose, which is routing stickiness. It makes the key
**stable, not unique**:

- two Claude Code sessions on one machine with the same tool set collide;
- the same session yesterday and today collide;
- `DefaultHasher` is seeded with fixed zero keys, so it collides across
  machines.

The row carries no pid, no cwd, no client port, and no client-supplied id;
`Exchange` is fully enumerated at `ironwire_ledger/src/lib.rs:50-101`.

**So the two sides cannot be joined on `conversation`.** A join on it would not
merely be imprecise — it would silently attribute one session's costs to another
contributor's trace.

### But both clients already send their session id, and IronWire already sees it

Captured live on 2026-09-01 by pointing each client at a logging endpoint
(credential-bearing headers redacted at capture).

**Claude Code 2.1.252** sends on every request:

```
X-Claude-Code-Session-Id: 5db811ed-ce4a-45a7-ab00-56890e111668
```

That value is the transcript filename stem —
`~/.claude/projects/<project>/5db811ed-ce4a-45a7-ab00-56890e111668.jsonl` — and
exactly what `ClaudeCodeSource` resolves as `conversation_id`. One match across
all projects.

**Codex 0.151.0** sends `session-id`, `thread-id` (equal for the main session),
`x-client-request-id` and `x-codex-window-id`. The `session-id` value
`01a05a5d-243a-7ac0-bdcd-ba06b4309c36` is the uuid suffix of the rollout
filename `rollout-2026-08-31T17-27-28-01a05a5d-243a-7ac0-bdcd-ba06b4309c36.jsonl`.

**IronWire forwards both.** `ironwire_upstream/src/headers.rs:25-37` denylists
only hop-by-hop headers, the client's own credentials (`authorization`,
`x-api-key`, `api-key`), and three rewritten connection headers (`host`,
`content-length`, `accept-encoding`). Neither session header is on any list, so
the proxy already has the value on the inbound request — it simply does not
record it.

This makes the upstream ask far smaller than parsing a response id, and it
changes the Codex answer entirely.

One caveat found in the same capture: Codex emits auxiliary requests carrying
`x-codex-turn-metadata: {"request_kind":"memory","thread_source":"memory_consolidation"}`
under a *different* `session-id` that maps to no rollout file. Those rows join to
nothing, which is the already-expected steady state below, not an error.

## The upstream ask

Two additive changes, one PR. Neither adds an upload path, a consent surface, or
credential exposure, so IronWire's trust invariants I1-I6 are untouched.

### Ask 1 — record the client's session id

One nullable column in the `SCHEMA` literal
(`ironwire_ledger/src/lib.rs:186-217`):

```
client_session_id TEXT
```

plus `pub client_session_id: Option<String>` on `Exchange` (`:50-101`). Three
call sites follow mechanically: the insert at `:257-270`, the `COLUMNS` constant
at `:455`, and `read_exchange` at `:463`.

The value is read from the **inbound request headers**, which IronWire already
has and already forwards. A small facade-specific mapping:

| Facade | Header |
|---|---|
| anthropic | `x-claude-code-session-id` |
| openai | `session-id` |

No response parsing, no translate-layer change, nothing touched in the streaming
decoder. The carrying pattern already exists — `Observation.served_model`
(`ironwire_upstream/src/observe.rs:86`) is the same shape, consumed in
`LedgerContext::write` at `ironwire_proxy/src/pipeline.rs:900`. Adding
`client_session_id` beside it makes the diff a field, not a mechanism.

Framing for the maintainers, on their own terms: it is a client-supplied opaque
identifier already present in headers IronWire forwards unmodified, it carries no
message content, and it makes `ironwire log` correlatable with the agent session
that produced each exchange — a benefit to a user who will never share anything,
which is the bar `ironwire_ledger/src/lib.rs:6-11` sets for what the ledger may
record.

An earlier draft of this spec asked for the provider's *response* id instead,
parsed out of the response body. That ask was strictly larger — it required
touching `ironwire_translate/src/anthropic.rs:420-427` and
`stream.rs:536-541` — and it did not work for Codex at all. It is superseded.

**Migration caveat.** `Ledger::init` runs `execute_batch(SCHEMA)`
(`ironwire_ledger/src/lib.rs:243`) against a `CREATE TABLE IF NOT EXISTS`, which
is a no-op on an existing ledger. The ask must specify an `ALTER TABLE exchanges
ADD COLUMN` guard. Whether IronWire has a migration mechanism elsewhere was not
exhaustively verified — raise it as a question in the issue, not as an
assertion.

### Ask 2 — wire `since` into `/_ironwire/log`

`LogQuery` has one field, `limit`, defaulted to 20 and clamped to 1000
newest-first (`ironwire_proxy/src/control.rs:439-449`, handler `:1287-1313`,
clamp `:1305`). `Ledger::since(from)` already exists
(`ironwire_ledger/src/lib.rs:329-341`) and is already called from
`control.rs:1086` for the status handler. It is simply not reachable from
`/log`.

Same PR, because without it a busy machine drops rows between polls — which
makes the join silently *incomplete* rather than *absent*, the worse of the two
failure modes.

### Not bundled: a read-scoped token

The control token is one token for the whole API, and that API includes
`POST /_ironwire/tools`, described at `control.rs:521-533` as "the one endpoint
that writes to a file outside `$IRONWIRE_HOME`" — it rewires the user's agent
configs. Reading the ledger therefore means our daemon holds the ability to do
that.

A read-only or `log`-scoped token is the clean fix, and it should be a separate
issue. Bundling it turns the first PR into a security conversation.

## Both clients join, exactly

Measured, not inferred — see the capture above.

| Client | Header | Joins to |
|---|---|---|
| Claude Code 2.1.252 | `X-Claude-Code-Session-Id` | transcript filename stem, `ClaudeCodeSource`'s `conversation_id` |
| Codex 0.151.0 | `session-id` (and `thread-id`) | uuid suffix of the `rollout-<ts>-<uuid>.jsonl` filename |

Both are exact one-hop joins against the identifier each adapter already
resolves. No heuristic time-window join is needed for either, and the
Claude-Code-only staging an earlier draft recommended is withdrawn.

The values are client-supplied, so they inherit the attribution-only rule below
like everything else in the overlay: a session id may address a row, and may
never authorize anything.

## The seam on our side is a decorator, not a source

`TraceSource` (`crates/trace-commons-contributor/src/source/mod.rs:148-152`) is
`discover() -> Vec<SessionRef>` plus `load(&SessionRef) -> SessionTranscript`
plus a path-to-session mapping for the file watcher. Registering IronWire as a
fourth source would make its ledger independently discoverable, queueable,
previewable and submittable, because everything downstream keys on a
`SessionRef` with a `path`, a `size_bytes` and a `group_modified_at`
(`source/mod.rs:57-72`). The ledger is not a session. It is an overlay on
sessions our scrapers already build, and all three trait methods would have to
lie.

**Use a decorating `TraceSource`** that wraps each adapter and overrides only
`load`. The codebase already demonstrates the shape works against the real
trait: `CountingSource` (`daemon/watcher.rs:806-827`) wraps a genuine adapter
and delegates `name`/`discover`/`load`, with `:801-805` explaining why wrapping
beats substituting a fake. It is test-only today; there is no production
decorator.

The decorator beats threading a parameter because there are three production
`load` call sites (`submit.rs:441`, `daemon/preview.rs:559`,
`daemon/watcher.rs:520`) and four public envelope builders (`envelope.rs:401`,
`:419`, `:448`, `:466`) funnelling into `build_raw_contribution_with_id`
(`:481`). Threading an `Option<&RoutingOverlay>` touches all of them. The
decorator has one insertion point: `all_sources()` (`source/mod.rs:350-383`),
the single registry the daemon, the preview path and the CLI all go through
(`commands.rs:644`, `:870`).

Concretely:

- New module `crates/trace-commons-contributor/src/routing/`. A trait
  `RoutingLedger { fn exchanges_since(&self, from: DateTime<Utc>) -> Result<Vec<RoutedExchange>> }`
  and one impl, `IronWireLedger`, doing an HTTP GET against
  `127.0.0.1:<port>/_ironwire/log`. Trait object, per the gate-seam convention
  in CLAUDE.md.
- **Zero IronWire crates as dependencies** — HTTP and serde only.
  `ironwire_ledger` itself depends on `ironclaw_common::llm_costs`
  (`ironwire_ledger/src/lib.rs:32`), and nothing from that tree may enter ours.
  This repo has no Ironclaw path dependency and gains none here.
- `RoutingEnrichedSource<S>` wrapping a `Box<dyn TraceSource>` and an
  `Arc<dyn RoutingLedger>`. `discover` delegates untouched; `load` delegates,
  then joins.
- `SessionTranscript` (`source/mod.rs:114-142`) gains `routing:
  Vec<RoutedExchange>`, defaulting empty — following the existing convention
  that the transcript carries everything the envelope builder needs.
- `raw_events_for` (`envelope.rs:754`) emits one extra event per joined
  exchange. `raw_event_for` (`:799-868`) is untouched: it is per-`SessionEvent`
  and its `latency_ms: None` (`:857`) and `cost_usd: None` (`:865`) hardcodes
  stay, because a routing exchange is not a session event.

### Event type: `RoutingDecision`

Not `HttpExchange`. `HttpExchange` already means "the agent made an outbound
HTTP call" and carries `SideEffectLevel::ReadOnly`
(`trace_contribution.rs:5133`); an inference hop is not a side effect the agent
performed. `RoutingDecision` is correctly `SideEffectLevel::None` (`:5131`).

`HttpExchange` is also not free: it is produced at `trace_contribution.rs:1791`
inside `from_recorded_trace`, live via `pilot_bootstrap/submitter.rs:214`,
`trace-commons-smoke-envelope.rs:124` and `ingest.rs:58058`. It is the
contributor *daemon* that has never produced one. `RoutingDecision` genuinely
has zero producers and zero consumers: outside its declaration
(`trace_contribution.rs:301`) its only references are the presence map (`:4750`)
and the side-effect map (`:5131`).

Fields: `parent_event_id` = the assistant event it belongs to, `timestamp` =
`started_at`, `latency_ms` = `total_ms`, `token_counts` from
`input_tokens`/`output_tokens`, `cost_usd` from the ledger.

The remaining ledger columns — `backend`, `rung`, `attempts`, `requested_model`
and `served_model`, and the cache token split — are strings and counts with no
typed home on `TraceContributionEvent`, so they go in `structured_payload`. That
is what forces the presence-category change in the next section. `redacted_content`
stays empty: the overlay is numbers and labels, never text.

## The presence-flag regression, and why it needs a protocol change

This is the highest-severity item in the design and it fails silently.

`envelope.rs:712-742` derives the envelope's content-presence flags from its
events. A `RoutingDecision` carrying a structured payload such as
`{"backend":"nearai",...}` sets `tool_payloads` via
`payload_carries_readable_content` (`trace_contribution.rs:1151-1161`, presence
map at `:4757-4759`). That pushes envelopes to Medium residual risk and
**quarantines them on a default deployment** (`:4718-4724`) for payloads they do
not carry.

`include_tool_payloads` has never been on anywhere in this project. Flipping it
would silently change what consent every enriched envelope declares, on traces
whose contributors consented to something else.

Putting the numbers in typed fields avoids it — `token_counts`, `cost_usd` and
`latency_ms` already exist on the event. But it does not fully solve it:
`backend`, `rung`, and the requested-versus-served model pair are strings with
no typed home, and `structured_payload` is the only place they can go.

**So the design requires a new presence category,
`EnvelopeContentPresence::routing_metadata`**, in the protocol and honoured by
the server, distinct from `tool_payloads`. This is a protocol and server change
on the critical path, and it is the piece most likely to be discovered late if
it is not written down here.

## Settings and consent

`DaemonSettings` gains `ironwire: Option<IronWireDeclaration>`, mirroring the
`SourceDeclaration` tri-state (`daemon/settings.rs:175-181`) as
`Watch { port } | Off` — with one deliberate divergence.

For session roots, `None` means "never asked" and falls back to the conventional
per-user location (`source/mod.rs:362`, `:374`). `SourceDeclaration`'s own doc
comment (`settings.rs:152-159`) names that as the mistake it was: `None` had to
carry both "never asked" and "I don't use this agent", "and the daemon resolved
that ambiguity by watching the real `~/.claude` or `~/.codex`... So the one
answer a privacy-conscious contributor is most likely to give was the one answer
that silently scanned their work."

**For IronWire, `None` means off, with no fallback.** An unannounced connect to
`127.0.0.1:8463` is a probe of a local service the contributor never mentioned,
and would repeat that error in a new place. `None` constructs no reader;
`Some(Watch { port })` is the only state that reads anything.

The control token is read from `$IRONWIRE_HOME/control.token` at poll time,
never copied into our settings file and never logged.

Consent splits into two questions with different answers:

- **Reading the local ledger** is not a consent-scope question. `VALID_SCOPES`
  (`src/consent.rs:12-18`) governs what the *server* may do with an uploaded
  trace, not what the daemon may read locally. Local reading authority is the
  declaration mechanism, which is why the IronWire declaration belongs beside
  `claude_source`/`codex_source`. `daemon/policy.rs` is the wrong home: it is
  per-project upload autonomy keyed on the session's cwd (`policy.rs:1-16`,
  `:33-42`), and an IronWire declaration is machine-wide.
- **Uploading cost and routing data** rides the same envelope under the same
  grant, so it needs no new scope. But `cost_usd` is a new data class — the
  contributor's own spend — and the consent card should name it explicitly
  rather than folding it under "routing metadata". New disclosure, not a new
  scope.

## Failure and absence

Every failure resolves to the same state: no routing events, envelope
byte-identical to today's. The overlay is `Option` all the way down.

| Condition | Behaviour |
|---|---|
| Not declared (the `None` default) | No decorator constructed in `all_sources()`. The majority case; costs one branch. |
| IronWire not installed | Token read fails; reader resolves to a no-op. Surfaced once in daemon health, not per-poll, not as an error. |
| Daemon not running | Connection refused; empty overlay. |
| Token unreadable, rotated, or another user's | 401 from `control.rs:1317-1335`; empty overlay. Do not retry-loop — a 401 here is a configuration fact, not a transient. |
| Capture off upstream | Handler returns `LogView { enabled: false, exchanges: [] }` (`control.rs:1295-1302`). Already a clean empty case. |
| Malformed JSON after an IronWire release | Parse failure; empty overlay. |
| Rows that join to nothing | Normal steady state. The ledger is machine-wide and covers sessions we do not watch. Dropped silently. |
| Session events with no exchange | Equally normal — IronWire installed mid-session, capture toggled, or retention pruned the row (`capture.retain_days` default 90, `ironwire_core/src/config.rs:136-140`). |

Partial enrichment is the expected outcome, never a warning.

Two constraints make this hold:

1. The join runs after `inner.load` succeeds and never touches `discover`.
   `SessionRef.size_bytes` and `group_modified_at` drive quiescence and
   eligibility (`source/mod.rs:57-72`) and must never depend on the ledger. A
   session is eligible on its own bytes.
2. The read happens before redaction and before envelope sizing, off any
   submission-critical path, with a short timeout. This mirrors IronWire's own
   rule for its ledger writes (`pipeline.rs:857-859`: "a ledger problem must not
   fail a user's inference request").

**Do not model this with a permanent-versus-transient distinction.**
`SessionTooLarge` (`source/mod.rs:38-44`) exists because a size refusal repeats
forever and the daemon must know that. A ledger read failure is the opposite:
the correct behaviour on failure is identical to the correct behaviour on
absence. The distinction would produce health noise for a condition with no
remedy and no consequence.

## What the gate does with this: nothing

Stated plainly, because the temptation to imply otherwise is real.

- `latency_ms`, `cost_usd` and `token_counts` return **zero hits** across
  `crates/trace-commons-server/src`.
- The same grep over `trace-commons-gate-api` and `trace-commons-gate-enclave`
  returns two hits, both an unrelated local `chunk_token_counts` in
  `gate-enclave/src/orchestrator.rs:201,205`.

Scoring is novelty plus perplexity over canonical event text. Routing metadata
does not enter it. This buys provenance and future optionality, not a scoring
change, and whatever ships should say so.

### Why that argues against a heuristic v0

A time-window join is buildable today without any upstream change — transcripts
carry `timestamp` and `durationMs`, the ledger has `started_at`, `total_ms`,
`facade`, `requested_model`. Do not build it. It degrades exactly on
multi-session users, and since nothing consumes the data there is no deadline
that justifies a wrong cost attributed to a trace.

The argument is stronger now that Ask 1 is one nullable column populated from a
header the proxy already forwards. An exact join is available for both clients
for a diff that size; accepting a heuristic to avoid asking for it would be
trading correctness for nothing.

Parallelizable with no upstream dependency: the `routing/` module and decorator
seam, the `IronWireDeclaration` tri-state and consent copy, and the
`routing_metadata` presence change — that last on the critical path.

## Attribution-only, and the gaming vector behind it

`SessionTranscript.conversation_id` carries a deliberate rule: "attribution
only, never a gate or scoring input (issue #298 S4a)" (`source/mod.rs`, field
doc). A client-supplied identifier must not influence scoring.

The same argument applies with more force here. IronWire is Apache-2.0 and runs
on the contributor's machine; they can patch it. If `cost_usd` or token volume
ever feeds credit, a modified IronWire reporting inflated costs is a direct
credit-farming vector — and that lands on top of the finding in the credit
redemption spec that the anti-farming controls (quality, dedup, per-contributor
cap) are all still shadow-mode.

The rule is recorded, and recorded as standing rather than one-off.
`docs/superpowers/specs/2026-08-26-issue-298-followups-design.md:221-223`: "Attribution
only, never authorization — the repo's standing envelope rule. It must not reach
any gate, scoring input, or tenant-scoping decision." CLAUDE.md carries the
other instance under tenant scoping.

**The routing overlay is attribution-only.** `cost_usd`, `token_counts`,
`latency_ms`, `backend`, `rung` and `served_model` are corpus metadata —
genuinely valuable for understanding what a trace cost to produce and where it
ran — and must not reach a gate, a scoring input, **a credit computation**, or a
tenant-scoping decision.

State the credit prohibition explicitly rather than relying on "scoring input"
to cover it. S4a's wording predates the credit work, and someone implementing a
credit-quality function two quarters from now will not obviously read the two as
the same thing.

There is also a non-adversarial reason, and it is sufficient on its own: an
*honest* `cost_usd` is not a quality signal. It scales with prompt size and
model price, so rewarding it would pay for verbosity and for choosing expensive
models — the opposite of what the corpus wants. The field would be a bad scoring
input with zero attackers in the world.

### What actually enforces it today, and the hole

Not convention, but not a written rule either. There is a real structural
barrier that exists for unrelated reasons.

The scorer and embedder never see the envelope. They see a rendered text form
built by a **three-field allowlist**: `parse_envelope_rendered_events`
(`crates/trace-commons-gate-enclave/src/chunker.rs:76-98`) reads `event_type`,
`tool_name` and `redacted_content`, and nothing else, emitting
`kind (tool): content\n`. Numeric fields are structurally unreachable through
this path. The same rendering feeds the dedup simhash
(`trace_gate_service.rs:714-719`).

That is a mechanism, not a comment — but it was not built for this. The reason
given at `chunker.rs:63-65` is signal quality: "Intentionally NOT raw JSON —
braces/keys would dilute the perplexity signal." The dedup reason
(`trace_gate_service.rs:705-713`) is that raw JSON carries per-submission-unique
fields, so identical resubmissions would never collide. Attribution-only is a
side effect of two decisions made for other purposes. Nothing names it and
nothing tests it.

**The hole:** both paths fall back to raw plaintext when
`parse_envelope_rendered_events` returns `None` — which happens when the payload
is not JSON, has no `events` key, or has an **empty `events` array**
(`chunker.rs:77-80`). The scorer path falls back at `chunker.rs:250-256` via
`String::from_utf8_lossy(plaintext)` into fixed windows; dedup at
`trace_gate_service.rs:718`. In that case the raw envelope JSON — `cost_usd` and
all — is exactly what the perplexity scorer consumes.

The fallback is correct as robustness. It just means the barrier is conditional
on a property of the envelope rather than being a property of the pipeline. How
reachable an empty-`events` envelope is end-to-end was not determined, and
should not be assumed unreachable.

This will not survive the credit pipeline leaving shadow mode, because the
credit path is new code that need not go through the chunker at all.

### Two prerequisites, not follow-ups

1. **A test that asserts the rule.** Build an envelope with a distinctive
   `cost_usd` sentinel, run it through the gate path, and assert the sentinel
   appears in nothing the scorer, embedder, dedup or credit function consumes.
   Highest-value item here: it costs little and it fails loudly the day someone
   widens the allowlist.
2. **Make the raw-text fallback not carry metadata** — either refuse an envelope
   with no renderable events, or render the fallback through a
   metadata-stripping pass. Today it silently widens the input surface at
   exactly the moment the envelope is malformed.

Neither is strictly in this join's scope and neither should block it. But
shipping cost data into the corpus without item 1 means the only thing standing
between a patched IronWire and the credit ledger is that nobody has yet written
the code that reads the field.

## Risks

- **The presence-flag regression** is the highest-severity item and is silent.
  Addressed above by the `routing_metadata` category; if that is dropped from
  the plan, the feature quarantines traces.
- **Control-token blast radius** — our daemon would hold a token that can
  rewrite the user's agent configs.
- **`cost_usd` presented as money.** It is priced, not billed. A subscription
  turn shows what it would have cost on the meter. Any surface that renders it
  as spend is wrong.
- **Version skew fails silently.** We parse another project's control-API JSON
  with no shared type. A rename degrades us to an empty overlay — the right
  failure, but invisible — so the daemon must be able to say "IronWire declared,
  reading nothing."

## Open items

- Whether either client's session header is stable across a `--resume` or a
  forked session. The capture covered a single fresh session per client; a
  resumed session reusing or rotating the id changes what a row addresses.
- How Codex's auxiliary requests (`request_kind: memory`) should be treated.
  They carry a session id that matches no rollout, so today they join to
  nothing and are dropped — correct, but it means their cost is invisible
  rather than attributed anywhere.
- How reachable an empty-`events` envelope is end to end, which is what decides
  whether the raw-text fallback is a live exposure or a theoretical one.
- Whether IronWire has a migration mechanism for its ledger schema.
