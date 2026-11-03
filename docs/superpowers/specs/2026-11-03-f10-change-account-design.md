# F10: change the NEAR AI login an account uses

Status: APPROVED design. The owner approved every recommendation in
section 9, the DRAFT strings in section 2 and both notices in 2.6 on
2026-11-03 ("go with your recs on F10 and build it").
Date: 2026-11-03.

This is fix F10 of the one-connected-account spec
(`2026-11-02-one-connected-account-design.md`, section 4). It builds on the
change-account spec (`2026-11-02-change-nearai-account-design.md`, "the
companion" below), which designed option (a): prove the current identity
and the new one in one ceremony, then move the account's anchor. This spec
does not repeat the companion. It records what changed since it was
written, adds what it left open, and settles the parts that F10 needs to
ship:

1. **A wallet-capable proof of the current identity.** Wallet-origin
   accounts (`near-` tenants) hold a `wallet` anchor, not a NEAR AI login.
   The only NEAR AI proof of a wallet was the `{account_id}@near` mapping,
   which #1862 removed. F10 needs its own (section 3).
2. **The tenant namespace stops deciding how a device signs in.** A
   wallet-origin account that records a NEAR AI login keeps its `near-`
   tenant. The client reads the tenant prefix to decide how to sign in
   and renew, and both renewal paths refuse a `near-` tenant, so such an
   account could never renew by its new login (section 4.4).
3. **What a change does to payouts, F11 carries, devices and sessions**,
   now that #1841, #1862, #1865, #1868 and #1879 have landed (section 5).
4. **The flow in each shell** (section 2), abuse cases (section 6), tests
   (section 8) and the owner decisions (section 9).

Code references are to `origin/main` at 96ba35043 (v0.22.0) unless a
branch is named. Paths under `crates/trace-commons-contributor/src/` are
written without that prefix.

## Why now

1. **The field case, again, on 0.22.0.** The owner signed in with Google
   and then GitHub on a Mac whose account was enrolled through the NEAR AI
   wallet login. Both were refused before anything was replaced: the
   daemon logged `near_ai_sign_in_failed step=commons
   reason=near_ai_credential_different_account` for each provider. That is
   #1837 and #1840 working as designed. There is still no way to move the
   account to the new login. F10 is that way.
2. **Production payouts.** `docs/operator/automatic-credit-settlement.md`,
   "Before production payouts": F10 and F11 must both land. F11 has
   (#1865, #1879). Wallet-origin accounts get a NEAR AI login only through
   F10. The pilot has 0 wallet-origin accounts today, so F10 gates the
   production rule, not a pilot payout.

The owner's case is the simpler half: the account's current identity is a
NEAR AI login (made with the wallet provider), so the companion's
NEAR AI-to-NEAR AI move covers it. The wallet half (section 3) is for
`near-` accounts.

## 1. Terms

- **Account.** A Commons account, `(tenant_id, account_id)`. Traces,
  credit, referrals, devices, sessions, payout links, accruals and carries
  are keyed by it, never by anchor (companion 1.1).
- **Anchor.** A row of `trace_near_account_anchors`: a blind index of an
  identity, with `identity_source` `wallet` (a NEAR account name, V58) or
  `near_ai_login` (a NEAR AI subject, V63). `UNIQUE (anchor_hash)` across
  the deployment.
- **Current identity.** The account's one live anchor before the change.
- **New login.** The NEAR AI login (Google, GitHub or NEAR wallet, chosen on
  the daemon's browser page, #1835) the account moves to. Always a
  `near_ai_login` anchor: F10 never records a wallet anchor.
- **Change.** Insert the new `near_ai_login` anchor, retire the current
  one, re-key the account's provisioned devices, in one transaction. The
  tenant and the account id never change.

Who F10 is for:

| Account | Current anchor | F10 |
| --- | --- | --- |
| NEAR AI account (`nearai-` tenant) | `near_ai_login` | Change to another NEAR AI login. Current proof: a fresh NEAR AI sign-in (companion 3.7, step 1) |
| Wallet-origin account (`near-` tenant) | `wallet` | Change to a NEAR AI login. Current proof: a NEP-413 wallet signature (section 3). This is how such an account reaches payouts |
| Principal-linked legacy or pilot account | none | Not offered. Paid through an F11 carry (V146) |
| Pooled Devfolio tenant | none | Not offered. Excluded from payouts by design |

The server refuses a change in any tenant outside the two anchor
namespaces (`ANCHOR_NAMESPACES`, `trace-commons-protocol/src/admission.rs:206`)
with the uniform deny. That is fail-closed and needs no decision.

## 2. User flow

Every string in this section is **APPROVED** (owner, 2026-11-03, with the
recommendations in section 9). The three marked APPROVED with an earlier
date were approved before. No string says "Trace Commons"
(owner, 2026-11-02). Copy follows `docs/ui-copy-guide.md`: short buttons,
consequences beside the action, separate title and button keys (rule 4),
and no new notice without approval (rule 9).

### 2.1 Entry points

There are two ways in. Both end in the same change sheet.

1. **After a sign-in with a different NEAR AI account.** Today the sign-in
   is refused and nothing is stored (#1837, #1840). The account section
   shows the refusal (approved #1836 wording):

   | Key | Text |
   | --- | --- |
   | `account.login.mismatch.body` | This app is connected to a different NEAR AI account. Sign in with that account again. (APPROVED 2026-11-02) |
   | `account.login.mismatch.sign_in` | Sign in |
   | `account.login.mismatch.change` | Use a different account |

   F10 adds the last button and keeps the refused login aside for the
   change (companion 3.7, "Hold a non-matching login aside"), so the
   person does not sign in with the new login twice.
2. **Deliberately, from the account section.** Beside the connected line
   ("Connected with {provider}", companion 5.1), a button
   `account.login.change` "Use a different account". There is no new
   login held aside, so the new login is signed in first, inside the
   sheet.

A wallet-origin account has a third, payout-shaped way in. Its payouts
section today shows the generic refusal and, after V147, an unlink with no
reason (accepted as a known gap until F10, owner 2026-11-03). F10 replaces
that with a status line and a button:

| Key | DRAFT text |
| --- | --- |
| `payouts.needs_login.body` | Payouts need a NEAR AI login. Add one to this account to set up payouts. |
| `payouts.needs_login.add` | Add a NEAR AI login |

It sits in the payouts section where the link status would be. It is a
status, not a banner or notification, but it appears on accounts where
nothing appeared before, so it is listed as a notice for approval
(section 2.6). The wallet sign-up disclosure, which already says
"Payouts need a NEAR AI login, which joining with a NEAR account doesn't
set up." (APPROVED, #1868), is unchanged.

### 2.2 macOS (lead client)

A sheet from the account section, in three steps, each a page of one
sheet. The step that proves the current identity comes first when no new
login is held aside, so a person who cannot prove it learns that before
signing in with anything new.

**Step 1: confirm it's you (current identity).**

- NEAR AI account: the daemon's browser chooser opens with only the
  remembered provider offered and marked "You used this login last time"
  (APPROVED 2026-11-02). The access token of that sign-in is held in memory
  for this ceremony and never stored (companion 3.7, step 1). Its subject
  digest must match the remembered one, or the step fails with
  `account.change.error.current_login`.
- Wallet-origin account: the browser opens the Commons wallet page, the
  same one `NearAccountConnectView` already uses for joining
  (`/account/near/provision/wallet`), with the change challenge
  (section 3). The person approves the message in their wallet.

**Step 2: sign in with the new login** (skipped when one is held aside).
The chooser opens with every provider offered and none marked.

**Step 3: confirm.**

| Key | DRAFT text |
| --- | --- |
| `account.change.title` | Use a different NEAR AI account |
| `account.change.body` | Your traces, credit and payout settings stay. From now on, sign in with {new_provider}. Your {old_provider} login will no longer connect to this app. |
| `account.change.body.wallet` | Your traces and credit stay. From now on, sign in with {new_provider}. Your NEAR wallet will no longer sign in to this app. |
| `account.change.private_ai` | Private AI will use your {new_provider} account. Keys made in the old account are removed. |
| `account.change.confirm` | Switch |
| `account.change.cancel` | Cancel |
| `account.change.step.current` | Confirm it's you |
| `account.change.step.current.wallet` | Approve the request in your NEAR wallet. |
| `account.change.step.new` | Sign in with the new account |
| `account.change.error.current_login` | Sign in with {old_provider} first to confirm it's you. |
| `account.change.error.wallet` | Your NEAR wallet didn't confirm. Try again. |
| `account.change.error.in_use` | That NEAR AI account is already in use. Choose a different account. |
| `account.change.error.too_soon` | This account changed its login recently. Try again later. |
| `account.change.error.unavailable` | Couldn't switch accounts. Try again. |
| `account.change.error.retired` | This app no longer uses that NEAR AI account. Sign in with {provider}. |

`{old_provider}` is NEAR wallet, GitHub or Google from the remembered
login; an unknown provider uses the body without it ("Your old login will
no longer connect to this app."). `account.change.error.retired` is shown
on an ordinary sign-in with a login a change retired, not in the sheet.
`account.change.error.too_soon` exists only if the owner chooses a
cooldown (decision 9).

After a change the account section shows "Connected with {new_provider}".
There is no separate confirmation (companion 5.3), so no new notice.

Accessibility: each step's primary button has its own accessibility name
naming the step ("Confirm it's you with GitHub", "Sign in with the new
account", "Switch to Google"), per copy-guide rule 5.

### 2.3 GTK, Windows and Tauri

Same three steps, same strings (shared through `witness_copy` as the
wallet strings are today), in each shell's account section. Native macOS is
the lead client and parity target (#1173 D1). GTK and Windows follow in a
second slice. Tauri is frozen on macOS and low priority elsewhere; it gets
the change only if the owner wants it before the Tauri freeze lifts.

Wallet-origin accounts are reachable from every shell's wallet join
(`NearAccountConnectView` on macOS, the wallet flows in GTK, Windows and
Tauri), so the payouts entry point (2.1) must reach every shell that can
make one. Decision 11.

### 2.4 Daemon verbs

Two IPC verbs and the matching C ABI functions (both header copies):

- `account_login_change { step }`, where `step` is `start`,
  `prove_current`, `sign_in_new`, `confirm`. Each returns the sheet's
  state: which step, which provider to show, and a fixed error label.
- `account_login_change_cancel`: drops the ceremony and the held-aside
  login, revoking any key minted in it.

The ceremony spans two browser legs (a wallet approval or NEAR AI
sign-in, then a NEAR AI sign-in), so its expiry is set for that, not
copied from the sign-up ceremony's, which covers one leg. The server's
expiry is published in the start answer; the daemon starts the ceremony
only once the held-aside login, if any, is in hand. An expired ceremony
starts again from the current-identity leg, since a wallet signature is
bound to the old nonce; a new login held aside is kept.

Status gains `login_change_available: bool` (the server publishes the
routes, the account's tenant is in an anchor namespace, the device did not
join by code, decision 8) and, for a wallet-origin account,
`payouts_need_login: true`.

### 2.5 What the person sees on other devices

Nothing new. Their device sessions stay (decision 6). Their stored NEAR AI
login is now the retired one, so their next NEAR AI sign-in or renewal is
refused as a different account, and they see the existing approved
mismatch body. "Sign in" there opens the chooser with the new provider
marked, once the server's answer tells them what it is (section 4.3).

A wallet-origin account's other devices have no NEAR AI login stored, and
their wallet anchor is now retired, so they can renew neither way. They
land in the existing needs-sign-in state, and their Sign in opens the
NEAR AI chooser. That works because the recorded identity kind (4.4), read
from the server's answer at their next sign-in, routes a `near-` tenant to
the NEAR AI path; until the daemon has it, the server's `identity` field
in the sign-in answer sets it.

### 2.6 Notices (both APPROVED by the owner, 2026-11-03)

1. The mismatch block's new "Use a different account" button
   (`account.login.mismatch.change`). The block is already approved; the
   button changes what it offers.
2. The payouts status line for wallet-origin accounts (`payouts.needs_login.*`).
3. Nothing else. No banner, no notification, no done message.

## 3. The wallet-capable proof of the current identity

### 3.1 Requirements

- **Server-verifiable and fresh.** Bound to this change ceremony's nonce,
  so it cannot be replayed from a sign-up, another change or another
  device.
- **Matches the account's live wallet anchor**, computed the way the
  anchor was: the blind index of `(network, account_id)`
  (`NearAccountIndexPepper::index`, `near_account_identity.rs:169`).
- **No new primitive.** The Commons already verifies NEAR wallets for
  sign-up.
- **No new dependence on NEAR AI's claims** about a wallet. The owner ruled
  that only a recorded NEAR AI login proves a payout link (#1862); a
  NEAR AI-side wallet mapping would bring that mapping back through F10.

### 3.2 Options

| | Proof | For | Against |
| --- | --- | --- | --- |
| **W1 (recommended)** | NEP-413 signature by a FullAccess key of the NEAR account, on the Commons wallet page, over a change challenge | Exactly the sign-up proof (`account_near.rs`: `verify_nep413`, `near_account_has_full_access_key` via `view_access_key_list`; `PendingNearProvisioning::issue/restore/verify_classified`). Server-checked freshness. The person's wallet, not the device's keychain, holds the key, which a device thief (T2) does not get from the device alone | Needs the NEAR RPC to answer (it already must, for sign-up). A lost wallet cannot change (T4, decision 10) |
| W2 | A NEAR AI wallet sign-in whose `/users/me` maps to `{account_id}@near` | One browser leg instead of two | The mapping #1862 removed. Depends on NEAR AI's undocumented login address and on NEAR AI's verification, not ours |
| W3 | Device key and native session alone | No wallet needed | Option (b) of the companion: what a device thief holds. A takeover path |

### 3.3 W1 in detail

`POST /v1/account/near-ai/rebind/start`, on an account whose live anchor is
`identity_source = 'wallet'`, answers the same challenge shape the wallet
sign-up start does (`near_provisioning.rs:591-659`): `message`, `nonce`,
`recipient`, `wallet_url`, `network`, plus the ceremony id and expiry. It
differs in three ways:

1. **Domain separation.** The message is a change message, not a sign-up
   message, and the stored ceremony records `purpose = rebind`. A sign-up
   finish refuses a rebind ceremony and a rebind finish refuses a sign-up
   one, so neither signature can stand in for the other. NEP-413 already
   binds `recipient` (the Commons origin) and the 32-byte nonce.
2. **The account name comes from the person**, typed in the sheet as it is
   at sign-up, and is checked at finish against the live wallet anchor's
   index. The server does not open the anchor's sealed account name to
   pre-fill it: that would put a decrypted wallet name on the wire for a
   device-only session (T2).
3. **Bound to the session.** The ceremony row carries the native session's
   tenant, account and device. Finish refuses if any differ.

`rebind/finish` carries
`current_proof: {"kind": "wallet", "account_id", "wallet_public_key",
"wallet_signature"}` in place of `current_access_token`, and the server:

1. takes the ceremony (single use; the five-attempt cap applies);
2. verifies the device signature over the ceremony, PKCE verifier and the
   digests of both proofs (companion 3.5, finish step 1);
3. verifies the NEP-413 signature over the change message, nonce and
   recipient (`verify_nep413`);
4. introspects the new NEAR AI login (`introspect_login`);
5. asks the NEAR RPC, last, whether `wallet_public_key` is a FullAccess key
   of `account_id` (as `finish_taken` orders it: an unreachable RPC is
   `503` for every account, `near_ai_sign_in_unavailable("near_rpc")`);
6. computes the wallet index of `(cfg.network, account_id)` and requires it
   to equal a live `wallet` anchor of this account. A mismatch is the
   uniform deny, so the answer never says whether the typed name is
   someone's account.

Readiness is the conjunction: the route denies, fail-closed, unless both
the wallet path (`wallet_readiness_refusal`) and the NEAR AI login path
(`near_ai_login_v2_readiness_refusal`) are ready. Either missing gives the
existing `not_ready` answer with the missing control's name.

For a NEAR AI account, `current_proof` is
`{"kind": "near_ai_login", "access_token"}` and step 3/5/6 become the
companion's: introspect, anchor, require a live `near_ai_login` anchor of
this account. The two NEAR AI tokens must anchor differently.

### 3.4 Freshness of a NEAR AI current proof

The server cannot tell a NEAR AI access token from a fresh interactive
sign-in from one minted with the stored refresh token. A device thief with
malware on the device can mint one. The companion's interactive re-sign-in
is a client rule, and it defends only against a thief without code
execution. A wallet proof does not have this gap. Decision 1 accepts or
narrows that; decision 9's cooldown and a revert window are the server-side
backstops.

## 4. Server

### 4.1 Migration V150

Next free number as of 2026-11-03 (V149 is on main; no open PR claims a
higher one). Re-check at implementation.

```sql
ALTER TABLE trace_near_account_anchors
    ADD COLUMN retired_at TIMESTAMPTZ;
CREATE INDEX trace_near_account_anchors_live
    ON trace_near_account_anchors (tenant_id, account_id)
    WHERE retired_at IS NULL;

-- One live anchor per account from now on (decision 3).
CREATE UNIQUE INDEX trace_near_account_anchors_one_live
    ON trace_near_account_anchors (tenant_id, account_id)
    WHERE retired_at IS NULL;

ALTER TABLE trace_near_provisioning_ceremonies
    ADD COLUMN purpose TEXT NOT NULL DEFAULT 'provision'
        CHECK (purpose IN ('provision', 'rebind'));

CREATE TABLE trace_account_login_changes (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id),
    account_id UUID NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    from_source TEXT NOT NULL CHECK (from_source IN ('wallet', 'near_ai_login')),
    switched_back BOOLEAN NOT NULL,
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
-- RLS forced, tenant_id = trace_current_tenant_id(), as every table.

GRANT UPDATE (retired_at) ON trace_near_account_anchors TO trace_ingest_runtime;
GRANT UPDATE (anchor_hash) ON trace_near_provisioned_devices TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_account_login_changes TO trace_ingest_runtime;
```

- **One live anchor per account.** V82 merges left accounts holding two
  anchors. Before the unique index, the migration checks for any account
  with two un-retired anchors and **fails** if it finds one, naming only
  the count. It does not pick one to retire: which identity a merged
  account keeps is an owner call. Pilot count expected 0 (to run before
  deploy; section 7).
- **Merges.** `trace_near_account_merge` (V82, owned by
  `trace_near_account_merge_guard`) moves the absorbed account's anchors
  onto the survivor and documents that "a survivor that already holds an
  anchor ends up holding both". Under the one-live-anchor index that merge
  would fail. V150 replaces the function, as its guard role (the V64/V81
  ownership pattern; never under the migration role), so that an absorbed
  anchor lands on a survivor that already holds a live anchor as
  retired: the row moves, so its `UNIQUE (anchor_hash)` still blocks a
  fresh sign-up (T5), but it no longer signs in. The guard role gains
  `UPDATE (retired_at)`. Which identity survives a merge is then the
  survivor's, which is decision 3's rule applied to merges. The
  alternative, enforcing one live anchor only in the change route and
  dropping the index, leaves merges as they are.
- `trace_account_login_changes` holds no anchor, subject or provider. It
  exists for the cooldown (decision 9) and to count changes per account;
  `trace_account_audit` remains the audit record.
- **The login resolver.** `near_anchor_tenant` (`:336`) runs as
  `trace_login_resolver` and must read `retired_at`. Whether that is a
  column grant or a change to a definer function depends on how the
  resolver reads the table today (`docs/operator/login-resolver-role.md`);
  the implementation PR states which and adds a test that runs the
  resolver as a non-superuser LOGIN role.
- **Pepper rotation.** `rotate_near_anchor_row`
  (`near_account_identity.rs:454`, anchor-hash salting spec) rewrites the
  row's index. It must carry `retired_at` unchanged; a test pins it.
- Column-scoped grants follow V100: a compromised runtime can retire or
  revive an anchor of its own tenant and re-point its own devices at its
  own anchors (the foreign key holds), but cannot rewrite an index.

### 4.2 Readers

The companion's four (3.5, "Readers to update") apply, with `retired_at IS
NULL` and a named `near_ai_login_retired` on a sign-in with a retired
login. Added since:

| Reader | Change |
| --- | --- |
| Wallet sign-in account lookup (`postgres_account_onboarding.rs:493`) | A retired wallet anchor resolves to "retired", answered with the uniform deny on the wallet path. It must not mint a fresh account: the row still holds the index (T5) |
| `payout_account_login_anchors` (`postgres_payout.rs`, after #1862) | `AND retired_at IS NULL`. A retired login cannot prove a new link |
| `trace_account_admission_linkage_ready()` (V81) | No change needed: it reads provisioned-device rows, which the change re-keys. A test asserts it stays true across a change in both namespaces |
| Payout link confirm's re-read ("the account holds a `near_ai_login` anchor", `postgres_payout.rs:1533-1541`), which is also what a V146 carry rides on | `AND retired_at IS NULL`, so a link, and so a carry source, needs a live login |
| `trace_link_legacy_instance_device` (V146, owned by `trace_legacy_invite_link_guard`): "the account holds an anchor" | No change needed: after a change the account always holds a live anchor. If it is ever narrowed to live anchors, the replacement runs as its guard role |
| V147's predicate | A one-off at deploy; nothing to change. Any later "no NEAR AI login" check reads live anchors only |

`auth_provider` stays recorded-only on the anchor. The sign-in answer gains
it (companion 3.5, "Sign-in answer") so a device can mark the right
provider.

### 4.3 Routes and the commit

`POST /v1/account/near-ai/rebind/start` and `/finish`, native session plus
device signature, published in `capabilities_v2` as
`near_ai_rebind_start_path` / `near_ai_rebind_finish_path`. The companion
(3.5, "Routes") gives the shape. Additions:

**Start** refuses, with the uniform deny:

- a tenant outside the anchor namespaces;
- a device that joined by code (decision 8), read from
  `device_keys.onboarding_origin`;
- a revoked device (the session check already fails; the start re-reads
  `device_keys.revoked_at` in its own transaction so a revoke between
  session check and start wins);
- an account with no live anchor, or (decision 9) a change within the
  cooldown, answered as the named `login_change_too_soon` (409,
  `no-store`) because the person holds a valid session of the account.

It answers `current_kind: "wallet" | "near_ai_login"` and, for a wallet,
the challenge in 3.3.

**Finish**, after the proofs (3.3), runs one transaction in the account's
tenant (`begin_trace_tenant_transaction`):

1. `SELECT ... FROM trace_accounts WHERE ... FOR UPDATE` on the account,
   then advisory locks on both anchors in sorted order (the provisioning
   lock, `pg_advisory_xact_lock(hashtextextended(anchor, 0))`). A
   concurrent change of the same account waits on the row; a concurrent
   sign-up with the new login waits on the anchor lock.
2. Re-read: the account is open; the device is not revoked; the current
   anchor is still live and still this account's; no admission
   reservation for it is in progress (`AdmissionDecision::Busy`); the
   cooldown (decision 9) has passed. Any failure: roll back, uniform deny
   (or `login_change_too_soon`).
3. Set the current anchor's `retired_at`. This comes first because V150
   allows one live anchor per account.
4. Insert the new anchor (`identity_source = 'near_ai_login'`, its
   `auth_provider`, sealed subject, pepper and key refs as
   `near_ai_login_provision_in_tenant` writes them) with `ON CONFLICT
   (anchor_hash) DO NOTHING`. Zero rows and the conflicting row is a
   retired anchor of this same account: clear its `retired_at` instead
   (switching back). Zero rows otherwise: roll back, which un-retires the
   current anchor, and answer `near_ai_login_in_use` (409, `no-store`).
5. Re-key every `trace_near_provisioned_devices` row of the account
   (revoked devices included, so no row points at a retired anchor) to
   the new anchor.
6. Payouts (section 5.1).
7. Insert `trace_account_login_changes` and the `trace_account_audit` row.
8. Commit. Answer `{account_id, tenant_id, anchor_hash, auth_provider,
   identity: "near_ai_login"}`. No new native session is minted; the
   device's session stays valid.

Neither token nor the wallet signature is stored, logged or put in an
error. Rate limits under their own labels (`near-ai-rebind-start`,
`near-ai-rebind-finish`), plus the per-ceremony cap.

### 4.4 Tenant namespace no longer decides the sign-in path

On the server, nothing in the NEAR AI sign-in path checks the `nearai-`
prefix: `provision_against_anchor` resolves the anchor to whatever tenant
holds it, and `near_ai_login_provision_in_tenant` finds the account by
anchor (`postgres_account_onboarding.rs:598-640`). V62 dropped the V58
check that tied a tenant id to its anchor. So a `near_ai_login` anchor in a
`near-` tenant signs in to that tenant. A PostgreSQL test pins it.

The client is different. Eleven sites (in `daemon/account_onboarding.rs`,
`daemon/account_session_renewal.rs`, `daemon/admission_setup.rs`,
`daemon/nearai_onboarding.rs`, `witness/transport.rs`) call
`is_near_ai_tenant_id` or `is_near_tenant_id`, most of them to choose how
to sign in or renew.
`renew_account_session` (`nearai_onboarding.rs:1084`) and
`renew_account_session_by_device_key` (`:1327`) both bail with
`near_ai_enroll_invalid` unless the tenant is `nearai-`, and
`Path::for_tenant` (`account_onboarding.rs:321`) picks the wallet path for
anything else. Without a change, a wallet-origin account that completes F10
can never renew by its new login.

The fix: the daemon records which identity the account signs in with
(`identity: wallet | near_ai_login`), from the server's answer at
enrollment and at a change, in the account record (and in F7's
connected-account record once it exists). Every site above keys on that,
not on the prefix. A record without the field (written before this
change) falls back to the prefix, which is right for every account that
has not changed. The prefix checks that guard shape (a tenant id must be
in one of the two namespaces) stay. Decision 4 is whether to do this or
move the account to a `nearai-` tenant instead.

### 4.5 Audit

`trace_account_audit`, action `near_ai_login_rebound`, actor the device
principal, outcome `success`, `safe_metadata`
`{"from":"wallet"|"near_ai_login","switched_back":bool}`. No anchor
hashes, no provider labels, no subject, no wallet name (companion 3.6).
`from` adds nothing the tenant prefix does not already say. Payout effects
write their own `trace_payout_audit` rows (5.1). Refusals are counted by
label in logs only (`near_ai_login_in_use`, `near_ai_login_retired`,
`login_change_too_soon`, `uniform_deny`), never with a tenant or account
id.

## 5. Effects on everything the account holds

Tenant and account id do not change, so these are untouched: traces,
credit and accruals, referrals and referral codes, the inference-connection
choice (V79), devices and their keys (V139), join codes (V140), V81 tenant
links, legacy claim principals (V146).

| Held | On a change |
| --- | --- |
| Current anchor | Retired, row kept (T5). Switching back revives it |
| Provisioned devices | Re-keyed to the new anchor (companion 3.5) |
| Native sessions of other devices | Kept (decision 6) |
| This device's session | Kept |
| Admission window | Restarts once, keyed by the new anchor (companion 3.5); bounded by the cooldown |
| Credit holds | Untouched |
| Payout links, pending link requests, carries | Section 5.1 |
| NEAR AI keys minted in the old NEAR AI account | Revoked by the daemon with the old session after commit, best effort, tagged with the old identity's digest (one-connected-account 3.2, step 5; F5) |

### 5.1 Payouts

**Pending link requests** (started, not confirmed) of the account are
deleted in the change transaction, as V147 did. A request proved by the
old login then answers `not_found` at confirm. Counted, not audited per
row, as V147 counts.

**A NEAR AI account's live link (`waiting` or `active`) is kept**
(decision 7). It pays an organization proved by the old login, and step 1
of the change just re-proved that same login. Its carries (V146) follow
the link and are untouched, since the source account, its tenant links and
its link are unchanged. New links prove against the new login only.

**A wallet-origin account's live link is unlinked** in the change
transaction, with V147's shape (`recipient_unavailable_at` and
`unlink_requested_at`, one `trace_payout_audit` row, label
`near_ai_login_required`). There should be none: links on accounts with no
NEAR AI login were refused from #1862 and revoked by V147 at deploy. This
is the backstop for the F11 spec's open note ("A link proved through a
wallet anchor before #1862 would be carried once that account records a
login", section 13): F10 is the event that makes a wallet-origin account
hold a login, so it is where such a link would otherwise become carryable.

**After the change**, a wallet-origin account holds a live `near_ai_login`
anchor, so:

- the person can link payouts with the new login (`payout_account_login_anchors`
  returns it);
- if the account holds V81 tenant links, the link it makes is carried to
  those legacy accounts by V146 at confirm, as for any NEAR AI account;
- its `payouts_need_login` status clears.

That is how F10 unblocks payouts for wallet-origin accounts. Nothing in F11
changes.

## 6. Abuse cases

| # | Case | What holds |
| --- | --- | --- |
| A1 | **Takeover with a NEAR AI login alone** | Start needs a native session of the account and a device signature. A login alone signs up a fresh account or is refused (companion T1) |
| A2 | **Takeover with the device** (stolen laptop, malware) | A NEAR AI account: the thief must also present a NEAR AI token that anchors to the current identity. With code execution on the device they can mint one from the stored refresh token (3.4). Backstops: cooldown, and decision 1's alternatives. A wallet-origin account: the thief must also sign with a FullAccess key of the wallet, which the device's keychain does not hold |
| A3 | **Replay** | Ceremony single-use, bound to the session's tenant, account and device, PKCE-bound, five attempts, short expiry. The device signature covers digests of both proofs. NEP-413 binds nonce and recipient; `purpose = rebind` keeps sign-up and change signatures apart. NEAR AI tokens are introspected at finish, not trusted from a cache |
| A4 | **New login already anchored elsewhere** | `ON CONFLICT (anchor_hash) DO NOTHING` under the anchor's advisory lock: `near_ai_login_in_use`. Shown only to someone who just proved both identities, so it reveals a fact about their own login. No cross-tenant merge (decision 5) |
| A5 | **Retired login reused for a fresh account** (Sybil, T5) | The retired row keeps `UNIQUE (anchor_hash)`. Sign-in with it answers `near_ai_login_retired`; it never mints an account |
| A6 | **Pooled Devfolio and legacy tenants** | Outside the anchor namespaces: uniform deny at start. They hold no anchor to change |
| A7 | **Revoked device** | Its session no longer authenticates; start and finish re-read `revoked_at` in their transactions |
| A8 | **Device joined by code** | Refused at start (decision 8). Such a device holds no NEAR AI login, and owner decision 2 of 2026-11-02 limits it to the account's own login |
| A9 | **Concurrent changes** | Account row lock, then anchor locks in sorted order. The second change re-reads its current anchor as retired and is denied. A change racing a sign-up of the new login: whichever takes the anchor lock first wins; the other gets `in_use` or a minted-then-rolled-back account (`DEVICE_KEY_REGISTERED_ELSEWHERE`) |
| A10 | **Change racing a payout link confirm** | The change deletes pending requests in its transaction. A confirm that committed first left a link proved by the old login, which is kept (NEAR AI account) or unlinked (wallet-origin) |
| A11 | **Change racing an admission reservation** | Refused while one is in progress (`Busy`), so no lease straddles two anchors |
| A12 | **Churn** (change back and forth to reset admission windows) | Cooldown (decision 9) and the per-account change table |
| A13 | **Cross-tenant rebind** | Forced RLS; every write in the session's tenant; the ceremony row records the tenant and finish refuses another. A PostgreSQL test runs tenant A's session against tenant B |
| A14 | **Leaking identities through audit or logs** | 4.5: no anchor pairs, providers, subjects or wallet names in any row or log line |

## 7. Rollout

1. Pilot, read-only, before V150: count accounts with more than one
   un-retired anchor (expected 0; V150 fails otherwise), and accounts by
   anchor source (wallet-origin expected 0 on the pilot, 2026-11-03).
2. Deploy server (V150, routes off in `capabilities_v2` until a flag is
   set, so a client cannot offer the flow early).
3. Ship the daemon and macOS with the flow behind the capability.
4. Owner runs the field case on his Mac: current proof with the NEAR AI
   wallet login, new login Google or GitHub. If the new login already
   holds an account (companion Q9), it answers `in_use`.
5. GTK and Windows in the next slice; Tauri per decision 11.
6. Update `automatic-credit-settlement.md`, "Before production payouts":
   F10 done, with the version that shipped it.

## 8. Test plan

Server unit tests:

- NEP-413 change message: layout pinned, and a sign-up signature fails a
  change verification and the reverse (`purpose` separation).
- The wallet index check: right name and key passes; right key, other
  name fails; FullAccess false fails; RPC unreachable is `503` for every
  account.
- Readiness: wallet path ready, login path not (and the reverse) denies
  with the missing control.

PostgreSQL (`ingest::app::`, against a real database; added to the
`ingest-bin-postgres` slice) for both a `nearai-` and a `near-` tenant:

1. Change succeeds: new anchor live, old retired, devices re-keyed,
   audit row label-only, change row written.
2. Old identity refused after the change: NEAR AI sign-in answers
   `near_ai_login_retired`; wallet sign-in answers the uniform deny; no
   tenant or account minted (count before and after).
3. New login signs in to the same tenant and account afterwards,
   including in a `near-` tenant.
4. `near_ai_login_in_use`: the new login already anchored in another
   tenant; and a concurrent sign-up with it, run from two connections.
5. Switching back revives the retired anchor; a third login after that.
6. Cooldown: a second change inside it is `login_change_too_soon`.
7. Two concurrent changes of one account: exactly one wins.
8. `payout_account_login_anchors` returns only the live anchor; a payout
   proof with the retired login is refused; with the new one, accepted.
9. Wallet-origin account: no link before; after the change a link
   confirms; with a V81 tenant link, the link is carried (V146).
10. Wallet-origin account with a live link (inserted by the test as a
    pre-#1862 link would be): the change unlinks it with one payout audit
    row.
11. NEAR AI account with a live link and carries: link and carries
    unchanged.
12. Pending link request deleted; its confirm answers `not_found`.
13. `trace_account_admission_linkage_ready()` true before and after.
14. Admission reservation in progress: change refused.
15. Revoked device, code-joined device, legacy tenant, pooled tenant: each
    refused at start.
16. RLS: tenant A's session cannot start or finish a change in tenant B,
    run as a non-superuser LOGIN role (a superuser would pass vacuously).
17. The login resolver reads `retired_at` as its real role.
18. Pepper rotation keeps `retired_at`.
19. V150 refuses to apply with an account holding two live anchors.

Daemon:

- Each of the eleven prefix sites with a `near-` tenant whose record says
  `near_ai_login`: renewal uses the NEAR AI login; with no field, the
  prefix decides as today.
- The ceremony: current-proof mismatch (`current_login_required`), no new
  login (`new_login_required`), `in_use`, `too_soon`, deny; each leaves the
  stored credentials unchanged.
- After success: old keys revoked with the old session (a failed revoke
  stays pending with the old digest), the held-aside session promoted,
  record rewritten with the new provider and digest.
- Cancel drops the held-aside login and revokes its key.

Shells: the macOS sheet's three steps (snapshot of each state, Swift unit
tests on the state mapping), the C ABI fields in both header copies. GTK
and Windows in their slice.

## 9. Owner decisions

**Decided 2026-11-03: the owner approved every recommendation below,
decisions 1 to 14, as written** ("go with your recs on F10 and build
it"). Each item keeps its options for the record; the recommendation is
the decision.

Each has a recommendation. The companion's open questions are folded in
where they still apply.

1. **Proof of a NEAR AI current identity.** A fresh interactive sign-in
   with the remembered provider (client-enforced; the server cannot tell
   it from a refresh, 3.4), or a silent refresh of the stored login.
   *Recommend: interactive sign-in*, with the cooldown (9) as the
   server-side backstop.
2. **Proof of a wallet current identity.** W1, a NEP-413 signature on the
   Commons wallet page; W2, NEAR AI's wallet mapping; W3, the device
   alone. *Recommend: W1.*
3. **One live identity per account.** A change retires the old anchor, and
   V150 enforces one live anchor per account. The alternative keeps the old
   one live beside the new (multi-link, companion option (c)). For a
   wallet-origin account this means its NEAR wallet stops signing in to
   the account. *Recommend: retire; one live anchor.*
4. **Wallet-origin accounts keep their `near-` tenant**, and the daemon
   decides how to sign in from the recorded identity, not the tenant
   prefix (4.4). The alternative moves the account to a `nearai-` tenant,
   which moves traces, credit, devices and links, at F11's scale.
   *Recommend: keep the tenant.*
5. **A new login that already has an account** is refused
   (`near_ai_login_in_use`). Later: merge across tenants, or let the person
   close the other account first. *Recommend: refuse in this slice.*
6. **Other devices on a change.** Keep their sessions, or sign them all out.
   *Recommend: keep.* Signing them out does not stop a thief who holds this
   device, and locks the owner out of the others.
7. **Payout links on a change.** NEAR AI account: keep the live link and
   its carries (the old login was re-proved in step 1). Wallet-origin
   account: unlink any live link (expected none). Pending requests:
   deleted for both. The alternative closes every link and asks for a
   relink with the new login, which also releases carries until then.
   *Recommend: as stated.*
8. **Devices that joined by code cannot start a change.** *Recommend:
   refuse*; the person changes from a device that signed in with the
   account's login, and the code-joined device follows.
9. **Cooldown and switching back.** Switching back is allowed (revives the
   retired anchor). A cooldown between changes bounds admission-window
   resets and a device thief's churn. *Recommend: 7 days, no revert window
   in this slice.* A revert window (the retired login can undo a change for
   N days) is the stronger defence for 3.4 and can follow.
10. **Lost current identity** (lost wallet, closed GitHub). No self-service
    path in this slice. Delayed recovery (companion option (d)) or an
    operator-assisted path later. *Recommend: neither now*; revisit if
    support sees requests.
11. **Shells and order.** macOS first; GTK and Windows in a second slice;
    Tauri only if needed before its freeze lifts. The payouts entry point
    (2.1) reaches every shell that offers wallet join, in the second
    slice at the latest. *Recommend: as stated.*
12. **Build order and F7/F8.** F10 was planned on F7 (the connected-account
    record) and F8 (`commit_connected`). Neither exists on main or in an
    open PR. Options: build F7 and F8 first; or ship F10's server half now
    and its client half on the existing staged-commit pieces (#1837's
    ceremony, `switch_identity`'s journal), moving it onto F8 when F8
    lands. *Recommend: server first (one PR), then the client on the
    existing pieces*, since F10 gates production payouts and the server
    half does not depend on F7 or F8.
13. **What "remember" shows.** Provider only, or a masked hint read
    locally from NEAR AI (companion Q7). *Recommend: provider only.*
14. **Copy and notices.** Every DRAFT string in section 2, and the two
    notices in 2.6: the mismatch block's "Use a different account" button
    and the payouts status line for wallet-origin accounts.

## 10. Where the server build differs from the text above

Recorded as the server half was built (2026-11-03):

1. **The ceremony's purpose lives in its payload, not a column.** A change
   ceremony row stores a `NearAiRebindPending` with a required `purpose:
   "rebind"` field, and every ceremony payload denies unknown fields, so a
   change row never parses as a sign-up row or the reverse. V150 adds no
   column to `trace_near_provisioning_ceremonies`.
2. **The login resolver is unchanged.** It still maps an anchor to its
   tenant. The retired check is in the in-tenant account lookup of both
   sign-in paths, which refuses a retired anchor before anything is
   written: by name (`near_ai_login_retired`, 409) on the NEAR AI path,
   with the uniform deny on the wallet path. No new resolver grant.
3. **A waiting payout link on a wallet-origin account holds the change**
   (`payout_link_review_required`, 409, logged) instead of being closed:
   the runtime cannot close a waiting link past V146's guard, and such a
   link should not exist after #1862 and V147. An active one is unlinked as
   section 5.1 says.
4. **The wallet's own sign-in keys are revoked.** A wallet-origin change
   also revokes the account's `trace_near_identities` rows for the proven
   NEAR account name, so the wallet stops signing in through the NEAR
   login route as well (decision 3).
5. **No rollout flag.** `capabilities_v2` publishes
   `near_ai_rebind_start_path` / `near_ai_rebind_finish_path` alongside the
   other paths; nothing calls them until a client that knows them ships.
6. **The evidence-ledger lease check is a guard-owned function**,
   `trace_admission_anchor_busy`, because the runtime cannot read the
   ledger. Granted to `trace_ingest_runtime` and, as V149 does, to the
   migrating login.
