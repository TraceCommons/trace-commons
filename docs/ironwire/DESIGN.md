# IronWire — design

> IronWire turns your AI subscriptions, API keys and local compute into one
> local inference endpoint. Point Claude Code, Codex or any AI tool at
> localhost; IronWire keeps it on the best available model and degrades
> gracefully when limits hit — so your agent never dies at the rate limit.

This is the finalized design. The review that produced it is in
[`CRITIQUE.md`](./CRITIQUE.md); the trust/consent posture it depends on is in
[`TRUST.md`](./TRUST.md); wire-level detail is in [`PROTOCOL.md`](./PROTOCOL.md).

---

## 1. What IronWire is, and is not

**Is:** a loopback HTTP daemon that presents native provider APIs to coding
agents, and forwards each request to whichever of the user's own capacity pools
can serve it with the required fidelity.

**Is not:** an agent, a harness, a conversation store, a hosted service, or a
credential custodian. The coding agent keeps the loop, the context, compaction,
tool execution and file state. IronWire sees a sequence of inference requests
and decides where each conversation goes.

The load-bearing consequence: **IronWire never needs to understand the task.**
It needs to understand the *wire*.

```
┌─────────────────────────────────────────────────────────┐
│  Claude Code   Codex   Cline   Aider   Zed   custom      │
└────────┬──────────────────┬─────────────────────────────┘
    Anthropic Messages   OpenAI Responses / Chat Completions
         │                  │
         ▼                  ▼
   ┌───────────────────────────────────────┐
   │  IRONWIRE  (127.0.0.1:8463)           │
   │                                       │
   │  façades ── policy ── backends        │
   │      │        │          │            │
   │  observe   fidelity   credential      │
   │  usage &   ladder     resolution      │
   │  limits    + quota    + refresh       │
   │                                       │
   │  local trace ledger (SQLite)          │
   └──────────────┬────────────────────────┘
                  │
   ┌──────┬───────┼────────┬──────────┬─────────┐
   ▼      ▼       ▼        ▼          ▼         ▼
Claude  ChatGPT  Anthropic OpenAI   NEAR AI   local
sub     sub      API key   API key            (ollama/
(OAuth) (OAuth)                                vLLM)
```

---

## 2. Two lanes

This is the central architectural decision. See CRITIQUE §3.

### Native lane — the default path

Inbound protocol == backend protocol. IronWire does **not** deserialize and
re-serialize the body. It:

1. Peeks at the fields it needs for policy (`model`, `stream`, message count,
   approximate size, presence of `cache_control` / `thinking` / images).
2. Chooses a backend.
3. Rewrites the auth headers and the URL; adjusts `model` only if the policy
   changed it.
4. Streams the request body up and the response body down **verbatim**,
   frame-for-frame for SSE.
5. Observes usage and rate-limit state from response headers and from a
   read-only tee of the SSE stream.

Fidelity is 1.0 by construction, including for provider features released after
we shipped: unknown fields survive because we never look at them.

Native pairs in the plan:

| Façade | Backend | Status |
|---|---|---|
| `/anthropic/v1/messages` | Claude subscription (OAuth) | M1 |
| `/anthropic/v1/messages` | Anthropic API key | M1 |
| `/openai/v1/responses` | ChatGPT/Codex subscription (OAuth) | M2 |
| `/openai/v1/responses` | OpenAI API key | M2 |
| `/openai/v1/chat/completions` | NEAR AI, OpenAI-compatible, Ollama | M2 |

### Translated pairs

Every ordered pair of the three wires, through a canonical IR rather than a
translator per pair (`docs/TRANSLATION.md`). All of them are gated on the same
turn-boundary rule.

| From | To | Cost beyond a native forward |
|---|---|---|
| `anthropic.messages` | `openai.responses` | signed thinking, cache breakpoints |
| `anthropic.messages` | `openai.chat` | signed thinking, cache breakpoints, system flattened |
| `openai.responses` | `anthropic.messages` | encrypted reasoning |
| `openai.responses` | `openai.chat` | encrypted reasoning, typed items flattened |
| `openai.chat` | `anthropic.messages` | — |
| `openai.chat` | `openai.responses` | — |

### Translated lane — fallback only

Inbound protocol != backend protocol. IR-mediated, and **capability-gated: a
route that cannot preserve the request's semantics is ineligible, not
best-effort.**

```rust
/// A route is eligible only if the target can serve the request without
/// *breaking* it. A route that would merely be worse is not refused — it is
/// announced.
struct RequestRequirements {
    tools: bool,
    parallel_tool_calls: bool,    // history already issues several calls per turn
    images: bool,
    reasoning: ReasoningNeed,     // informational: continuity lost, not illegal
    prompt_cache: bool,
    cached_prefix_tokens: u32,
    structured_output: bool,
    min_context_tokens: u32,      // prompt estimate (body bytes / 4)
    output_tokens: Option<u32>,   // the client's own max_tokens; on a provider-stated
                                  // window, eligible() reserves it and adds a margin
    mid_tool_loop: bool,          // the cross-family gate
    encrypted_reasoning: bool,    // the cross-account gate (§3)
}
```

The one cross-family correctness rule is **switch at a turn boundary, never mid
tool loop**. A conversation caught mid-loop waits for the next clean turn; it is
not disqualified. Provider-private reasoning state (signed Anthropic thinking,
encrypted OpenAI reasoning) is a *quality* signal, not an eligibility one — a
foreign provider never validates it and the originating API drops rather than
rejects it. `docs/PROTOCOL.md` §6 has the full reasoning, including why an
earlier version of this document got it wrong.

---

## 3. Routing: per-conversation, not per-request

### Conversation identity

IronWire has no session ID from the client. It derives one:

```
conversation_key = H(protocol, client session, client login)  # when session supplied
otherwise: H(protocol family, system_preamble[..512 bytes], sorted tool names, client login)
```

Deliberately not the messages, which change every turn: a key that changes
every turn is not affinity. Keys are memory-only, TTL-evicted, and never
persisted with content.

A request at a login-bound URL (§6) also mixes in the login. Every session of
one client version shares a preamble and a tool set, so without it the Claude
Code on one login and the Claude Code on another would be one conversation,
each evicting the other's route.

Each conversation gets a sticky `Affinity { backend, model, rung, since,
client_login, … }`, reused only by a request from the login it was chosen for.
Beside it, and deliberately outliving it, `Policy` records the native Responses
backend and credential scope for each client session. Subscriptions use the
provider account id; API backends use their actual credential fingerprint.
The affinity is forgotten when a backend fails; the replay origin must not be
(`PROTOCOL.md` §6). Unknown origins refuse native encrypted continuation.

### The fidelity ladder

Fallback is a state transition on the conversation, taken under pressure, and
never reversed within a conversation unless the higher rung recovers *and* the
conversation is at a safe boundary (no in-flight signed reasoning state).

| Rung | Transition | Cache | Reasoning state | User told? |
|---|---|---|---|---|
| 0 | stay | warm | intact | no |
| 1 | same account, smaller model | mostly warm | intact | no |
| 2 | same wire format, different credential: a metered key, or another of your subscriptions (opt-in) | cold | intact, except ChatGPT's (below) | no |
| 3 | different family (needs translation) | cold | dropped | **yes** |

Rung 2's second form is off by default. A client is served by its own login
(§5); with `[subscriptions] cross_account_fallback = true` a client whose own
subscription is under the pressure that triggers every rung — at least 90% used
as the provider reports it, or spent — may be served by another of the user's
subscriptions to the same product, before a metered key. Pressure outranks the
preference for the client's own login, so a fresh selection moves at once, and
a conversation already on that login once the pressure has lasted the
twenty-second descent debounce. Every such request is recorded at rung 2. Why
it is opt-in — the providers' rate-limit terms, and a request body that names
the account it came from — is `TRUST.md` §2.

"Intact" has one exception, and it is enforced rather than tolerated: OpenAI
binds encrypted reasoning to the organization that minted it and rejects it
anywhere else. So a Codex conversation that replays encrypted reasoning never
changes ChatGPT account, even with the opt-in. It stays on the account that
minted it, or crosses families at a turn boundary, where the reasoning is
dropped rather than rejected (`Ineligible::ReasoningBoundToAccount`,
`PROTOCOL.md` §6). The eligibility gate also refuses a descent from the
ChatGPT subscription to a metered OpenAI key for encrypted native replay,
including pins and route overrides. It does not infer an API organization.
Anthropic's documentation calls signed thinking "compatible across platforms"
and says nothing about accounts, so a Claude conversation moved by the opt-in
carries its thinking blocks on an assumption nobody has checked — also in
`ROADMAP.md`.

Hysteresis: a conversation already on a backend changes rung only once the
trigger has persisted past a debounce window, so a single 429 with a 3-second
`retry-after` waits rather than resetting a 200k-token cache. A fresh selection
has no cache to keep: there a pressured backend sorts behind an unpressured one
from the first request.

### Inputs to the decision

| Input | Source | Trust |
|---|---|---|
| requested model | request body | client hint → tier |
| requirements | body peek | authoritative |
| the client's login | the façade URL it was pointed at (§6); the plain URL is the default login's while it is in its default place | authoritative: the user wired it |
| the client's session | its own session header (`PROTOCOL.md` §3) | keys affinity, privacy salt and replay origin |
| approximate prompt size | body byte length + message count | heuristic |
| observed quota | provider rate-limit headers | authoritative, with age |
| recent errors | circuit breaker per backend | authoritative |
| latency (TTFT p50) | local measurement | authoritative |
| user policy | config + `ironwire pin` | overrides all |
| complexity score | `ironclaw_llm::smart_routing` (optional) | weak signal, tier only |

The client's model string is a **quality hint mapped to a tier**
(frontier / balanced / fast) when choosing *between* backends, not a hard
selection — Claude Code cannot type `ironwire/auto`. Once a backend is chosen,
a name that backend's provider reported in its own catalogue is served as
named: it is a selection the provider can serve, and swapping it for a sibling
of the same tier would make the tool's model picker decorative. A compiled-in
catalogue is a guess and does not count; a deliberate descent to a smaller
model and a model named by a pin or `X-IronWire-Route` still win. Escape
hatches: `ironwire pin`, the `X-IronWire-Route` header, and `ironwire/*` slugs
for clients that accept free-form model strings.

### Default policy

1. Serve at the requested tier from the highest-fidelity backend that is
   authenticated, in-quota and healthy.
2. Prefer marginal-cost-zero capacity (subscriptions) over metered — the
   client's own subscription, and with cross-account fallback on, the user's
   others after it.
3. Under quota pressure, descend the ladder — do not jump.
4. Never interrupt the client: a request that cannot be served at *any* rung
   returns a protocol-correct error the client already knows how to handle,
   with `retry-after` preserved.

---

## 4. Observed quota, never estimated

`QuotaSnapshot` is only ever built from provider-supplied values:

```rust
enum Headroom {
    /// Provider told us. Carries when.
    Observed { used_pct: f32, resets_at: Option<DateTime<Utc>>, observed_at: DateTime<Utc> },
    /// We are inside a 429's retry-after window.
    Exhausted { until: DateTime<Utc> },
    /// Backend authenticated but has told us nothing yet.
    Unknown,
}
```

`Unknown` is displayed as `unknown`. There is no fourth variant for a guess.
Metered backends (API keys) report *spend*, which we compute from observed
usage tokens × the price table (`ironclaw_common::llm_costs`), and label as an
estimate because it is one.

### 4a. What we *do* measure: our own traffic

There is a second question the quota rule does not answer, and users ask it
constantly: **how fast is this going, and will it last?** A rate-limit header
says where you are; it does not say whether the next hour is affordable.

`ironwire_usage` answers that from the trace ledger — the tokens IronWire
watched go past, cut into session windows, differentiated into a burn rate and
carried forward to the window's close. This is not an exception to §4, for one
structural reason: **it never claims to describe a provider's books.** It
describes the requests this machine made. `Headroom` gains no variant, nothing
computed there reaches routing or eligibility, and `capacity:` on the status
screen still says `unknown` for any pool the provider has not spoken about.

Every figure it produces carries a `Basis`:

| Basis            | Means                                     | Example |
| ---------------- | ----------------------------------------- | ------- |
| `Measured`       | summed from the ledger; happened          | `100.0k tokens · 12 exchanges` |
| `Projected`      | a measured rate × the time left           | `2.5M tokens by the time it closes` |
| `SelfCalibrated` | the user's own completed windows          | `your own p90 over 14 past sessions` |
| `Declared`       | a limit the user wrote in their config    | `the Max 5× limit you declared` |

There is deliberately no fifth variant meaning "a limit we assumed". Published
per-window token limits do not exist; the figures in circulation are
reverse-engineered. So IronWire ships that table (`usage::plan`) but never
consults it unless `usage.plan` is set — at which point the ceiling is the
user's claim about their own subscription, and the screen says so. Unset, the
comparison is against their own history, which needs no table. A backend with
neither yields no percentage and no bar rather than a plausible one.

The algorithms — five-hour windows rounded to the hour, gap blocks,
overlap-weighted hourly rate, and a p90 taken over sessions that look like they
hit a cap — are ported from
[`Maciek-roboblog/Claude-Code-Usage-Monitor`](https://github.com/Maciek-roboblog/Claude-Code-Usage-Monitor)
(MIT), which worked them out against real Claude Code transcripts. The
percentile deliberately matches Python's `statistics.quantiles` estimator so
the two tools agree on the same machine. One behaviour is not carried over:
there, a lone request reports its whole token count as a per-minute rate. One
request is a point, not an interval, so here it reports no rate at all.

---

## 5. Eligibility and identity

A subscription backend is eligible for an inbound request **only when the
request already carries that product's client identity.** Concretely: the
Claude-subscription backend serves requests whose first system block is Claude
Code's; the ChatGPT-subscription backend serves requests that arrive as Codex.

IronWire does not synthesize that identity for other clients. Aider pointed at
the Anthropic façade routes to an API key, NEAR AI, or a local model — not to
the Claude subscription. This is the difference between *routing the traffic a
subscription was sold for* and *dressing one client up as another*, and it is a
hard rule, not a default. See [`TRUST.md`](./TRUST.md).

### A product, and a login

A user may hold several logins of one product — Claude Code under two config
directories, Codex under two `CODEX_HOME`s — and each is a backend of its own
(§6). So the identity a subscription requires is a product *and* a login: the
Claude subscription in `~/.claude-work` serves the Claude Code logged in there,
and by default nothing else.

IronWire learns which login a client is from the URL it was pointed at, never
from the body. `ironwire connect claude --config-dir ~/.claude-work` writes
`…/anthropic/account/claude-work` into that directory's `settings.json`, so the
Claude Code that reads it names its login on every request; a request at the
plain façade is the default login's. Only while that login is in its default
place, though: the client still on the plain URL is the one that kept the
default directory, so a default moved with `config_dir` is reached at its own
bound URL (`/anthropic/account/claude-sub`) and the plain URL then has no login
of that product. The body cannot answer the question: Codex sends no account id
to a custom provider at all, and Claude Code's `metadata.user_id` is a
client-private field whose format has already changed once. The peek does not
read it.

By default a client is served by its own login only (`TRUST.md` I8). That keeps
every request self-consistent — Claude Code's body names its own account, and
goes out on that account's token — and keeps a subscription under pressure from
being quietly replaced by another account, which is the plainest reading of
what both providers' rate-limit terms forbid. `[subscriptions]
cross_account_fallback = true` relaxes the login half and never the product
half: Aider still does not reach a Claude subscription, and Claude Code still
does not reach a ChatGPT one.

---

## 6. Component map

```
crates/
  ironwire_core       types, config, capability gate, policy, quota
  ironwire_creds      credential discovery + consent
  ironwire_ledger     the local trace ledger
  ironwire_usage      session windows, burn rate, projections (§4a)
  ironwire_translate  cross-family translation (the fallback lane)
  ironwire_upstream   Backend trait; native passthrough clients; observation
  ironwire_proxy      axum façades, router wiring, control API, trace sink
src/
  the `ironwire` binary — the workspace root package, so `cargo run` and
  `cargo install --path .` work without knowing the layout
```

Dependency direction is strictly downward; `core` depends on nothing of ours.

### Where backends come from

Discovery is the product: a fresh install with a Claude Code login has working
backends and an empty `config.toml`, and `init --write` generates no
`[[backends]]` section on purpose. Configuration adjusts that; it does not
replace it.

```
for each [[backends]] entry:
    enabled = false          → never registered, and not shown in status;
                               a disabled login's files are not even read
    id is one discovery uses (claude-sub, anthropic-key, codex-sub,
                              openai-key, nearai)
                             → discovery builds it; the entry overrides
                               base_url, api_key_env, config_dir (which
                               moves a default login) and, for nearai,
                               models. Its kind must be the one discovery
                               builds under that id
    any other id             → built from `kind`; a subscription kind is an
                               additional login, read from its config_dir
then: every backend discovery can produce that no entry named
```

Config-declared backends are appended after the discovered ones, in the order
the file lists them. Registration order is the tie-break in `Policy::select`, so
putting them first would silently change which backend wins a tie for everyone
who already had a config. Among logins of one product it is also the order
cross-account fallback tries them in, after the client's own.

Whether a backend appears without a credential depends on who asked for it.
Discovery registers what it finds — someone who never ran Claude Code is not
shown a Claude subscription — with one exception: NEAR AI is registered with or
without a key, because it is the destination `privacy.mode = "full"` names by
default and a trusted destination the user cannot see is one they cannot act
on. A login the user declared by directory is registered whether or not anyone
has logged in there yet: they named it, and `status` saying "not logged in —
run `CLAUDE_CONFIG_DIR=~/.claude-work claude`" is what they need, where silence
reads as though IronWire never heard of it. An unauthenticated backend is
unhealthy to the router, so nothing is sent to it. `enabled = false` is a
different statement, and does omit it.

`kind` must be one of `claude-subscription`, `anthropic-api`,
`codex-subscription`, `openai-api`, `nearai`, `openai-compatible`, `local`.
These fail at load, naming the entry and what to change — before the port is
bound, because a backend that silently never appears is the worst way to learn
about a typo:

- an unknown kind, or a duplicate id;
- an `openai-compatible` entry without both `base_url` and `api_key_env`;
- a `config_dir` on anything but a subscription, or one that is neither
  absolute nor `~/…` — the daemon's working directory is not the user's shell's;
- two logins of one kind whose directories are the same path, which would show
  one pool as two;
- an additional login whose directory is the product's default one (`~/.claude`,
  `~/.codex`): that is the default login moved there, so the message says to put
  `config_dir` on `claude-sub` or `codex-sub` instead.

Two shapes that main accepted still load, with a warning, and configure
nothing: one of discovery's ids under a kind discovery does not build for it,
and an additional subscription login without a `config_dir`. The registry
skips both rather than building a second copy of the default login. The refusal
for a mismatched kind under a discovered id that carries `config_dir` stays
(main's `BackendConfig` had no such field), and so do the `openai-compatible`
and `local` `base_url` checks under a discovered id. See
`a_host_still_starts_on_config_shapes_main_accepted`.

Two backends under one id are refused again at registration
(`BackendRegistry::try_push`), so nothing that slipped past validation can
shadow another: consent, quota, pins and ledger rows are keyed by id, and every
lookup takes the first match.

### More than one login of one product

`claude-sub` and `codex-sub` are the default logins, and keep those ids
forever: consent, persisted quota, pins, `privacy.trusted_backends`,
`[[limits.backends]]` and ledger history are all keyed on them. Every other
login is a `[[backends]]` entry with an id of its own and the directory its
client keeps the login in:

```toml
[[backends]]
id = "claude-work"
kind = "claude-subscription"
config_dir = "~/.claude-work"     # what you export as CLAUDE_CONFIG_DIR for it

[[backends]]
id = "codex-work"
kind = "codex-subscription"
config_dir = "~/.codex-work"      # what you export as CODEX_HOME for it
```

`ironwire connect claude|codex --config-dir DIR [--id ID]` writes that entry
(the id defaults to the directory's name: `.claude-work` → `claude-work`) and
points the client that uses `DIR` at its login-bound URL:

```
http://127.0.0.1:8463/anthropic/account/claude-work       ANTHROPIC_BASE_URL in DIR/settings.json
http://127.0.0.1:8463/openai/account/codex-work/v1        base_url in DIR/config.toml
```

The URL is how a client says which login it is (§5). The account segment is
IronWire's and is never forwarded, an id that names no login of that product is
refused rather than served as the plain façade, and the bound model lists offer
only what that login's client could be served (`PROTOCOL.md` §1). `config_dir`
is kept exactly as typed, because Claude Code names a login's Keychain item
after the exact directory string (`TRUST.md` §2). Logins are declared, never
discovered by scanning — that is a destination the user did not choose. A
`--config-dir` naming a directory an entry already declares reuses that entry
as declared, spelling included, so the CLI checks, and asks consent for, the
same Keychain item the daemon will read
(`a_codex_home_in_the_shell_naming_a_declared_login_means_that_login`). The
exception is Claude Code on macOS, whose Keychain item is named after the exact
directory string: a spelling that reaches the same directory on disk (a
trailing slash, a symlink, a case variant) but names a different Keychain item
is refused, not reused, because both would write one `settings.json` and one
client would be served the other's token
(`a_shell_using_an_alias_of_a_declared_claude_directory_is_refused`).

Moving the default is the same command with `--id claude-sub` (or
`codex-sub`), and a `--config-dir` that names the default directory itself
means exactly that, never a second login. A moved default is wired with its
bound URL, `…/anthropic/account/claude-sub`, like any other login: the plain URL
belongs to the client that kept the default directory, which the moved login's
token does not serve. So `connect` warns when that client's file still points
at the plain façade — it will now be served by no subscription — and names the
command that takes it off
(`moving_the_default_login_warns_about_the_client_still_pointed_at_the_plain_url`),
and `doctor` flags the same state
(`a_client_left_on_ironwire_by_a_moved_default_login_is_flagged`).

Any move leaves the client file it was wired in still holding the URL that
names the login, and the daemon resolves that URL by id alone — so whatever
directory the id reads now would serve that client: a default moved back by
taking `config_dir` off it, or an additional login given another directory.
The consent ledger records the directory each consent answered, which is the
one record of where a login was. `connect` and `doctor` read it and warn about
a client there still bound to the login, naming the command that takes it off,
and leave the file alone
(`moving_a_login_back_warns_about_the_client_still_bound_to_it_in_the_old_directory`,
`doctor_flags_a_client_still_bound_to_a_login_that_moved_away`). A consent
given in the new place replaces that record, which is why `connect` says this
before it asks.

Writing a bound URL replaces a plain URL of ours in that file, and never a URL
that is not ours. The reverse never happens: a connect that names no login —
plain `ironwire connect claude`, `init`, the menu bar app — keeps a file's
existing binding and only follows the daemon to a new port. Otherwise
re-running any of them on a work directory would quietly move that client
onto the default login's subscription
(`a_connect_that_names_no_login_does_not_unbind_one`, in both
`claude_settings` and `codex_config`). The menu bar app's connect is the
daemon's, and the daemon never follows its own `CLAUDE_CONFIG_DIR`: it writes
the default login's `~/.claude/settings.json`, never the directory that variable
names
(`a_control_api_connect_never_writes_the_plain_url_into_another_logins_directory`).
Nor does it write the plain URL into a directory config has moved a login to —
`claude-sub` relocated to `~/.claude` itself, or `codex-sub` to the daemon's
`CODEX_HOME`. That login is served only at its bound URL, so the connect is
refused with reason `login-directory` and the `connect --config-dir` command
that binds it
(`a_control_api_connect_never_writes_the_plain_url_where_a_moved_login_reads`).

### The daemon and its control API

One daemon per machine, lockfile at `$IRONWIRE_HOME/daemon.lock`, listening on
`127.0.0.1:8463`. Everything except the two façade prefixes lives under
`/_ironwire/`:

```
GET  /_ironwire/status          full state snapshot (JSON)
GET  /_ironwire/backends        the same handler as /status
GET  /_ironwire/settings        what can be changed, what is selectable, and
                                which coding agents are wired to us
GET  /_ironwire/log?limit=      recent exchanges from the local ledger,
                                also ?since= and ?after_id= to page
GET  /_ironwire/summary?since=  calls, cost and proof status per model and
                                backend, split routed (NEAR AI) vs outside
GET  /_ironwire/events          SSE: route decisions, quota changes, errors
GET  /_ironwire/health          liveness. The one route with no token
POST /_ironwire/pin             { backend, model } — no backend clears the pin
POST /_ironwire/privacy         { mode }
POST /_ironwire/consent         { backend, granted, prompt_version, binding }
                                `binding` is the opaque string the consent
                                prompt carried, echoed back. A grant without
                                it, or with one that no longer matches the
                                login the prompt names, is refused (400); a
                                withdrawal (`granted: false`) needs neither
                                `binding` nor a current `prompt_version`
POST /_ironwire/tools           { id, connect } — point a coding agent here,
                                also { dry_run } to be told the change without
                                making it, and { as_previewed } to make only
                                the edit a preview described. A refusal
                                carries `reason` beside `error`: unknown-tool,
                                no-path, unparseable, jsonc, unusable-entry,
                                login-directory
POST /_ironwire/probe           hit every backend for real, except a
                                subscription awaiting consent: that one is
                                reported unprobed, with `connect_command`
```

There is no shutdown route. The daemon stops on a signal, which is what a
service manager sends anyway; an HTTP endpoint that ends the process would be
one more thing the control token has to be trusted with.

`ironwire status` and the macOS menu bar app are both **clients of this API**.
No routing logic lives in the CLI or in Swift; otherwise they diverge.

The tool list on `/settings` is that rule applied to detection. `ironwire
connect` is what edits somebody's agent config, so IronWire is the only thing
that knows which agents are here and which are pointed at it; both the CLI and
the API read it from one place (`ironwire_agents::tools`). A client that
detected agents for itself would be a second answer to that question on the
same machine, and the wrong one.

That rule is why `/settings` reports which privacy modes are *selectable* rather
than only which exist. `full` routes solely to backends the user has named, so
with none named it would take every backend out of service — a rule that lives
in `Config::validate`. A client that worked it out for itself would be a second
implementation of it, in a language that cannot see the config.

`POST /tools` is the third, and the one that writes outside `$IRONWIRE_HOME`.
`ironwire_agents` splits working an edit out from making it because every path
that touches somebody's agent config shows the change first; the CLI prints the
plan and waits. `dry_run` is that half of the split reaching the API, so a GUI
can ask the question rather than report the answer. The plan itself never
leaves the daemon — it holds the whole file — so a preview hands back a
`digest` of the edit it worked out, and a caller sends that back as
`as_previewed` to be sure the edit that lands is the one it showed. The digest
covers the file, the state it was found in and the state it would leave,
because the caller picks the tool and the direction again on the second call: a
half-wired config yields a real connect *and* a real disconnect from identical
bytes, so a digest over the file alone would let a client that was shown the
additions commit the removal. Neither call lets the caller name a file or
supply contents: the request is a tool and a direction, and the daemon works
the rest out both times. The confirmed write re-reads the file immediately
before writing, so an edit that landed while the user was being asked refuses
rather than being overwritten; that is a check and then a write, and the window
it does not close is documented on `commit_if_unchanged`.

The two writes follow from the same principle. `POST /privacy` changes the
running daemon *and* `config.toml`, because a change that only applied at the
next restart is a switch that appears to do nothing, and one that only lived in
memory would quietly revert the user onto a weaker filter. `POST /consent`
carries the prompt version the user was actually shown and refuses anything
else: an answer belongs to the wording it answered (`docs/TRUST.md` §2). It is
recorded with the directory the backend's login is read from, for the same
reason — an answer belongs to the account it answered, and a moved login is
asked again.

---

## 7. Reuse of `nearai/ironclaw`

Reuse is deliberate and layered, because binary size is a distribution
constraint (`brew`/`npx`/`apt`/`pip` all want one small self-contained
executable) and `ironclaw_llm` pulls `rig-core`.

| What | From | Status |
|---|---|---|
| Codex credential discovery, `auth.json` format, ChatGPT-vs-key base URL split | `ironclaw_llm::auth::load_persisted_credentials` (`CredentialSource::CodexCli`) | **delegated** — `ironwire_creds::codex` is a thin wrapper. Two exceptions, both noted in that module: the `CODEX_HOME` override (ironclaw hardcodes `~/.codex`) and the `chatgpt_account_id` claim (ironclaw's extractor is private) |
| Price table | `ironclaw_common::llm_costs::price_usage` | **delegated** — `ironwire_ledger::price`. Closed a real gap: `spend_today_usd` was structurally always `None` before |
| Circuit-breaker vocabulary (`CircuitState`, `CircuitBreakerConfig`) | `ironclaw_llm::circuit_breaker` | **delegated** — types reused so both products report backend health in the same words. The transitions are ours (`ironwire_upstream::breaker`) because ironclaw's live on `LlmProvider`; see below |
| Claude Code credential reading | `ironclaw_llm::anthropic_oauth` (private module) | **ported** into `ironwire_creds::claude` — cannot be delegated until upstream exposes it. Upstream PR: add `CredentialSource::ClaudeCode` |
| Anthropic OAuth header/beta constants, refresh-on-401 shape | `ironclaw_llm::anthropic_oauth` | pattern reused, ~200 LOC |
| Codex `client_version` detection (`codex --version` → `/models?client_version=`) | `ironclaw_llm::codex_chatgpt` (private module) | **not yet done** — gates newer models, so a stale value silently hides models the account is entitled to. Tracked in `ROADMAP.md` |
| Error classification, `retry-after` parsing | `ironclaw_llm::error` | pattern reused in `UpstreamError` |
| Complexity scoring (tier hint only) | `ironclaw_llm::smart_routing` | not built; feature `complexity`, off by default |
| OpenAI Chat/Responses wire types, SSE framing, error mapping | `ironclaw_openai_compat` | not reused — see the note on the native lane below |
| **Whole trace contribution pipeline** — consent policy, deterministic redaction, on-disk queue with holds, manual-review gating, classification, credit/claims, device-key onboarding | `ironclaw_trace_commons` | git dep, feature `contribute`, off by default |

`ironclaw_trace_commons::capture::capture_conversation_trace(scope, messages,
task_failed)` is keyed on a scope string and a plain `ConversationMessage`
list — which is close to exactly what a proxy can synthesize from an observed
exchange. That crate is the entire NEAR-AI-credits half of the product and we
should not reinvent any of it.

**What we deliberately do not reuse:** `ironclaw_llm`'s `LlmProvider` trait and
`CompletionRequest`/`ToolCompletionRequest` types, and any wire-type crate that
implies them. They are a chat-completions-shaped abstraction — exactly the lossy
common denominator the native lane exists to avoid. A typed request struct
cannot express "unchanged", and "unchanged" is the entire fidelity claim
(`tests/passthrough.rs`, `tests/codex_on_subscription.rs`). They are appropriate
for an agent that owns its own prompts; they are not appropriate for a pipe.

That boundary is narrower than it first looks, and an earlier draft of this
document over-applied it. It rules out the *datapath*: the bytes on the wire,
the request and response types, the streaming decode. It says nothing about
credential discovery, price tables, or health vocabulary — all of which are
now delegated rather than reimplemented, per the table above.

---

## 8. Trace ledger

Local capture is **on** by default and stored in
`$IRONWIRE_HOME/ledger.sqlite`; upload is **off** by default.

Recorded per exchange: timestamp, conversation key, the client's own session id
when it sent one, façade, chosen backend + model + rung, requirements, token
usage, cost, TTFT and total latency, finish reason, retry/fallback events,
— behind `capture.logprobs = true` — the confidence aggregates below, and
— behind `capture.bodies = true` — the request and response bodies.

The bodies are stored **verbatim**, as bytes, and the row carries the SHA-256 of
each alongside the provider's own response id. That combination is what makes an
exchange checkable against the provider rather than only against ourselves: NEAR
AI signs `<request digest>:<response digest>` and serves it at `GET
/v1/signature/{id}`, keyed by the same id. So the bytes hashed are the ones the
*upstream* saw — after a model override, after the privacy filter, after
translation — and for a streamed response they are the raw concatenated event
stream, not anything reassembled. A body that was re-serialised on the way in,
or a response that was cancelled or restarted, has no honest digest and is
recorded as having none.

The bodies themselves are held on a **rolling window of one per session**:
recording an exchange's bodies releases the previous exchange's for that
session, so a finished session leaves exactly one — its final call. That is the
only call anything downstream attests, and keeping every turn would hold the
maximum possible amount of the user's content to buy something nothing uses. A
released row gives up `body_ref` and keeps both digests: a hash is not the
content, and a row still pointing at a file we had deleted would claim a body it
cannot produce. The window is scoped by the client's own session id, falling
back to the conversation key for a client that sends none — that over-merges two
concurrent sessions sharing a tool list, which deletes more than it should
rather than less. `retain_days` is unchanged and now bounds only the *age* of
those surviving final-call bodies; the window bounds their *number*.

The session id is worth separating from the conversation key, because they
answer different questions. The conversation key is a routing-affinity hash
of protocol, supplied session and client login. Without a supplied session it
falls back to protocol family, the preamble head, the tool list and client login;
that fallback cannot distinguish identical concurrent conversations. The session id is what the agent itself
calls the session: `x-claude-code-session-id` from Claude Code, `session-id`
from Codex, both already forwarded untouched. It is what makes a row in
`ironwire log` line up with the session a user is actually looking at. Recorded
only when it is a plain identifier, so nothing a client sends can reach a
terminal as an escape sequence.

The user-facing value comes first (`ironwire log`, `ironwire replay`, cost
attribution, "what did my agent actually send"). Contribution is a separate,
later, explicit decision that hands the same records to
`ironclaw_trace_commons`, which owns redaction and consent. See
[`TRUST.md`](./TRUST.md) §4.

Because IronWire sees consecutive calls in one conversation, the ledger
naturally captures the signal that matters: *model proposed X → tool returned an
error → model repaired it with Y → next call succeeded.* A later opt-in hooks
plugin can add `git diff`, test results and human acceptance to close the loop.

### `capture.receipts` — whether an answer carries proof

Every row carries a `proof` label. A row served by a backend that signs its
answers (NEAR AI) is recorded `pending`; anything else is `outside` and is never
checked. With `capture.receipts = true` a background task later fetches the
receipt for each `pending` row's `upstream_id` and settles it on exactly one of:

| label | meaning |
|---|---|
| `verified` | a `provider_tee` receipt over this row's digests, signed by a key a verified TDX quote binds for the model. The only label that is proof. |
| `gateway_only` | a valid receipt, but the gateway's: it names the relay, not the model. |
| `unattested` | a valid `provider_tee` receipt whose key nothing has tied to a verified quote. |
| `unavailable` | no receipt to be had (a brokered model 404s permanently), no digests to check one against, a provider-rewritten body, or retries exhausted. |
| `failed` | a receipt that does not check out. |

The task never runs on the response path -- it reads rows that are already
written -- and it can only move a row *out* of `pending`, so a verdict is never
overwritten. Retries and concurrency are bounded (`ironwire_proxy::proof`). A
404 is looked at once more, a round later, before it is believed: a brokered
model's 404 is permanent, but a hosted call checked moments after it finished
can be ahead of the provider writing its record, and one extra `GET` per
brokered call is cheaper than a wrong `unavailable`.

IronWire has no DCAP quote verifier of its own, so standalone, a receipt that
checks out settles `unattested`, never `verified`. An embedding host that
verifies quotes supplies the attestor (`docs/EMBEDDING.md`).

Receipts are checked against the digests of the upstream bytes. With
`capture.receipts` on those are taken as the bytes stream past whether or not
`capture.bodies` is on; with bodies off nothing is accumulated or written, only
hashed. The digest is SHA-256 over the request exactly as sent and the response
exactly as received (for a stream, the raw concatenated events), pinned against
a live NEAR AI capture in `crates/ironwire_proxy/tests/proof_status.rs`.

`GET /_ironwire/summary` rolls the same rows up per model and backend. There is
no kind-of-work breakdown: nothing classifies work, so `work_kind` is `null`.

### `capture.logprobs` — per-token confidence, cross-family only

`capture.logprobs = true` asks a Chat Completions backend for the
log-probability of each token it generates. `false`, the default, does not ask.
It needs `capture.enabled` as well: with no ledger there is nothing to write the
result into, and asking would be per-turn cost with no reader.

The point is a signal the transcript does not carry: *where the model was
uncertain*. A trace's value is not that it is unlike the others; it is that a
human's judgement resolved something the model was unsure about, and entropy at
a decision point is the only thing here that localises that.

**One wire, and that is a fact about the wires rather than a limitation of the
implementation.** Chat Completions has `logprobs: true`. Anthropic Messages has
no such parameter at all and answers an unknown field with a 400 — on every
request, not on some of them. Responses spells it as `top_logprobs` plus an
`include` entry, and has no boolean form. So the pivot IR carries the intent
(`Params::logprobs`) and exactly one emitter honours it; the other two drop it,
which is the one place a silent drop is correct rather than a bug.

**`logprobs`, never `top_logprobs`.** The alternatives are tokens the model
considered and the user never saw. Nothing reads them: the confidence
reduction below is defined over the chosen token. Asking for them would be bandwidth on every
frame and — because `capture.bodies = true` records response bodies verbatim —
a way to persist continuations that were never generated into text. The two
flags together would compose into exactly that, so the alternatives are not
requested at all rather than requested and dropped.

**Cross-family only.** Rule 1 says the native lane forwards bytes;
`docs/PROTOCOL.md` §2 enumerates the mutations and
`crates/ironwire_proxy/tests/passthrough.rs` pins them. The setting is applied
in one place — `pipeline::translate_request`, on the path that already builds a
fresh body — so a request that is not translated is never modified to carry it,
at any setting. A client that asked for log-probabilities itself keeps them
either way: the setting ORs into the parsed request rather than replacing it.

Three reasons it is off by default, any one sufficient:

- It changes what the provider is asked to produce, so a captured exchange is
  **not comparable** to an uncaptured one. The same non-comparability the
  privacy filter records per exchange ([`PRIVACY.md`](./PRIVACY.md) §3).
- It inflates every response materially — on an agent loop that is per-turn
  bandwidth and storage, not a one-off.
- The distributions are conditioned on the whole context, which makes them more
  sensitive than the text they describe.

That last point is where this feature and the privacy filter interact, and the
interaction is favourable. Log-probabilities describe whatever the provider
actually saw. Because substitution happens on the way **out**, before the
request is sent, an upstream running under the filter generates conditioned on
placeholders — so the numbers describe the substituted text, and there was
never an unsubstituted generation to leak. Redaction applied *after* generation
could not make that claim: it cannot scrub numbers produced before it ran. It
is still bounded by the filter's false-negative rate, which
[`PRIVACY.md`](./PRIVACY.md) §7 is explicit cannot be measured on real user
data — so this reduces the exposure rather than removing it, and no interface
may say otherwise.

### Confidence aggregates

The raw distributions never leave the machine and are never written to the
ledger. `ironwire_core::confidence` reduces them, at the end of the exchange, to
four numbers — mean p(chosen token), its population standard deviation across
tokens, a coarse bucket, and the token count the mean is over — and those are
what the ledger records, in `mean_confidence`, `confidence_variability`,
`confidence_bucket` and `confidence_tokens` — four flat, queryable columns,
added through the same additive migration as every other late column, so an
older ledger file keeps working and an older IronWire keeps reading it. Four aggregates give an attacker
essentially nothing to invert; a per-token distribution gives them a great deal.

The vocabulary is borrowed from Swayamdipta et al., *Dataset Cartography*
(2020), and the borrowing is inexact in a way the module documents rather than
papers over: cartography measures p(gold) across training epochs, and
single-pass generation has neither epochs nor a gold label. `variability` here
is dispersion across *tokens*, which differs in kind rather than degree. The
names match the Trace Commons envelope fields they feed, which is the only
reason to keep them.

Both paths are covered. Streaming accumulates in the pivot stream parser as
frames arrive; a non-streaming answer is read out of the buffered body in
`pipeline::translated_body`. The original version of this feature captured only
on the streaming path, which meant a `stream: false` request paid for the
inflated response and recorded nothing — a cost with no signal is the worst of
both.

---

## 9. Failure semantics

| Situation | Behavior |
|---|---|
| Upstream 429 before first byte | try next rung; if none, return the provider's own 429 with `retry-after` intact |
| Upstream 429 after first byte | terminate the SSE with a protocol-correct `error` event. **No transparent retry** — replaying would corrupt the stream |
| Upstream 5xx before first byte | retry same backend (bounded, jittered), then descend |
| Upstream 4xx | the request itself is wrong, so no other backend is tried; the provider's own status and its own explanation are returned to the client and written to the ledger's `error` column |
| Client disconnects | **abort the upstream request** — abandoned requests must not burn quota |
| Credential expired | never refreshed — the store belongs to the client that wrote it, and refreshing rotates the token out from under that client. A subscription token is re-read on every request, one past its stated expiry is not sent, and a Claude token rejected with a 401 is re-read once in case Claude Code refreshed it meanwhile. Then the backend is `NeedsAuth`, the conversation descends, and `status` names the command that logs that login in again |
| No eligible backend | protocol-correct error the client already handles; never a made-up success |
| Daemon not running | CLI says so and offers `ironwire serve`; never silently proxies to the real provider |

---

## 10. Install surface

Target: `brew`, `npx`, `apt`, `pip`, plus a one-line shell installer. All five
ship the same prebuilt binary; see [`PACKAGING.md`](./PACKAGING.md).

Connecting a client is a command, not a docs page:

```bash
ironwire connect claude   # writes ANTHROPIC_BASE_URL into the user's shell profile / settings.json
ironwire connect codex    # writes the model_provider block into ~/.codex/config.toml
ironwire connect near     # device-key enrollment
ironwire connect openai-api / anthropic-api
ironwire status
ironwire doctor           # verifies each connection end to end with a real 1-token call
```

Every `connect` is reversible (`ironwire disconnect <client>`) and prints the
exact file it is about to modify before modifying it.

---

## 11. Non-goals for v1

- Hosted/multi-user mode. Not "later" — see TRUST.md.
- Owning conversation state, compaction, or tool execution.
- Being a general OpenRouter competitor (arbitrary third-party providers). BYO
  OpenAI-compatible endpoints work, but the product is about *the user's own*
  capacity.
- Training-data collection as a load-bearing business assumption.
