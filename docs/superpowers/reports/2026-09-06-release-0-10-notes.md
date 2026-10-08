# Release notes draft — 0.10.0

The body below is the text to publish for **both** `app-v0.10.0` and
`contributor-v0.10.0`. The 0.9.0 pair carried the same body, and the two
release workflows append their own install paragraphs (`release-apps.yml`
"Publish", `release-contributor.yml`) on top of whatever notes the tag is
created with, so this text is the part that has to be written by hand and set
with `gh release edit`.

Everything below the rule is the notes text. Nothing above it is.

**State of this draft.** It is written against `origin/main` at `7abd0557`,
which is 76 commits past `app-v0.9.0`. The attested-inference sections were
rewritten after the four receipt-verification corrections landed (#676, #677,
#678, #679): 0.10.0 ships that path working rather than dormant, with the
limits stated below. There are no `TO CONFIRM` markers left; do not add one
back into publishable text.

---

## Contribute now waits for enrollment on macOS

On macOS, `Contribute` is now disabled until this device is enrolled.
Previously it armed as soon as a preview loaded, while the daemon refused the
press — the button was lying about what it would do. The disabled button now
explains why:

> This device isn't connected yet, so this preview was built without your
> identity and nothing here can be contributed.

This brings macOS into line with Windows and GTK, which already required
enrollment. An approval binds to the envelope a preview pinned, and a preview
built without an enrollment pinned nothing — so the button that approves it
should not have been pressable. Anyone using the macOS app unenrolled will
notice the change immediately.

The same change makes `Contribute` disarm, on macOS and on Windows both, when
the consent sentences cannot be decoded. The statement above the button is
the whole of what a contributor is told before pressing it; a build that
cannot render it must not offer a pressable button above a blank space where
the claim should be.

The sentences themselves now come from one place —
`crates/trace-commons-contributor/src/consent_copy.rs` — and cross to all
three shells already assembled, rather than being written out three times and
drifting.

## Answer model calls on this computer

The daemon can now run a local inference proxy: it answers the model calls
your tools make, from this machine, and keeps the record of them here. Each
call is still passed on to whoever you have set up to answer it. All three
shells offer it once on first start, and it stays in Settings either way.

It is **off by default**, and turning it on exposes something that has to be
said plainly. This is the shipped sentence, verbatim from
`crates/trace-commons-contributor/src/private_inference_copy.rs`
(`OFFER_EXPOSURE`), and every shell puts it on the offer and on the settings
card:

> While it is on, anything else running on this computer can send calls
> through it as well, charged to the accounts you have set up here. On a
> computer only you use that is your own software; on a shared one it is
> anyone who can log in.

The longer form, for anyone deciding on a shared machine: the proxy's control
API requires a token, but the `/openai` and `/anthropic` inference paths do
not. Any process on the machine that can reach loopback can send calls
through it, authenticated and billed with whatever upstream credentials the
proxy home is configured with. On a single-user computer that is your own
software; on a shared or multi-user machine it is anyone with a local shell.
Turning it on also starts the proxy's own background work, including
catalogue discovery, which makes network calls the daemon did not previously
make.

The internal setting is named `private_inference`. That name is not a claim,
and the shipped surface is forbidden from repeating it as one — a test sweeps
that file for claims of privacy, safety or encryption. Turning this on does
not make your calls private. It moves where they are answered from and keeps
the record here.

### Quick start

Two settings keys, in the order they take effect:

```
private_inference          # the daemon answers model calls on this computer
ironwire_attested_bodies   # a call's bodies go to the witness
```

Both are set over the daemon's settings channel:

```bash
trace-commons-contributor daemon settings --set private_inference=true
```

There is no dedicated subcommand. For the witness pins, the receipt endpoint,
the refusal labels and the rollback, see
[`docs/operator/attested-inference.md`](docs/operator/attested-inference.md).
For the full key list and the `private_inference_state` labels, see
[`docs/contributor-daemon-ipc-v1_1.md`](docs/contributor-daemon-ipc-v1_1.md).

## Inference receipts are verified against the key that is actually attested

0.9.0 shipped receipt verification that could not accept a real receipt. Four
changes in this release fixed it, and 0.10.0 ships that verification working
rather than dormant.

NEAR AI issues **two** legitimate kinds of receipt for the same hosted model,
and the request protocol decides which:

| the inference call | receipt `signature_kind` | signer |
|---|---|---|
| Chat Completions, `POST /v1/chat/completions` | `provider_tee` | the per-model ed25519 key in `model_attestations` |
| Responses API, `POST /v1/responses` | `gateway` | the single shared key in `gateway_attestation` |

Both are now verified, each against its own attested key, in the client and
at the witness. The receipt's own `signature_kind` selects exactly one key
source and never both; on a **pinning** witness an unrecognised kind, and the
absence of the field, name no source and are refused. A witness that pins
nothing — the default, and production today — is dormant on that check: it
still verifies a receipt's signature against the key the receipt itself
names, and refuses nothing for its kind.

Two mistakes are worth naming, because both looked like working controls:

- `signing_algo` is a query parameter of `GET /v1/attestation/report` and its
  **default is ECDSA**. A report fetched without `signing_algo=ed25519` is
  well formed and attests keys that sign nothing we verify. The client now
  sends it.
- A `model_attestations` entry carries **no `report_data` field**. The
  binding lives inside `intel_quote` at the TDX `report_data` position — byte
  offset 568, 64 bytes, `signing_address || nonce`. Code that required the
  JSON field refused every real model attestation. The receipt- and
  report-verification fixtures are now live captures of real reports and real
  receipts, so this class of mistake cannot recur against a fixture written to
  match the bug.

**What none of this establishes is the hardware.** The client reads NEAR AI's
attestation report, checks its self-description and the internal binding
(`report_data == signing_address || nonce`, out of the quote) — and stops
there. **The TDX quote's own signature is not verified anywhere in this
repository**, against Intel collateral or otherwise, and neither are the
quote's measurements. A key obtained this way is a claim by NEAR AI, not a
proof that an enclave made it. Tracked as issue #684.

The Codex CLI matters here specifically: it speaks the Responses API
exclusively, so all Codex-driven receipts are `gateway`-signed. A deployment
verifying only model keys refuses that traffic entirely.

### A gateway receipt attests the bytes, not the model

This is a first-class property of the path, not a footnote.

A `gateway` receipt's signed text is the two-part
`{requestHash}:{responseHash}`. There is no model in it, and the gateway key
is a single key shared across every hosted model, so it cannot distinguish
them either. The consequence, stated plainly: a contributor holding a genuine
gateway receipt for a cheap model can declare that exchange as **any other
model**, and both the client and a gateway-pinned witness will accept it. The
signature is real and the bytes are exactly the bytes; the model label beside
them is an unattested claim.

- A gateway-attested exchange proves the request and response **bytes**. It
  does **not** prove which model served them.
- Anything downstream that keys credit, scoring, pricing or eligibility off
  the **declared** model must not read a gateway receipt as evidence of that
  model.
- A deployment that needs the model attested has to require the
  chat-completions path, whose three-part receipt text
  (`{model}:{requestHash}:{responseHash}`) signs the model name — which means
  not accepting Codex traffic. That is a product trade-off and the witness
  cannot make it for anyone.

This is inherent to the receipt format and cannot be fixed here.

### What has been verified, and what has not

Verified live against NEAR AI, for **both** receipt kinds: the receipt is
fetched, and its signer checks out against the ed25519 attestation that binds
it.

The rest of the chain up to that point — the daemon-hosted proxy serving a
hosted model, the exchange correlated to its session with bodies captured, and
the receipt fetched for that exchange — was run on the chat-completions
(`provider_tee`) path only, against `Qwen/Qwen3.6-35B-A3B-FP8`. The gateway
receipt is an equally real capture — NEAR AI's own bytes, its real signature
checked against its real text — but it was taken at the receipt rather than
driven through the proxy, and its two-part text names no model, so there is no
model to attribute that capture to. The
proxy's own lifecycle (`running` / `running_elsewhere` / `off`, including
reclaiming the home from a standalone proxy) was confirmed live in the same
run.

**Not yet run:** the remaining legs — dry-run submit, the witness call, and
the stored-envelope body-absence check. Attested inference has therefore
**not** completed end to end, and this release does not claim it has. The run
record is
[`docs/superpowers/reports/2026-09-06-attested-inference-first-run.md`](docs/superpowers/reports/2026-09-06-attested-inference-first-run.md);
it records the run, the two obstacles found, and its own correction after the
report endpoint was re-queried with `signing_algo=ed25519`. The dormancy
warning in `docs/operator/attested-inference.md` stays until those legs run.

### Operators: what to change, and in what order

**`TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN` is retired.** It applied one key to
every receipt, including the `provider_tee` receipts it could never match, so
a witness with it set refused every real receipt under the same folded
`witness_inference_receipt_unverified` label as a forgery. A witness with it
still set now **refuses to start** and names the replacements, rather than
starting unpinned while an operator believes it pins. Re-reading the old value
under the new semantics would give it a meaning nobody chose, so the
replacements are new names deliberately:

- `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` — `model=key[,model=key...]`, keyed
  by the model in the receipt's own signed text.
- `TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS` — bare 32-byte ed25519
  keys, `key[,key...]`, no `0x` and no `model=` prefix.

Both are unset by default, and unset is dormant. A malformed value, the empty
string included, is a startup failure rather than a control that silently
matches nothing.

**On a witness that pins either kind, the kind it did not pin is refused.**
That is fail-closed and deliberate: pinning model keys is not agreement to
accept any gateway-signed receipt from any signer. A deployment that sees both
protocols needs both variables set.

**Enabling pins is a three-deployment rollout, and the order is
load-bearing** — `signature_kind` is a wire incompatibility in both
directions:

1. Deploy the new witness with **both** pin variables unset. This upgrades the
   code that can read `signature_kind` without enforcing anything with it.
   *This step is already done in production.*
2. Upgrade the contributor clients. Only a client at this version sends
   `signature_kind`, and the witness request body is `deny_unknown_fields`, so
   an older witness refuses a newer client's body outright.
3. Only then set the pins. A pinning witness refuses a receipt whose kind it
   cannot place, and an un-upgraded client sends no `signature_kind` at all —
   setting pins before every client is upgraded refuses those clients'
   submissions.

Steps 1 and 3 are separate witness deployments. In the committed compose the
pins live inside the measurement, so step 3 moves the measurement and needs
re-allowlisting everywhere it is pinned; plan it as its own change. The
derivation commands for both pins, including the `report_data` quote slice and
the nonce binding, are in
[`deploy/witness/README.md`](deploy/witness/README.md) and
[`docs/operator/attested-inference.md`](docs/operator/attested-inference.md).

Rolling back is in the same runbook, in the order that keeps the deployment
fail-closed at every step.

## The witness measurement is not printed here

The witness signing address and current measurement are published in
[`deploy/witness/README.md`](deploy/witness/README.md) under "The production
deployment". Read them from there, not from these notes: every witness
redeploy moves the measurement, and a release note is only a snapshot of what
was current when it was written. The witness was redeployed during this
release cycle, and the `app-v0.9.0` notes carry an `mrconfigid` that was
already stale when they published, which is why this convention exists.

The signing address is stable across an upgrade —
`0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798`, unchanged since 0.9.0 — and an
address that had changed would mean a new app rather than an upgraded one.

## Dependency pins

IronClaw is held at reviewed revision `6dccbfbc`; upstream has advanced since,
and re-advancing it is its own reviewed change rather than part of this
release. It is pinned in the two lockfiles, because upstream IronWire still
names `branch = main` for it, and both workspaces now build `--locked` in CI
and in the app release builds — a lockfile that is not committed in step with
a manifest is a failure rather than a silent re-resolve.

IronWire is pinned to revision `90c9ff94`, which confines the legacy
home-detection path: no-follow regular-file opens, a 1 KiB bound on the port
file, a loopback-only health probe with redirects and environment proxies
disabled, and `uid == euid` plus group/world-writable rejection on Unix. It
adds no package — 577 workspace packages before and after — and the embed API
this daemon binds is unchanged.

## CI

Branch protection on `main` now requires **13** status checks, including all
three shells — `macOS app tests`, `windows contributor app`, `windows
contributor crate tests`, `windows named-pipe ACL`, and `linux-shell desktop
integration (weston + portal)` — alongside `cargo check (permissive crates,
standalone)`, which is the only job that builds each MIT/Apache crate the way
a third-party harness gets it. CI also runs on merge-queue groups, and
committed dependency locks are required in root CI, GTK CI and the app release
builds.

## Everything else in this release

**macOS.** Waiting decisions are badged on the menu-bar mark (#605). Welcome
comes before folder consent, and onboarding has a Back step (#607).
Permissions and watched folders can be changed from Settings (#611), and the
notification-permission prompt now comes after onboarding rather than during
it (#612). The queue health banner opens the waiting-folder list (#633).
Review controls are easier to open, search and resize (#635). Odd-sized type
scales correctly (#636). Development notes are out of the credit record
(#638), and stale action messages can be dismissed (#639).

**Windows.** The window closing to the tray no longer stops watching (#621).
Invite launches are redirected to the running app (#624), and a packaged
cold-start invite activation is read correctly (#618). Recovery actions route
to connection and waiting sessions (#617), review can be restored for armed
projects (#613), Ignore can be undone during onboarding (#649), and daemon
error codes stay out of the status label (#629). "Look inside" is the
accented button on a review card and both Submit buttons are secondary, so
the screen no longer recommends sending without reading (#614).

**Linux/GTK.** Label and routing presentation helpers are shared rather than
duplicated (#637).

**CLI.** Destructive actions are confirmed, and an offline daemon is
identified as offline rather than reported as an error (#608).

**Wording.** The consent gate's sentences moved into the core and are shared
by all three shells (#668). The wording scanners now count what each shell
actually authors — Windows (#644), macOS (#669), GTK (#670) — and the
onboarding-window baseline was lowered to the scanner's real count (#659).

**Answering model calls on this computer.** IronWire runs inside the daemon
behind an off-by-default switch (#652). The offer is made once on first start
in all three shells (#656), and offers and switches stay tied to confirmed
settings writes (#662). Settings are persisted before the proxy is activated
(#657). The daemon retains ownership of the proxy through cancellation and
shuts it down within a bound (#665), state displays stay truthful during
uncertainty and shutdown (#660), metadata routing is derived from opted-in
owned inference (#666), and discovery health probes are confined (#667).
Upstream pins moved as described above (#658, #675).

**Attested inference and the witness.** The receipt key that is actually
attested is the one verified (#603), and the witness image that reads the
receipt scheme is pinned (#604). The witness gained a receipt signing-key pin
(#650), deployed (#674) and then unset once a live run showed it was correct
for no model (#677); that run is recorded (#676). Verification was rewritten
against the model's attested ed25519 key (#678) and extended to both receipt
kinds with separate pin variables (#679), and the witness that reads both
kinds was deployed pinning neither (#681) — step 1 of the rollout below. The
stale-container window is
recorded in the deploy README (#647), and the path from dormant to enforced is
documented (#642).

**Server and infrastructure.** The PostgreSQL-backed test suite was repaired
(#655), and a migration and its recording are now committed together (#651).
Worker IPC is verified against Orchard-generated signatures (#653). Native
onboarding connects to verified NEAR admission (#602), and HoloNear compute
lifecycle and package verification are integrated (#610).

**Build and CI.** Committed dependency locks are required in root and GTK CI
(#661) and in the app release builds (#663). CI runs on merge-queue groups
(#664). The contributor crates are at 0.10.0 (#673).

**Internal consolidation, no behaviour change.** Operator CLI plumbing
(#606), contributor source-root resolution (#616), prefixed protocol digests
(#622), source event shape constructors (#620), the chain-agnostic claim
plumbing (#625), ingest free-text validation (#628), migration execution
(#630), daemon test fixtures (#631), daemon IPC dispatch and entry lookup
(#632), and the FFI string panic tails (#634, #646). Submission test options
are defaulted without changing enrollment checks (#623), Antigravity
dead-code allowances are narrowed to fixture metadata (#626), and fixture
defaults and async-only dispatch refusals are guarded (#643).

**Designs and plans, no shipped behaviour.** The design and plan for
answering model calls locally (#609), corrected against the API that shipped
(#615); the ed25519
receipt verification plan, corrected against what was built (#641); and the
plan for moving the shells' words into the core (#648, #654).
