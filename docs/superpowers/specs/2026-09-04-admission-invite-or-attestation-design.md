# Admission: an invite or attested inference

**Status:** design in progress; admission changes are unimplemented. The user
has authorized a worktree, a subagent implementation plan, and work on native
onboarding. That scope authorizes the native foundation and this design work;
it does not settle the subsidy amounts, credit redemption terms, or broad
admission enablement proposed below.

Today admission to Trace Commons is by invite. The proposed alternative is
**an invite or qualifying attested inference for each submission**, plus a
bounded onboarding window in which an eligible new contributor may submit
existing unattested sessions. Existing authentication, tenant grants, privacy,
quality, deduplication, and usage caps continue to apply on every path.

The native experience and phased implementation are described in
[Native inference onboarding](2026-09-04-native-inference-onboarding-design.md)
and its [implementation plan](../plans/2026-09-04-native-inference-onboarding.md).
Neither native setup completion nor a local setting grants admission.

## Why the window exists

Producing a receipt requires already having used NEAR inference. The proposed
window lets someone with previous agent work contribute it before acquiring
inference access:

```
verified NEAR identity + provisioned Trace Commons account
   |
bounded window: contribute existing sessions
   |
accepted work -> pending credit -> settlement/redemption
   |
available inference funding + provider credentials
   |
useful NEAR inference -> account-bound receipt
   |
qualifying attested submission
```

Every arrow is a separate capability. Accepted work is not immediately
spendable inference, and wallet authentication does not provision a provider
API key. The redemption and credential-provisioning bridge is a release
prerequisite for promising this loop to users, even though its economic terms
belong to the credit design.

A contributor with neither previous sessions nor funded inference needs a
separate starting path: existing provider funding, an invite explicitly
including sponsorship, or a sponsored starter allowance. The latter two are
product proposals, not benefits an ordinary invite currently guarantees.
Do not display a funded allowance until a server has actually provisioned it.

## Identity: a NEAR account, with explicit account provisioning

The identity anchor remains a verified NEAR account. It is an identity anchor,
not proof of a unique human or a sufficient economic defense against repeated
onboarding. Account creation cost alone does not establish sustainable subsidy
limits; measure actual processing exposure and abuse during an invited pilot.

The current NEP-413 login resolves a previously enrolled identity to an
existing Trace Commons account. It does not create an account, tenant,
inference credential, or contributor-device authorization. Current enrollment
links a NEAR identity to an existing account. Self-service onboarding therefore
requires a distinct provisioning ceremony with verified wallet ownership,
account-to-tenant assignment, device authorization, recovery, and hash-only
audit. Do not turn an unknown-key login into implicit tenant creation.

One window belongs to the verified account anchor, not a device installation,
session cookie, or newly generated key. Key rotation, reinstall, logout,
relinking, and device enrollment must not reset its budget. Account unlinking,
merge, and deletion must have explicit lifecycle rules before opening the
window; none may silently restore an already used subsidy. Define retention
of the minimum abuse-prevention record alongside those rules.

## Separate identity, admission, funding, and activation

A future versioned server status contract should report these independently:

| Concern | States the native surface must distinguish |
|---|---|
| Identity | Disconnected; existing account linked; new account provisioned |
| Submission entitlement | Invite required; invited; window available; window exhausted; qualifying attestation required per submission |
| Inference | Not configured; funding required; available |
| Activation | Setup complete; useful inference observed; contribution processing; contribution accepted |

These are design states, not shipped API types. Missing, unknown, stale, or
unverifiable capability data must not enable an admission path. The client may
explain a state and resume setup, but only authenticated server state can grant
entitlement or report spendable funding. Keep pending contribution credit
separate from available inference balance.

An invite retains its explicitly configured privileges. A successful attested
submission does **not** permanently unlock future unattested submissions.
After a window is exhausted, each submission on the attestation route must
satisfy its evidence requirements, unless an independent invite entitlement
applies. All routes retain the existing quality floor.

## Prerequisite: account-bound, signed inference evidence

A receipt proves inference happened over specific bytes. It does not prove the
submitter made that call: a receipt and its bodies can be pasted into another
trace. The witness is stateless, and ingest currently has no inference-receipt
deduplication.

There is another prerequisite: the current redaction witness certificate v1
contains no signed inference-verification result. A permissive witness and a
requiring witness can share the same image measurement. Ingest cannot infer
successful receipt verification from that measurement, a client assertion,
or a valid v1 redaction certificate.

Before attestation may grant admission:

1. The server issues an unpredictable, account-bound challenge with a short,
   explicit validity period and an audience/domain identifying this purpose.
   Tenant and account context come from authentication. Store only the
   necessary hash-only binding and lifecycle metadata.
2. Capture carries that challenge inside the final upstream request bytes that
   the provider hashes. IronWire transformation, model replacement, privacy
   filtering, and cross-family translation must preserve the binding. A header
   or separate client claim outside the hashed body is insufficient.
3. The witness verifies the provider receipt over the captured raw request and
   response and extracts the binding from those verified request bytes.
4. A separate certificate profile, with a distinct signing domain and
   unambiguous canonical encoding, signs the inference verification result,
   canonical receipt identity, extracted binding, and redacted artifact digest
   together. Design the concrete profile across witness, contributor, and
   ingest before changing any encoder. Do not graft ambiguous optional fields
   onto v1 or accept v1 as an inference-admission credential.
5. Ingest verifies the profile and its configured trust policy, resolves the
   challenge to the authenticated account, checks expiry and consumption, and
   commits admission and receipt consumption atomically.

The canonical replay key should derive from verified provider signing identity
and request/response digests, with explicit domain separation. Do not key it
only by user-supplied receipt text or signature spelling: alternate encodings
must not turn one verified inference into multiple credentials. The dedup
boundary must prevent reuse across tenants while preserving tenant isolation;
a narrow hash-only uniqueness mechanism needs an explicit database design,
not an unrestricted cross-tenant query.

A qualifying certificate attests to the final declared inference call over the
bytes the provider hashed. It does not establish a complete conversation,
useful work, an uncompacted history, or a unique human. Existing quality gates
remain necessary.

## Proposed bounded window and reservation lifecycle

Use both a finite submission-attempt allowance and an aggregate processing
spend ceiling per account. Add a global pilot spend ceiling so many accounts
cannot collectively escape the intended exposure. Exact allowances, cost
units, reset policy, eligibility, and credit rates remain product decisions;
there is no default free allowance or automatic recurring reset in this spec.

The reservation lifecycle must be part of the server transaction design:

- Local discovery, local preview, and consent review consume no window budget.
- On an authorized submission, atomically reserve the attempt and a conservative
  processing-cost bound before starting chargeable work. Check the account and
  global ceilings together; missing cost controls refuse the subsidized path.
- Quality rejection consumes the attempt and charges processing actually used.
  Rejected work does not earn credit merely because it consumed an attempt.
- A transient failure before processing releases its reservation. After work
  begins, retain incurred cost and recover or retry the same attempt rather
  than turning repeated failures into unlimited free processing.
- Successful or rejected terminal processing settles the reservation once.
  Expired leases and worker crashes need reconciliation that accounts for work
  already performed; expiry alone must not mint free retries.
- An unchanged retry with the same account and idempotency key recovers the
  previous reservation/result. A changed body under that key is refused on
  these new admission paths. Receipt consumption, challenge consumption, and
  durable submission admission must have one atomic outcome.

Existing quarantine remediation is a separate supported behavior. Its current
idempotency rules must be reconciled explicitly with this new admission path;
do not accidentally block legitimate remediation or let it reset the subsidy.

A short wall-clock onboarding deadline is not required. Avoid penalizing a
contributor for setup delays or server processing time. Challenge expiry is a
security property distinct from window expiry. If a window expiry is later
introduced, display it before the user begins and honor previously reserved
work according to an explicit policy.

At exhaustion, explain the remaining choices: fund and run qualifying NEAR
inference, redeem genuinely available credit when supported, or use an invite.
Keep pending contributions visible and process already admitted work. Do not
silently extend the window or imply unearned funding is available.

## Native consent and activation requirements

Inference routing, off-device privacy scanning, local raw-body capture, and
sending captured bodies to the witness are separate decisions. Permission for
an extra NEAR privacy scan does not authorize inference routing or attested
contribution. Before enabling capture/contribution, explain that IronWire can
retain request/response bodies locally and that the contributor sends those
bodies to a configured witness. A settings save is not evidence that the proxy
is capturing correctly, the provider is funded, or a receipt has verified.

Setup may finish before activation. The activation milestone is a useful NEAR
inference session followed by an accepted contribution, with each intermediate
state visible. Do not label an account activated solely because a toggle was
saved, an invite was redeemed, or a certificate was received.

## Implementation and enablement gates

The first implementation can improve native orientation, consent, and honest
readiness feedback while preserving invite enrollment. It must not introduce
an unused authorization state machine or simulate wallet signup, available
inference funds, verified receipts, or successful redemption. The initial
consent slice exposes the existing attested-body contribution setting across
native shells; it does not establish capture or activation readiness.

The enrolled native preview/upload path currently refuses a configured witness
with `witness_claim_unavailable`: it cannot obtain the upload claim needed to
build the witness envelope (`daemon/preview.rs`). Resolve that claim acquisition
and envelope construction path, preserving its fail-closed behavior, before
claiming that the native invited pilot can complete attested contribution.

Before opening a subsidized window or attestation admission, require:

- Explicit account provisioning and stable account-anchor lifecycle rules.
- A complete funding/redemption/credential contract, or narrower wording that
  does not promise earned inference.
- Account and global spend controls, durable reservations, and recovery.
- Capture-side challenge preservation and the separate signed evidence profile.
- Atomic receipt/challenge consumption and tenant-safe replay protection.
- Live end-to-end validation, then an invited pilot demonstrating the full
  useful-inference-to-accepted-contribution loop.
- Explicit selection of economic limits and operator enablement; admission
  remains disabled when any required control is absent.

## Acceptance tests for the future admission implementation

1. An unknown NEAR login cannot create an account or tenant. Invalid wallet
   signatures and missing ownership controls fail without provisioning state.
2. A v1 redaction certificate, a permissive witness, unsigned receipt fields,
   and a client-declared verified flag cannot grant inference admission.
3. Missing, expired, wrong-audience, or foreign-account binding challenges fail.
   Provider-hashed bytes must contain the binding after every supported proxy
   transformation; a separately attached challenge does not qualify.
4. One receipt cannot admit different submission IDs or accounts, including
   concurrent cross-tenant requests and alternate signature encodings. The
   denial must not reveal another tenant's identity or submission.
5. Identical retries recover the same durable result without consuming another
   attempt, challenge, receipt, or spend reservation. Changed content fails;
   explicitly supported quarantine remediation retains its defined behavior.
6. Concurrent submissions cannot exceed either account attempts, account spend,
   or global spend. Failed quality checks consume attempts; local preview does
   not. Pre-processing failures release reservations, and post-processing
   failures and crash recovery preserve incurred costs.
7. Reinstall, key rotation, additional devices, account relinking, and supported
   account lifecycle operations cannot reset an exhausted window.
8. Admission cannot bypass authentication, tenant access grants, privacy or
   quality gates. Missing evidence fails closed on the new entitlement without
   weakening the existing invited path. `GET /v1/source` remains public.
9. Native onboarding never conflates the privacy scanner, inference funding,
   witness configuration, body-capture consent, processing, and acceptance.
   Pending credit never appears as spendable inference.
10. The invited live pilot verifies an actual provider receipt, accepted
    contribution, and absence of raw request/response bodies in the stored
    redacted artifact before broad enablement.

## Related

- [Attested-inference release design](2026-09-04-attested-inference-release-design.md)
  records deployment ordering and the limits of the current evidence profile.
- [Credit numbers API design](2026-09-01-credit-numbers-api-design.md) separates
  reported contribution credit from spendable value.
