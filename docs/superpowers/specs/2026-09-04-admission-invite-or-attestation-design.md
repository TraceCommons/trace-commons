# Admission: an invite or attested inference

**Status:** design in progress. One decision taken, the hard problems named,
NOT ready for an implementation plan.

Today admission to Trace Commons is by invite. The proposal is that the
server accept **either an invite or attested inference**, plus a bounded
onboarding window in which a new user may submit unattested traces as a way
of earning inference.

Nothing here is built.

## Why the window exists

Attested inference cannot admit a *new* user on its own. Producing a receipt
requires already having used NEAR AI, so admission-by-attestation is closed
to exactly the people who most need a way in. The onboarding window breaks
that cycle:

```
NEP-413 signature -> NEAR account id
   |
free window: N unattested traces
   |
credit earned -> NEAR AI inference
   |
receipts -> attested traces
   |
admission no longer needs the window
```

It also points the credit model at its own bootstrap: the first thing credit
buys is the inference that makes a contributor self-sustaining.

## Decision taken: the identity anchor is a NEAR account

A free submission window is only meaningful if minting accounts costs
something; otherwise it is N free traces *per attacker*, repeatable. The
invite currently IS that anchor, so removing it for a subset means replacing
it.

**NEP-413 wallet login**, which already exists in this codebase. Account
creation has a real cost; sybil resistance is whatever that cost is. It is
also the identity the credit model settles to, so the window, the credit
earned, and the inference bought with it all hang off one anchor rather than
three.

Rejected: device key plus rate limiting (device keys are free to mint, so
the limit does all the work); attestation-only with no window (no bootstrap
— users must already be NEAR AI customers, a much narrower funnel).

## The problem that must be solved first

**Attested inference is a transferable credential.**

A receipt proves inference happened on NEAR AI hardware over specific bytes.
It does **not** prove the submitter did it. A receipt and its bodies can be
pasted into a trace someone else wrote, and **receipt replay is deduped
nowhere** — the witness holds no state by design, and ingest has no dedup.

That is tolerable while attestation is a quality signal. The moment it
becomes an admission credential it is a way in, and receipts become worth
acquiring. So two things stop being follow-up work and become preconditions:

1. **Receipt dedup**, at ingest — the only component that holds state.
2. **Binding a receipt to the submitting account.** Without it, admission by
   attestation is admission by anyone holding a receipt. The witness's own
   docs already note that closing the paste-in gap needs a capture-side
   identity nonce inside the request body, which nothing sends today.

Until both exist, "invite OR attested inference" is "invite OR a credential
that can be bought second-hand".

## Open questions

- **What bounds the window?** A trace count, a time box, a credit ceiling,
  or a combination. A count is simplest; a credit ceiling ties naturally to
  what the window is *for*.
- **What stops re-onboarding?** One window per NEAR account is only as good
  as the cost of a NEAR account. Is that cost enough, and is it stable?
- **Do window traces earn full credit?** They are unattested and therefore
  the weakest evidence in the corpus. Full credit makes the window the
  cheapest way to earn; no credit makes it useless for its stated purpose.
- **What happens at the boundary?** A user whose window expires with no
  attested traces and no invite is locked out mid-flow. Does the window
  extend, degrade, or refuse?
- **Does an invite still grant more?** If an invited user and a windowed
  user are treated identically after onboarding, the invite is only a
  bypass. If not, the difference needs stating.
- **Quality floor.** The existing gates (novelty, perplexity, dedup, caps)
  apply to windowed traces too. Does a windowed submission that fails them
  consume window budget?

## Not in this design

The credit model itself, and what credit redeems for. That exists
separately and is not changed by this.

## Related

`2026-09-04-attested-inference-release-design.md` — the system this admits
against, and the limits it currently has.
