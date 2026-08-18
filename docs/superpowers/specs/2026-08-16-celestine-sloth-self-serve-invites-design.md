# Self-serve invite claims for Celestine Sloth Society holders

**Date:** 2026-08-16
**Status:** Approved for implementation
**Scope:** A second self-serve invite cohort, gated on a CosmWasm `cw721`
holding rather than a NEAR NFT holding.
**Stacked on:** `near-legion-self-serve-invites` (`e304e285`, "Add self-serve
NEAR Legion invite claims"). That branch is the base; nothing in it is edited.

## What this adds

The Legion slice lets a NEAR account prove itself with a NEP-413 wallet
signature, checks it holds a token of `nearlegion.nfts.tg`, and mints one
invite code redeemable a fixed number of times. This slice does the same thing
for Celestine Sloth Society — originally a Stargaze collection, now on the
Cosmos Hub — with a Cosmos wallet signature and a `cw721` ownership query.

The two cohorts are independent: separate module, separate policy label,
separate pool, separate cap, separate env prefix.

## Why a parallel module rather than a generalised one

A collection-parameterised module (one config list, routes mounted per slug)
was considered and rejected. It looked attractive when both collections were
assumed to be NEAR NFTs, where the only difference would have been a contract
string. They are not: the chains differ, the signature schemes differ, the
address formats differ, the ownership queries differ, and the binding checks
differ in *kind* — NEAR needs a second RPC round-trip to bind a key to an
account, Cosmos does not need one at all.

Under those facts the shared surface would have been a slug and an HTTP shape,
with a chain-shaped enum under it. That is not reuse; it is a switch statement
with extra indirection, and it puts the working Legion path at risk to save
duplication that is mostly boilerplate. The parallel module keeps the base
branch's blast radius at zero.

The cost is real and accepted: `ClaimError`'s label/status discipline, the
challenge-store handling, the CORS layer, the cap check, and the grant-write
block are each written twice, and a future fix to that shared shape has to land
in both files. `InviteGrantSink` is the one thing genuinely shared — it is
already generic over `policy_label`, so it is imported, not redeclared.

## What differs from the Legion path

|                   | Legion (NEAR)                        | Sloths (Cosmos)                                     |
| ----------------- | ------------------------------------ | --------------------------------------------------- |
| Identity          | `alice.near`                         | `cosmos1…` bech32                                    |
| Signature         | NEP-413 / Ed25519 via `ring`         | ADR-036 / secp256k1 via `k256`                       |
| Binding check     | RPC: is this key FullAccess?         | Local: derived address must equal claimed address    |
| Ownership         | `nft_supply_for_owner` JSON-RPC view | `cw721 tokens{owner,limit:1}` smart query over LCD   |
| Network calls     | two (binding, ownership)             | one (ownership)                                      |

The binding check collapsing to a local computation is the substantive
improvement. A secp256k1 public key *is* the address:

```
addr = bech32(hrp, ripemd160(sha256(compressed_pubkey)))
```

so proving possession of the key and proving control of the address are the
same act. There is no equivalent of `PublicKeyNotFullAccess` needing an RPC
that can fail closed; there is only `PublicKeyAddressMismatch`, computed
locally and never able to return "unavailable".

## Eligibility overlap

An account may hold both a Legion token and a Sloth, and will then be able to
claim from both pools — two grants, `max_uses` seats each. The one-live-grant
rule is the V42 partial unique index on `(policy_label,
credential_binding_hash)`, and the labels differ.

This is deliberate. A cross-label check would be theatre: the binding hashes
are over addresses on two different chains, so the *same person* holding both
NFTs is not detectable at all, and nothing stops one person from claiming from
a NEAR account and a Cosmos account they both control. The cap is the real
control on total issuance; per-account uniqueness only stops the trivial
double-claim, and it still does that within each pool.

## Wire contract

Three routes, mounted only when configured:

```
POST /v1/onboard/celestine-sloths/challenge
  { "address": "cosmos1..." }
  -> 200 { "challengeId", "message", "nonce", "signDoc" }

POST /v1/onboard/celestine-sloths/claim
  { "challengeId", "address", "publicKey", "signature" }
  -> 201 { "inviteCode", "maxUses", "expiresAt" }

GET  /v1/onboard/celestine-sloths/status
  -> 200 { "claimed", "cap", "remaining", "maxUses" }
```

`publicKey` and `signature` are base64, matching what Keplr's and Leap's
`signArbitrary` return. `signDoc` is the full ADR-036 document the wallet is
asked to sign, returned so the page does not reconstruct it and drift from what
the server verifies.

The challenge id is returned in the response body, not a cookie, for the same
reason as Legion: the claim page is served from a different origin than the
issuer, and a `SameSite=Strict` cookie would not be sent. The id is
high-entropy, single-use, and TTL-bounded, so it is a capability handle in its
own right.

`CLAIM_MESSAGE` is distinct from both the Legion claim message and the account
enrolment message, so a signature captured from one ceremony can never be
replayed into another.

### Refusals

Every refusal maps to one stable public label and one status. Labels are wire
values the claim page switches on, so a cap-reached refusal never renders as a
signature failure.

| Label                           | Status |
| ------------------------------- | ------ |
| `AddressMalformed`              | 400    |
| `ChallengeNonceInvalid`         | 400    |
| `SignatureInvalid`              | 400    |
| `PublicKeyAddressMismatch`      | 400    |
| `AccountHoldsNoSlothToken`      | 400    |
| `AccountNotEligible`            | 400    |
| `InviteCredentialAlreadyBound`  | 409    |
| `CelestineSlothClaimCapReached` | 409    |
| `ChainRpcUnavailable`           | 503    |
| `ClaimBackendUnavailable`       | 503    |

An LCD failure surfaces as `ChainRpcUnavailable`, never as "holds nothing" — a
non-holder and an unreachable endpoint must not be indistinguishable, or the
page tells a genuine holder to go buy an NFT they already own.

### Check order

Signature first, so nothing downstream is observable without proving key
possession. Then the address binding, which is local and free. Then the cap, a
cheap local read — a full pool should not spend a network round-trip to refuse.
Then the denylist. Ownership, the only network call, last before the write.

## Grant shape

| Field                     | Value                                        |
| ------------------------- | -------------------------------------------- |
| `policy_label`            | `celestine-sloths`                           |
| `issuance_source`         | `celestine-sloths-cw721`                     |
| `issued_by_label`         | `celestine-sloth-claim`                      |
| `credential_binding_hash` | `sha256("cosmos-account:" ‖ address)`        |
| `tenant_mode`             | `Derived`, from the configured template      |
| `max_uses`                | 3 (default)                                  |
| `expires_at`              | now + 30 days (default)                      |

The `cosmos-account:` prefix is domain separation: the digest can never collide
with a `near-account:` digest or an `invite:` digest. The raw bech32 address is
never persisted, logged, or returned — consistent with the hash-only audit
convention.

The raw invite code exists in memory and in exactly one response body. It is
never stored, logged, or retrievable afterward. Registry cache invalidation
happens after the commit, so the cache never advertises an invite the database
rejected.

## Configuration

All keys are `TRACE_COMMONS_CELESTINE_SLOTHS_*`:

| Key                | Required | Default                             |
| ------------------ | -------- | ----------------------------------- |
| `ENABLED`          | yes      | off                                 |
| `CONTRACT`         | yes      | none                                |
| `LCD_URL`          | yes      | none                                |
| `TENANT_TEMPLATE`  | yes      | none                                |
| `BECH32_HRP`       | no       | `cosmos`                            |
| `CAP`              | no       | 100                                 |
| `MAX_USES`         | no       | 3                                   |
| `GRANT_TTL_DAYS`   | no       | 30                                  |
| `DENYLIST`         | no       | empty                               |
| `CORS_ORIGINS`     | no       | same list as Legion                 |

There is deliberately no default contract. The Legion module hardcodes
`nearlegion.nfts.tg` because that address is known and verified; the Sloth
contract address on the Cosmos Hub is not yet confirmed, and a wrong default
that silently queries a nonexistent contract is worse than an unmounted route.
Any missing required value makes `from_env()` return `None`, the routes stay
unmounted, and the surface 404s — the repo's fail-closed convention. A
half-configured deployment must not half-work.

Numeric values that fail to parse fall back to their defaults rather than
failing the process, matching Legion: an unparseable cap must never open the
surface wider than intended.

## Ownership query

A CosmWasm smart query over the LCD REST gateway:

```
GET {lcd}/cosmwasm/wasm/v1/contract/{contract}/smart/{base64({"tokens":{"owner":"<addr>","limit":1}})}
```

The response is `{"data":{"tokens":[...]}}`; a non-empty array means the address
holds at least one token. `limit:1` keeps the response bounded — the count is
irrelevant, only "any" matters.

Bounded 10-second timeout, as Legion has. Both a transport failure and a
contract-level error are `Err`, and the caller fails closed on `Err`.

This is the cw721 enumerable extension, which Stargaze's `sg721` implements.
**This assumption is unverified against the live contract** and is the one
thing in this design that must be checked against the real endpoint before the
feature is enabled in production. It is config-driven precisely so a mismatch
is a deploy-time discovery, not a code change.

## Dependencies

Three new direct dependencies, approved:

| Crate    | Version | Purpose                                 |
| -------- | ------- | --------------------------------------- |
| `k256`   | 0.13    | secp256k1 ECDSA verification            |
| `ripemd` | 0.1     | RIPEMD-160 for address derivation       |
| `bech32` | 0.11    | bech32 encoding of the derived address  |

All three are RustCrypto or core-ecosystem crates under permissive licenses
with small transitive trees. `cosmrs` was considered — one dep instead of three
— and rejected: it drags in `tendermint-rs`, `prost`, and `tonic` for what
amounts to one signature verification and one address derivation.

The `Cargo.toml` change lands as its own commit so the dependency addition is
reviewable on its own rather than buried in a feature diff.

## Wiring

- `lib.rs`: one `pub mod celestine_sloth_claim;`
- `trace_upload_claim_issuer.rs`: a second `Option<CelestineSlothClaimState>`
  field on the issuer state, built once in `build_state` so the in-flight
  challenge store is shared across requests, and a second conditional
  `.merge()` in `router_from_state` beside the Legion merge. The existing
  issuer tests construct state with `None`, as they already do for Legion.

No migration. V42 already provides everything this needs; `policy_label` is
just a different string.

## Testing

Hermetic, no network, TDD — test first for each unit below.

**Address derivation.** Known secp256k1 pubkey/address vectors; compressed and
uncompressed input; a non-default HRP.

**ADR-036 sign-doc canonicalisation.** Byte-exact expected output. This is what
signatures are computed over, so a formatting drift is a silent
authentication break rather than a visible failure.

**Signature verification.** Valid accept; corrupted signature reject; wrong
message reject; and the case that matters most — a signature that verifies
correctly against a public key that derives a *different* address must be
refused with `PublicKeyAddressMismatch`, not accepted.

**Challenge handling.** Single-use `take`; a challenge minted for one address
refused against another; unknown id refused.

**Ownership parsing.** Empty `tokens`; non-empty; malformed JSON; a
contract-level error; an HTTP failure. Each must land on the intended
`ClaimError`.

**Refusal table.** Every `ClaimError` variant asserted against its label and
status, so a rename cannot silently change the wire contract.

**Claim flow.** Cap boundary (at cap refuses, one below succeeds); denylist;
`CredentialAlreadyBound` → 409; a successful claim returns a code and writes
exactly one grant with the expected policy label and binding hash.

**Status endpoint.** Counts and remaining, including a backend failure → 503.

The ownership checker and the grant sink are both trait objects with test
doubles, exactly as Legion does it, so the whole flow runs without a live
Postgres or a live LCD.

## Out of scope

- The `/sloths` claim page itself, which lives in the community-site repo. This
  slice defines the wire contract it will target.
- Confirming the contract address and the LCD endpoint. Both are operator
  configuration; the design does not depend on their values.
- Any cross-chain identity linking between the two cohorts.
