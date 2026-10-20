# IronWire

**One local inference endpoint for all your AI capacity.**

You are probably paying for Claude Max, ChatGPT Pro, Copilot and a couple of API
accounts — and every one of them is an isolated pool that runs out
independently, in the middle of your work.

IronWire is a loopback proxy that sits at the inference boundary. Point Claude
Code, Codex, or any AI tool at `127.0.0.1`, and IronWire keeps it on the best
available model and degrades gracefully when limits hit.

> **Your coding agent never dies at the rate limit.**

It also stops dying at *"API Error: Response stalled mid-stream."* IronWire
keeps a thinking upstream alive with heartbeats, restarts a stream that failed
before producing any text, rides out a 529 instead of moving you to a metered
key — and when a response really does die, ends it with a stated error rather
than a dropped connection you have to guess about.

```
Claude Code ──┐
Codex ────────┤
Cline ────────┼──► IronWire (127.0.0.1:8463) ──► Claude sub · ChatGPT sub
Aider ────────┤                                   API keys · NEAR AI · local
custom ───────┘
```

Your agent keeps its own loop, context, compaction and tool execution. IronWire
only decides where each conversation's inference goes.

---

## Status

**Both native lanes now run against live subscriptions.**
`scripts/acceptance.sh` completes a real coding task through IronWire on a
Claude Max account and on a ChatGPT Pro account — a fresh crate, a failing test,
the agent fixing it — and both lanes pass.

Working, with tests that pin each claim: both native lanes (Claude Code →
Anthropic, Codex → ChatGPT/OpenAI), the translated lane to NEAR AI, byte-identical
passthrough, observed quota, consent-gated subscription access, per-backend
circuit breaking, stream resilience, compaction-aware routing, a local trace
ledger with real costs, spend caps, and an optional privacy filter.

**What the first live run found is the argument for doing it.** Every one of
these passed the whole mock suite: both products had rotted out of their own
identity checks (Codex 0.145 stopped sending `instructions`; Claude Code 2.1.226
moved its system block and reworded it), so each was refused the subscription it
owns. The URL for both real providers was built wrongly — a mock mounted at the
server root is the one shape where the bug is invisible. A model the catalogue
had never heard of was silently replaced with an older one. Anthropic's
rate-limit headers were read under names it does not send, so capacity showed
`unknown` forever.

**What is still unproven, and it is the interesting half.** No provider has
actually rate-limited us, so descent under real exhaustion, the cross-family
fallback to NEAR AI, and the promotion back up the ladder are all still
mock-only. `docs/ROADMAP.md` lists the rest — a signing key, hosting, a Mac —
and none of them are "not done yet".

## Install

```bash
curl -fsSL https://ironwire.dev/install.sh | sh   # any Unix
brew install nearai/tap/ironwire                  # macOS, Linux
npx ironwire@latest                               # no install
pip install ironwire                              # the Aider crowd
```

Or from source: `cargo install --git https://github.com/nearai/ironwire`.

> The hosted channels above need a published release. Until one exists, build
> from source — `docs/ROADMAP.md` lists what each channel is waiting on.

## Quick start

```bash
ironwire init       # then run `claude`
```

That is the whole setup. `init` reads the room — a Claude Code login, a Codex
login, API keys in your environment, a model server on a local port — asks one
question, and does the rest:

- **Finds your capacity**, including keys your agents are already configured
  with, and says which of them IronWire will actually be able to see.
- **Asks once** whether it may use the subscriptions it found, stating the risk
  before it asks. Say no and everything else still works.
- **Points every agent it finds** at IronWire, in that agent's own config file —
  `~/.claude/settings.json`, `~/.codex/config.toml` — so the setup survives a
  new terminal. It never takes a setting you were already using; it says so and
  leaves it.
- **Leaves the daemon running** as a user service, so it comes back after a
  reboot. No supervisor available (a container, a bare SSH session)? It says so
  and tells you to run `ironwire serve`.

`ironwire init --dry-run` shows every change without making one.

```bash
ironwire doctor     # confirm it end to end, with a real request per backend
```

`doctor` checks the *clients*, not just the backends. Every backend can be
healthy while your agent still goes straight to the provider because nothing
points it here — the commonest way this looks broken when it is not.

Then:

```bash
ironwire status     # capacity, as the providers reported it
ironwire log        # what your agents sent, and what it cost
ironwire watch      # live routing; silent unless something changes
ironwire doctor     # probe every backend you have enabled, for real
```

```
IronWire 0.1.0 — http://127.0.0.1:8463

Claude subscription
  connected · subscription
  capacity: [████████░░] 82% used · resets in 2h14m · observed 40s ago
  models: claude-opus-4-6, claude-sonnet-4-6, claude-haiku-4-5

Anthropic API
  connected · api key
  capacity: unknown (the provider has not reported yet)

1 conversation(s) with a sticky route
```

Every number there came from the provider. Where nothing was reported, IronWire
says `unknown` rather than showing a plausible guess — see
[`docs/CRITIQUE.md`](docs/CRITIQUE.md) §4 for why that matters.

## How it decides

Routing is **per conversation, not per request.** A coding session carries a
large warm prompt cache and often provider-private reasoning state; moving it
costs money and latency, and moving it across API families can cost correctness.
So fallback is a state transition taken under quota pressure — at once for a new
conversation, and for one already under way only once the pressure has lasted
twenty seconds — down an explicit ladder:

| Rung | Route | Cache | Reasoning | Told? |
|---|---|---|---|---|
| 0 | same account, same model | warm | intact | — |
| 1 | same account, smaller model | warm-ish | intact | — |
| 2 | same wire format, different credential | cold | intact¹ | — |
| 3 | different API family | cold | dropped | **yes** |

¹ Except between two of your own subscriptions (opt-in, below). ChatGPT's
encrypted reasoning is bound to the account that minted it and is rejected on
any other, so IronWire keeps such a Codex conversation on its account; whether
Anthropic accepts thinking minted on another account is undocumented and not yet
verified.

Rungs 0–2 need no translation at all — IronWire forwards your request's bytes
untouched. Rung 3 does: a Claude Code session can keep working on NEAR AI (or
any OpenAI-compatible endpoint) when your Anthropic capacity runs out.

**The ladder goes both ways.** A conversation climbs back once a better rung has
been available for five continuous minutes — against twenty seconds to descend.
The asymmetry is the point: falling is urgent because the alternative is a
failed turn, while climbing is not, and a provider hovering at its limit would
otherwise move your session, and throw away its warm cache, every twenty
seconds.

The rule that makes rung 3 safe is **switch families at a turn boundary, never
mid tool loop** — a conversation caught mid-loop waits one turn rather than
being refused. Everything that would genuinely *break* the request (tools a
backend does not have, images it cannot see, a context that does not fit) is
still refused outright rather than silently degraded. See
[`docs/PROTOCOL.md`](docs/PROTOCOL.md) §6.

## More than one subscription

Claude Code keeps a login per config directory — Anthropic's own docs suggest
`CLAUDE_CONFIG_DIR` for keeping a work and a personal account apart — and Codex
keeps one per `CODEX_HOME`. IronWire treats each login as a backend of its own,
with its own consent, capacity and ledger rows:

```bash
ironwire connect claude --config-dir ~/.claude-work --subscription
ironwire connect codex  --config-dir ~/.codex-work  --subscription
```

That adds a `[[backends]]` entry naming the directory, points the Claude Code
(or Codex) that uses it at a URL of its own —
`http://127.0.0.1:8463/anthropic/account/claude-work` — and asks that login's
consent question. The daemon reads its logins when it starts, so restart it
once; then run the agent as you already do:
`CLAUDE_CONFIG_DIR=~/.claude-work claude`. In a shell where `CLAUDE_CONFIG_DIR`
is already set, plain `ironwire connect claude` means that login. `CODEX_HOME`
keeps meaning the default Codex login, as it always has, so a second Codex home
always takes `--config-dir`. `ironwire disconnect claude --subscription
--config-dir ~/.claude-work` (or `--id claude-work`) withdraws one login's
consent and leaves the others alone.

**Each agent is served by its own login.** When the work subscription comes
under quota pressure — 90% used, as the provider reports it — or runs out, the
work Claude Code descends the ladder as it always did — API key, NEAR AI,
local — and never lands on your personal subscription. Moving to another account after a rate limit is the plainest
reading of what both providers' terms forbid, and the provider can see it:
Claude Code's request names the account it is logged in as.

If you want it anyway, it is one line, off by default:

```toml
[subscriptions]
cross_account_fallback = true
```

It moves on the same trigger as every rung, and that pressure outranks the
preference for your own login: at 90% used as the provider reports it — not
only once a plan has run out — a new conversation goes to your other
subscription at once, and one already on yours after the twenty-second descent
debounce. Every request it moves is recorded at rung 2. Even then, a Codex conversation that carries
encrypted reasoning stays on the ChatGPT account that minted it, because OpenAI
rejects that reasoning on any other. [`docs/TRUST.md`](docs/TRUST.md) §2 has the
whole argument.

To move your default login rather than add one, connect with `--id claude-sub`
(or `codex-sub`). The moved login then has a URL of its own like any other, and
a Claude Code still on the plain URL is served by no subscription; `connect`
and `ironwire doctor` say so. On macOS, `~/.claude` itself is a special case:
Claude Code run with `CLAUDE_CONFIG_DIR=~/.claude` keeps a Keychain item of its
own but reads plain `claude`'s `settings.json`, so moving the default there
(`--config-dir ~/.claude`, or `connect` in a shell exporting it) has a plain
`claude` served by the moved login too; `connect` says so before writing.
Moving any login asks for its consent again: a yes answers the directory it was
given for.

## How fast it is going

`ironwire status` has two kinds of number on it, and it keeps them apart.

**What the provider said.** `capacity:` is a rate-limit header, or `unknown`.
There is no third option — IronWire will not guess a subscription's remaining
headroom, because one confidently wrong percentage costs you belief in every
other number on the screen.

**What IronWire measured.** The `Session` block is arithmetic on your own
traffic, out of the local trace ledger: a five-hour window, how fast you are
spending it, and where that lands by the time it closes.

```
Session (5h window) — measured from IronWire's own ledger, not reported by the provider
  claude-sub opened 1h ago · closes in 4h
    [█████░░░░░] 50% of your own p90 over 14 past session(s)
    used: 100.0k tokens · 12 exchange(s) · $2.00 at metered rates
    burn: 10.0k tokens/min · $12.00/hour · 1.7k/min last hour
    at this rate: 2.5M tokens by the time it closes · $50.00
    you reach your usual ceiling in 10m — 3h50m before the window closes
```

The ceiling is **your own history**, not a table: the ninetieth percentile of
your past windows, preferring the ones that look like they ran into a limit.
Per-window token limits are not published, so IronWire does not assert one. If
you know your plan you can say so — `plan = "max5"` under `[usage]` — and the
screen will label it as your claim rather than its measurement. With neither,
you get the burn rate and no percentage, which is the honest answer.

The window logic, the percentile and the burn-rate maths are ported from
[Claude-Code-Usage-Monitor](https://github.com/Maciek-roboblog/Claude-Code-Usage-Monitor)
(MIT), which worked them out against real Claude Code transcripts.

Output is coloured when it is going to a terminal, and never when it is not.
`--color always|never` overrides that, and `NO_COLOR` overrides everything.
Colour is only ever emphasis: every state it marks is also stated in words, so
`ironwire status | grep` and a monochrome terminal see the same screen.

## How it tells you

IronWire will not write into your agent's transcript. The only channel there is
the response stream, and a line in it would be a line the model appears to have
said. So it uses the places a harness keeps for something other than the model:

```bash
ironwire connect claude    # also offers to install the status line
ironwire statusline        # what it prints there, if you want to see it
```

In Claude Code that is the status bar, via the `statusLine` hook — which
IronWire fills only if you have not written your own, and removes on
`ironwire disconnect claude`. It stays quiet: where traffic is going, a
substituted model, a pool above a third used, a fallback for the ten minutes
after it happens, and a release when one exists.

A host that embeds IronWire can serve the same line from its own executable:
the rendering lives in `ironwire_proxy::statusline`, and the host installs its
own command in the slot under the same rules (`docs/EMBEDDING.md`).

Codex has no status-line hook (`tui.status_line` exists in its config and
renders nothing in 0.145), but it does render one line from the server on a
usage-limit response. That is where IronWire says whether it had anywhere else
to go — the moment you are stopped is the moment that matters.

`ironwire watch` is still there for a second terminal, and is still the only
place that shows every decision as it is made.

## The optional privacy filter

Off by default. When you turn it on, IronWire substitutes sensitive values on
the way out and restores them on the way back, so a provider never sees them
and you never see a placeholder.

```bash
ironwire privacy check src/config.rs   # what it would catch, before you rely on it
```

Two things it deliberately does not do. It does not claim to make anything
*safe* — it has a false-negative rate nobody can measure on your data, so
`privacy check` reports what it found and never says "clean"
([`docs/TRUST.md`](docs/TRUST.md) I7). And it does not substitute values inside
code blocks, tool results, or documentation ranges like `example.com` and
RFC 1918 addresses: in a coding session those are load-bearing *code*, and
replacing them makes the model write something that does not work.

[`docs/PRIVACY.md`](docs/PRIVACY.md) is the design and the critique of it,
including where the mechanism stops.

## What IronWire promises about your credentials

IronWire sits where all of your source code and all of your API credentials
pass. That position comes with hard commitments, not defaults
([`docs/TRUST.md`](docs/TRUST.md)):

- **Loopback only.** Binds `127.0.0.1`. No remote mode, no `--host`, no tunnel.
- **Credentials never leave this machine**, and are only ever attached to the
  host that issued them.
- **No hosted IronWire holds anyone's subscription token.** Not planned, not
  later.
- **No identity forgery.** IronWire will not present one product's client
  identity to unlock another product's subscription. That means Aider pointed at
  the Anthropic façade routes to an API key, not to your Claude Max.
- **Subscription backends are off until you say yes**, once, to a specific
  question that names the risk and whose account bears it.
- **One request, one login.** A request's token and every account header beside
  it belong to the same login, and with several Claude or ChatGPT logins each
  agent is served by its own. Falling over to another of your subscriptions is
  an opt-in you make in `config.toml`.
- **Traces stay local by default.** Upload is a separate, explicit decision.
- **IronWire never updates itself.** `ironwire update` tells you a release
  exists and prints your package manager's command. The only thing it refreshes
  on its own is a signed provider-catalog document — which, by construction,
  cannot name a host, so it can never redirect where a credential goes.

## Documentation

| | |
|---|---|
| [`docs/DESIGN.md`](docs/DESIGN.md) | The architecture |
| [`docs/CRITIQUE.md`](docs/CRITIQUE.md) | The design review that produced it |
| [`docs/PROTOCOL.md`](docs/PROTOCOL.md) | Wire fidelity: what is mutated, what is refused |
| [`docs/TRUST.md`](docs/TRUST.md) | Credentials, consent, traces |
| [`docs/PRIVACY.md`](docs/PRIVACY.md) | The optional privacy filter, and where it stops |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Milestones |
| [`docs/PACKAGING.md`](docs/PACKAGING.md) | brew / npx / apt / pip |
| [`docs/UPDATES.md`](docs/UPDATES.md) | notify-only updates, and the signed catalog channel |

## Layout

```
crates/
  ironwire_core       protocols, capability gate, routing policy, quota
  ironwire_creds      credential discovery + consent
  ironwire_ledger     the local trace ledger
  ironwire_usage      session windows, burn rate, projections (from the ledger)
  ironwire_catalog     the signed provider-catalog channel
  ironwire_update     notify-only update checking
  ironwire_privacy    reversible substitution (the optional filter)
  ironwire_translate  cross-family translation (the fallback lane)
  ironwire_upstream   backends: native passthrough and observation
  ironwire_proxy      axum façades, pipeline, control API
src/
  the `ironwire` binary — the workspace root package, so `cargo run` and
  `cargo install --path .` work from a fresh clone
```

## Development

```bash
cargo test              # unit + conformance
cargo clippy --all-targets
```

These suites carry the claims the design rests on:

| Suite | Proves |
|---|---|
| `tests/passthrough.rs` | request and response bytes are identical to the originals, modulo the mutations `docs/PROTOCOL.md` §2 enumerates |
| `tests/multi_turn.rs` | a three-turn tool loop — signed thinking, replayed tool ids, cache breakpoints — survives, and stays on one backend |
| `tests/cancellation.rs` | an abandoned request stops the upstream, and still records what it consumed |
| `tests/claude_code_on_nearai.rs` | a Claude Code session keeps working on NEAR AI when Anthropic capacity is exhausted — and waits for a turn boundary rather than switching mid tool loop |
| `tests/codex_on_subscription.rs` | Codex reaches ChatGPT byte-identical, and a non-Codex client is never merely refused but never dialled |
| `tests/stalled_stream.rs` | the "stalled mid-stream" failures: a thinking upstream is kept alive, a thinking-gap failure is restarted invisibly, a post-content failure is reported rather than replayed, a 529 is retried |
| `tests/circuit.rs` | a dead backend stops being dialled — but the last one standing is still tried, because a breaker should waste less time, not turn a degraded proxy into a dead one |
| `tests/privacy_filter.rs` | with the filter on, the provider never sees the value and the client never sees a placeholder |
| `tests/privacy_compaction.rs` | the same, across a compaction boundary on all three wires — where a mistake becomes permanent history |

Two scripts run the parts a unit test cannot see:

```bash
scripts/journey.sh          # the commands a person runs, in order, against a mock
scripts/test-install.sh     # install.sh, including its failure paths
scripts/test-packaging.sh   # the release scripts, before a tag depends on them
```

`scripts/journey.sh` is where every integration bug in this project has been
found — including a privacy-filter path that bypassed reversal on reconnect,
which every unit test was happy about.

`scripts/acceptance.sh` is the one check the mocks cannot replace: a real
Claude Code task, through IronWire, against real providers. It costs
subscription quota — run it before a release, not in CI.

## License

MIT OR Apache-2.0.

Scale workload, resource bounds and remaining live-account acceptance are documented in [docs/SCALE.md](docs/SCALE.md).
