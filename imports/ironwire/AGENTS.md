# Working rules for IronWire

Read [`docs/DESIGN.md`](docs/DESIGN.md) and [`docs/TRUST.md`](docs/TRUST.md)
before changing anything. This file is the short version of what those two
documents mean for code you are about to write.

## The four rules that are not negotiable

1. **The native lane forwards bytes.** If inbound protocol == backend protocol,
   do not deserialize-and-reserialize the body. The mutation set is enumerated
   in `docs/PROTOCOL.md` §2 and pinned by
   `crates/ironwire_proxy/tests/passthrough.rs`. Adding a mutation means
   updating both, in that order, with a reason.

2. **Never invent a number.** Quota, usage and rate-limit state come from the
   provider or they are `Unknown`. [`Headroom`](crates/ironwire_core/src/quota.rs)
   deliberately has no variant for a guess. If you find yourself wanting one,
   the answer is a better observation path, not a better estimate.

   The one thing that looks like an exception is `ironwire_usage`, and is not:
   it measures *IronWire's own traffic* out of the ledger and labels every
   figure with the basis it came from. It never claims to know a provider's
   remaining capacity, and nothing in it reaches routing (`docs/DESIGN.md` §4a).

3. **Refuse rather than degrade.** A route that cannot preserve the request's
   semantics is ineligible. `eligible()` in
   [`capability.rs`](crates/ironwire_core/src/capability.rs) is the only place
   that decision is made; keep it there and keep it total.

4. **No identity forgery, no credential drift.** A credential is only ever
   attached to its own `issuer_host`. IronWire never synthesizes another
   product's client identity. Both are enforced in code, not by convention.

## Layering

```
ironwire_core  ← no I/O, no deps on our other crates
     ↑
ironwire_creds ← credential discovery, consent
     ↑
ironwire_usage ← burn rate and projections, over ironwire_ledger's rows
     ↑
ironwire_upstream ← HTTP to providers
     ↑
ironwire_agents ← where a coding agent keeps its config, and how to edit it
     ↑
ironwire_proxy ← axum, façades, pipeline
     ↑
src/ (the `ironwire` binary, at the workspace root)
```

Dependencies point one way only. `ironwire_core` is where anything testable
without a network belongs — which is most of the interesting logic, on purpose.

## Reuse from `nearai/ironclaw`

`docs/DESIGN.md` §7 has the full table. The short version:

- **Do reuse** the auth readers, error classification, retry/circuit-breaker
  *semantics*, the price table, and — for M6 — `ironclaw_trace_commons`
  wholesale. It already owns redaction, consent policy, the on-disk queue,
  claims and device-key onboarding.
- **Do not reuse** `ironclaw_llm`'s `LlmProvider` trait or its
  `CompletionRequest` family. Those are a chat-completions-shaped common
  denominator — correct for an agent that owns its prompts, wrong for a pipe.
- Heavy dependencies (`rig-core` via `ironclaw_llm`) stay behind off-by-default
  features. Five ecosystems ship this binary, so keep it lean — but there is no
  hard size gate. CI reports the size; growing it is a decision you make in a
  review, not a threshold you trip (`docs/PACKAGING.md`).

## Tests

- Name tests after the behaviour they protect, not the function they call:
  `a_subscription_is_not_unlocked_for_a_client_that_is_not_its_own`, not
  `test_usable`.
- Every invariant in `docs/TRUST.md` has a test. If you add an invariant, add
  the test in the same change.
- Anything touching the wire needs a case in the conformance suite.

## Style

- No `unsafe` (forbidden at the workspace level).
- Comments explain *why*, and only where the reason is not evident from the
  code. Most of this codebase's non-obvious decisions are load-bearing tradeoffs
  — write those down; skip the rest.
- Errors a user can hit should say what to run next.
