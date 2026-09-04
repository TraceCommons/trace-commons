# B: explicit NEAR account provisioning handoff

Status: verification module implemented; no route, database migration, account
creation, session issuance, device registration, funding, or admission enabled.

## Implemented contract

`account_onboarding::PendingNearProvisioning::issue(cfg, account_id,
device_public_key, browser_binding, now)` validates the canonical account spelling
without normalization, accepts configured mainnet/testnet, generates a CSPRNG
nonce, and retains a 300-second ceremony. `browser_binding` is a nonzero 32-byte
hash derived from a server-held browser/native authorization ceremony, not a
finish-body assertion. The configured network label must match the configured
RPC chain; this module does not attest the chain itself.

The returned wallet challenge commits the provisioning purpose, network,
account, device-key digest, browser binding, and expiry in its message; NEP-413
also signs the nonce and configured recipient. The device separately signs
`device_signing_bytes()` to prove possession. Root may expose these bytes to the
native client as part of the start payload; they contain no bearer credential.

`verify(self, cfg, assertion, browser_binding, now)` rejects wrong/expired local
bindings and invalid wallet/device signatures before the existing FullAccess
RPC check. Unavailable RPC and absent ownership refuse uniformly. Configuration
changes during a ceremony refuse rather than switching recipient/network/RPC.
The public production entry uses the real existing RPC verifier. Only internal
unit tests can inject an ownership answer.

`VerifiedNearProvisioning` has private fields, no Deserialize/Clone/Debug, and
read-only accessors for account ID, network, wallet key, device key, anchor hash,
ceremony hash, and expiry. It proves control, not a tenant or entitlement.
The root transaction must recheck expiry after asynchronous RPC verification.

Canonical anchor bytes:

```
SHA256("trace_commons.near_account_anchor.v1\n"
       || u64_le(network.len) || network
       || u64_le(account_id.len) || account_id)
```

Fixture `mainnet` / `alice.near` yields
`9c2335d9afa6312a1b75700f1baf786dd207823002eaff79da64dd572cf53463`.

These 32 opaque bytes remain stable across keys/devices; do not substitute a
public-key hash. Root approved this framing and the named 300-second security
TTL. Economic amounts, redemption, and account-retention policy remain open.

## Root integration: exact ordering and ownership

The only shared-file change in this branch is `pub mod account_onboarding;` in
server `src/lib.rs`. Root owns all remaining wiring below.

1. Add explicit start/finish routes beside existing account handlers, behind a
   default-disabled provisioning capability. Preserve existing unknown-key
   login denial. Rate-limit before issuing random challenges or querying RPC.
   Preserve the existing uniform status/body and timing-floor approach for
   account-facing denials; this module provides a safe error label, not HTTP
   anti-enumeration behavior.
2. Reuse `account_native_auth` PKCE, exact loopback redirect and one-time code
   exchange. Bind the browser ceremony to the native authorization request,
   chosen account/network and device. Recheck account-switch context at finish.
   Do not weaken native weak-session restrictions to link authenticators.
3. Store `PendingNearProvisioning` only in a server-owned ceremony registry.
   Atomically take the matching record at finish before `verify`. A missing,
   consumed, wrong-purpose or mismatched browser ceremony yields the same deny.
   Current module cannot rehydrate a durable record; extend its trusted storage
   adapter if choosing PostgreSQL rather than the existing in-process store.
   An in-process implementation must retain its single-instance limitation.
4. Verify, then begin the provisioning transaction and recheck expiry. Resolve
   or reserve the network/account anchor with a narrow privileged operation.
   Invalid assertions must not create a tenant, account, or device row.
5. Resolve an existing mapping or create a new account/tenant under a root-owned
   tenant-allocation policy; write the anchor, identity and authorized device
   plus hash-only audit as one transaction. Never silently reassign a key already
   linked to another account. An anchor conflict must resolve to the same
   account or a non-enumerating refusal, not a second onboarding window.
6. Finish native code issuance only after commit. Retrying a committed operation
   needs a durable idempotent result/recovery path; replaying an assertion must
   not create a new account. A failed consumed ceremony may require a fresh
   ceremony without consuming an admission allowance. No module function issues
   account sessions, upload claims, inference keys, or window budget.

V33 currently makes `public_key` globally unique but `near_account_id` is merely
attribution and is not uniquely namespaced by network. Reusing that key resolver
as the stable anchor would reset identity when keys rotate. Root must allocate
and register a migration for a narrow network/account-anchor mapping and its
transactional API; no migration number is reserved in this branch. Forced RLS
and restricted cross-tenant resolution apply. Store no raw key/account/body in
new audit rows. Existing identity records retain their existing storage contract.

## Remaining lifecycle decisions

Key rotation/additional devices resolve the same anchor. Define proof/recovery
and user intent for linking to an already-existing local account; the wallet
signature alone must not silently merge accounts or redirect payouts. Account
consolidation, unlink/delete, and minimal abuse-record retention need explicit
policy before public provisioning; deleting a device or key must not erase the
ledger's stable anchor. Never infer a unique human from account creation cost.

## Validation and integration gates

Focused module tests cover canonical identities, exact TTL boundaries, purpose,
recipient, config drift, browser binding, wrong device, account/ceremony replay,
FullAccess failure/outage, and stable anchors across devices/keys. Compile-fail
doctests reject cloning pending ceremonies and deserializing verified results.
Integration tests call only local refusal paths and issue no network requests.

Root still must run real PostgreSQL transaction/race/rollback and RLS tests,
handler-level take-once replay and unknown-login regression tests, native PKCE
interception/account-switch tests, and real wallet signature-vector validation.
This branch's cryptographic tests sign independent NEP-413 preimages locally;
they are not evidence of a deployed wallet signup or a working native funnel.

Local verification uses this worktree's `CARGO_TARGET_DIR=target`:

- `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --lib --locked`: passed.
- `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --lib account_onboarding --locked`: 5 passed.
- `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test account_onboarding_contract --test license_boundary --locked`: 2 contract and 4 license tests passed; server binaries compiled.
- `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --doc account_onboarding --locked`: 2 compile-fail doctests passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Warnings-denied `cargo clippy -p trace-commons-server --lib --locked` with the repository's existing five lint allowances: passed. Full all-target/feature gates remain root integration checks.
