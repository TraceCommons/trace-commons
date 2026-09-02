# What Trace Commons needs from IronWire

Five changes to `nearai/ironwire` that let the Trace Commons app be honest,
configuration-free, and — for one of them — ship a whole feature far earlier
than its current path allows.

Written after reading IronWire's README and control surface on `main`
(`70827f42`). Everything attributed to IronWire below was read there; anything
inferred is marked.

## Why ask rather than work around

Both sides are ours to change, so the question for each gap is only which side
is the honest place to fix it. Four of these are currently worked around in our
client with code that exists solely because IronWire does not say something it
knows. The fifth is not workable on our side at all.

## 1. A discovery file, so nothing has to be configured

**The problem.** Our daemon needs IronWire's port and control token. Today it
reads `$IRONWIRE_HOME/control.token`, falling back to `~/.ironwire`.
**Environment variables are not set for a GUI install** — an app launched from
Finder, the Dock or a desktop entry gets the session manager's environment, not
a shell profile's. So a contributor whose `IRONWIRE_HOME` points anywhere else
gets silence, and silence is indistinguishable from "not running".

The workaround we are building is a port field and a token-directory field in
our settings UI. Both exist only because the daemon cannot find IronWire on its
own, and both ask a person to know something the machine already knows.

**The ask.** Write a small pointer at a fixed path — `~/.ironwire/endpoint.json`
— **regardless of where `IRONWIRE_HOME` points**, containing the control base
URL and the absolute path of the token file. Mode 0600 alongside the token.

```json
{ "control_url": "http://127.0.0.1:8463", "token_path": "/custom/home/control.token" }
```

A fixed path under the user's home is reachable from a GUI launch, which is
exactly what an environment variable is not. It costs IronWire one file write at
startup and deletes two fields, a probe failure mode and a class of support
question from us.

This does not weaken the credential rule: the pointer names where the token is,
never what it is, and anything that can read the pointer can already read the
token beside it.

## 2. Say which session a row belongs to, in the id we key on

**The problem, and it is the one that can make everything else worthless.**
Our enrichment joins IronWire's ledger rows to sessions by `client_session_id`.
Our adapters key on `conversation_id` read from the agent's own transcript.
**Nobody has confirmed those are the same string in production.** Both sides of
our end-to-end test are fixtures we wrote, so the entire path can be correct and
join nothing, forever, with no error anywhere — and a person looking at our app
would see a correct privacy claim beside a permanent zero.

**The ask.** State in `docs/PROTOCOL.md` what `client_session_id` is derived
from for each façade, and — where the agent's own conversation id is visible on
the wire — emit *that*, rather than something IronWire computes. If the two
cannot be made equal for some façade, say so explicitly, because a documented
mismatch we can map is far better than an undocumented one we cannot detect.

**This is the highest-value item here.** It converts an unverifiable assumption
into either a guarantee or a known, mappable difference.

## 3. Report remaining NEAR AI balance

**The problem.** The app wants to tell someone how much inference they have
left. IronWire is the component actually talking to NEAR AI; we are not. Today
we would have to either ask NEAR AI ourselves — duplicating a credential
relationship IronWire already owns, which its own trust rules discourage — or
show nothing.

**The ask.** Where NEAR AI reports remaining balance, surface it on the control
API beside the capacity IronWire already reports per backend.

This fits IronWire's existing discipline rather than straining it: `status`
already distinguishes *what the provider said* from *what IronWire measured*,
and already prints `unknown` rather than guessing. A balance is squarely in the
first category. If NEAR AI does not report one, `unknown` is the right answer
and we will render it as such.

## 4. Keep the attestation, and hand it over

**This is the ask that changes a roadmap rather than a screen.**

NEAR AI runs inference inside an attested TEE and serves a nonce-bound TDX
quote. IronWire sits at the inference boundary where that is visible; nothing
downstream is better placed to capture it, and by the time a trace reaches us
the moment has passed.

**Why we want it.** Our app deliberately shows nothing about ownership or
contribution to someone who cannot act on it. What moves a person from "this is
a privacy tool" to "this work is mine and worth something" is a **local,
checkable fact**: sessions on their machine that carry proof they ran where we
say. Without that, the unlock waits on a witness service we have specified but
not built — a TEE deployment, its own attestation, its own operational burden.

If IronWire recorded, per NEAR AI exchange, the signing address and the quote it
verified, that unlock ships on a far shorter path.

**The ask.** When routing to NEAR AI, capture the attestation the request was
served under and store it on the ledger row; expose it through the control API
with the row.

Two things worth stating so the ask is not larger than intended. It is
**capture, not verification** — we verify quotes ourselves, in a crate built for
it. And a per-request quote may not be what NEAR AI offers; a per-session or
per-connection attestation, recorded once with the rows it covers, is nearly as
useful. **Verify what NEAR AI actually serves before designing the field.**

## 5. Tell us which tools you have connected

**The problem.** `ironwire init` already finds Claude Code, Codex and others,
and rewires each in its own config file. Our app wants to show the same list.
Reimplementing that discovery would mean two components disagreeing about
which tools exist on one machine — and ours would be the wrong one, because
IronWire is what actually changed the config.

**The ask.** Expose, on the control API, the tools IronWire knows about and
whether each is currently pointed at it.

Then our Tools screen reflects IronWire's truth instead of a second opinion, and
`ironwire connect` / `disconnect` remain the one place that changes it.

## What we are not asking for

- **Anything that uploads.** Traces staying local by default is IronWire's rule
  and it is a good one; every item here is read-only or a local file.
- **Anything about credentials.** No item asks IronWire to hand over a
  subscription token, and item 1 deliberately points at the token rather than
  carrying it.
- **Per-harness routing control.** Our design routes all tools or none, and
  IronWire's per-conversation ladder is a better mechanism than anything we
  would drive from a settings screen. If per-tool destination becomes a real
  requirement it should be raised on its own, not smuggled in here.

## Sequencing

**2 first**, because everything else can be right while it is wrong, and it is
the only item that can silently invalidate work already merged on our side.

**1 next**: it is the smallest, and it deletes UI we are otherwise about to
build — cheapest before that lands, not after.

**5**, then **3**: both improve screens rather than enabling them.

**4 last in effort, first in value.** It needs a fact nobody here has
established — what NEAR AI actually serves per request — and that fact should be
checked before anything is designed against it.
