# Legacy account payout migration

Status: APPROVED design (owner, 2026-11-03: all eleven recommendations in
section 12, "looks good"). Implementation on the same branch.
Date: 2026-11-03. Fix F11 of
`2026-11-02-one-connected-account-design.md` (section 4).

Code references are to `origin/main` at 33aed75aa, unless a branch is
named. The payout gate this design must fit is on branch
`claude/payout-near-ai-login-only` (commit caca5bc2e, not yet a PR): a
payout link is accepted only when the proving NEAR AI login matches one of
the account's recorded `near_ai_login` anchors, on every deployment.

## 1. The problem

A payout is per account and per tenant. Every payout table is keyed
`(tenant_id, account_id)` with a foreign key into the same tenant's
`trace_accounts` (`migrations/V132__payout_rails.sql:97-128` links,
`:194-229` accruals, `:249-277` lines). Credit earned in a tenant is owed
by an account in that tenant and paid through a link in that tenant.

Legacy and pilot accounts live in tenants outside `near-` / `nearai-`. They
are principal-linked: a legacy device's principal gets an account through
`create_or_reuse_account` the first time it mints a login link
(`crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:21547-21590`;
`crates/trace-commons-server/src/db/postgres.rs:3884`). They hold no
`near_ai_login` anchor, so under the gate every payout link they ask for is
refused (`NoLoginAnchor`, answered on the wire as `subject_mismatch`). Their
credit can accrue but can never be paid. Production payouts stay off until
that is fixed.

There are three kinds of legacy tenant:

| Kind | How it was made | Today |
| --- | --- | --- |
| Invite tenant | A device redeemed an individual invite (`device_keys.onboarding_origin = 'invite'`, with an `onboarding_invites` row) | Can link to a NEAR account with the V81 tenant link; the app offers the move (`legacy_migration.rs:204`) |
| Instance-enrolled tenant | A device enrolled with an operator instance key. It is `invite`-origin in `device_keys` but has no `onboarding_invites` row | Cannot link: the link function answers `device_not_eligible` (`migrations/V81__legacy_invite_link.sql:247-252`); the app does not offer the move (`offered_for` requires an empty `instance_id`, `legacy_migration.rs:204-206`). Pilot accounts are likely this kind (owner query Q2) |
| Pooled tenant | A shared event code (the Devfolio tenants), marked in `trace_legacy_invite_pooled_tenants` | Never links (V81 rule 3). Excluded from payouts by design. Nothing here changes for it |

## 2. Constraints the code imposes

These decide the design. Each was checked in the code, not inferred.

**C1. An anchor is a sign-in route.** NEAR AI sign-in resolves the login's
anchor to the tenant that holds it and provisions there
(`provision_against_anchor`,
`crates/trace-commons-server/src/db/postgres_account_onboarding.rs:414-441`;
`near_ai_login_provision_in_tenant`, `:594-650`). It inserts a
`near_ai`-origin device into whatever tenant the anchor names. An anchor
recorded on a legacy account would put a `near_ai` device into a `tenant-`
namespace at the holder's next sign-in. That fails readiness rule 2
(`trace_account_admission_linkage_ready()`,
`migrations/V81__legacy_invite_link.sql:388-425`: "an active device
outside those namespaces whose origin is not `invite`"), and ingest refuses
to start with account admission on
(`account_admission_permissions_or_linkage_not_ready`). One migrated pilot
account, one sign-in, and the next restart fails for the whole deployment.

**C2. A login can be anchored once in the whole deployment.**
`trace_near_account_anchors` has `UNIQUE (anchor_hash)`
(`migrations/V58__near_account_provisioning.sql:23`). Anyone who already
uses the app's Private AI with their NEAR AI login already has a `nearai-`
account holding that anchor. The same login cannot also be anchored on
their legacy account. The closed #1847 hit exactly this and answered
`near_ai_login_unavailable`.

**C3. Credit cannot cross tenants.** Accruals carry an FK into the same
tenant's accounts, and an owned accrual keeps its owner
(`trace_payout_accruals_guard_update`, `V132__payout_rails.sql:419-431`).
Settlement batch lines are never re-keyed (payout spec
`2026-10-21-nearai-credit-payout-design.md`, section 6). Account merge is
same-tenant only (`docs/operator/account-merge.md`, "confirms B is a
distinct, open account in the same tenant").

**C4. A legacy account has no app session.** The payout routes need an
account session; a device bearer is refused (`require_account_session`,
`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/payout_link.rs:112-125`).
The app gets an account session only from NEAR AI enrollment or renewal
(`daemon/nearai_onboarding.rs:1030`, `:1274`). A legacy device has a device
bearer and, at most, a web session from a login link. After the app's
legacy move, the legacy key is retired (`legacy_migration.rs`, flow step
5), so nothing can act as the legacy account afterwards.

**C5. The tenant link already proves ownership.** The V81 link
(`POST /v1/account/invites/legacy-link`) is made under a NEAR account's
session, with a statement signed by the legacy tenant's registered device
key (`docs/operator/legacy-invite-migration.md`, "What linking does"). One
legacy tenant links to one NEAR account; a second claim is refused and
recorded as a conflict; pooled tenants never link. It is the one place
today where the system has proof that a NEAR AI login holder also holds a
legacy tenant.

Together: the two F11 candidates as written fail. (a) "move each account
onto a NEAR AI tenant, carrying its credit" runs into C3. (b) "record a
NEAR AI login on the legacy account" runs into C1 and C2, and C4 leaves the
person no session to record it with.

## 3. Design: the payout link follows the tenant link

The holder links payouts once, from their NEAR AI account, exactly as today.
The server carries that link to each legacy account in every legacy tenant
the NEAR AI account has linked. Each legacy account is then paid its own
credit, from its own tenant, to the organization the NEAR AI login proved.

```text
NEAR AI account (nearai-…)          legacy tenant (tenant-…)
  near_ai_login anchor  ──proves──▶ payout link L (org X)
  V81 tenant link ───────────────▶ carried link L' (org X, same seal)
                                    legacy account's accruals ──▶ paid to X
```

### 3.1 How the legacy account gets an anchor a proof can match

It does not get one. The proof is the NEAR AI account's own anchor, which
the gate already requires; the bridge to the legacy account is the V81
tenant link, signed by the legacy device key. Nothing is written to
`trace_near_account_anchors` in a legacy tenant, so C1 and C2 never arise:
sign-in, readiness and the anchor table are untouched.

Whether this counts as "a NEAR AI login that matches a recorded
`near_ai_login` anchor" is owner decision 2. The recommended reading is
yes: the login matched is recorded, and the legacy tenant's link to its
account is device-signed.

### 3.2 Who triggers it

- **The person, in the app.** Two steps, both existing: the legacy move
  (`legacy_invite_migrate` IPC, `legacy_migration.rs:935`), which signs in
  with NEAR AI and makes the V81 link; then the payout link from the NEAR AI
  account (`daemon/payout_link.rs`). The carry is automatic. The order does
  not matter: whichever step comes second carries (section 3.3).
- **The operator, never by default.** No operator route links a tenant or
  carries a link. The operator's part is marking pooled tenants (already a
  prerequisite of V81) and resolving V81 conflicts. The one exception, an
  instance-enrolled tenant whose device key is lost, is owner decision 3.

### 3.3 The carry

A SECURITY DEFINER function, `trace_carry_legacy_payout_links(p_tenant,
p_account, p_rail)`, owned by a new NOLOGIN, NOBYPASSRLS role
`trace_legacy_payout_carry_guard` (the V81 pattern). Called with the NEAR
account's tenant context set; it refuses if `p_tenant` is not
`trace_current_tenant_id()` or is not `near-` / `nearai-`.

For the NEAR account's live link on the rail (the `active` or `waiting`
one; none means nothing to carry), and for each live V81 link of that
account (`trace_legacy_invite_links`, `revoked_at IS NULL`):

1. Re-check the legacy tenant is not pooled. V81 already refuses pooled
   links; this is belt and braces, because a tenant can be marked pooled
   after it was linked.
2. For each open account in the legacy tenant (owner decision 4):
   - if it already has a live link that is not a carry, skip it, with an
     audit label (owner decision 6);
   - if it has a live carry of an older generation of the source link,
     mark that carry for unlink, as a recipient change does today;
   - write a `waiting` link in the legacy tenant: the same
     `recipient_kind`, `sealed_recipient`, `recipient_index`,
     `recipient_ref`, `recipient_key_ref`, `index_pepper_ref` and
     `subject_index` as the source. The seal is bound to the rail and
     recipient only, not to a tenant (`payout_recipient_seal.rs:39-55`),
     so a copy opens correctly.
   - record the carry in `trace_legacy_payout_carries` (section 4).
3. The payout pass activates the waiting link as it does any other, once
   the account's previous link is closed (`postgres_payout.rs:660-668`).
   Nothing on the send side looks at the tenant's namespace or at anchors.
   The pass finds tenants through `tenants_with_live_payout_links`
   (`postgres_payout.rs:1785-1803`, under `trace_payout_enumerator`, every
   tenant with a live link on the rail), and the shared predicate checked
   before every send is `payout_account_standing`
   (`postgres_payout.rs:1805-1845`: the account exists, is not closed, and
   has no open credit hold), applied in `payout_pass.rs:956-963`. A legacy
   account meets it like any other.

It is called in three places, each in the caller's transaction, so a carry
never lands half-done:

- `confirm_payout_link_request` (`postgres_payout.rs:1449`), after a `link`
  is written on a `near-` / `nearai-` account;
- the same function after an `unlink`: every live carry of that source gets
  `unlink_requested_at`, as the account's own unlink would;
- the V81 link handler, after a `linked` outcome, so a person who linked
  payouts before moving their legacy identity is carried at the move.

It is idempotent: a second call with nothing changed writes nothing.

**V81 revocation.** When an operator revokes a tenant link
(`legacy-invite-migration.md`, "Resolving a conflict"), its carries must
stop. A trigger on `trace_legacy_invite_links` (`revoked_at` set) marks
every live carry under it for unlink and sets `released_at` (owner decision
10 offers a runbook step instead). The operator revokes under the NEAR
account's tenant context, so the trigger function must be SECURITY DEFINER,
owned by `trace_legacy_payout_carry_guard`, with the guard's legacy-side
policies; as an invoker trigger it would match no legacy row under forced
RLS and silently do nothing. A test revokes as a non-superuser login and
asserts the carry was released. Credit already paid stays paid, as with
any unlink.

**No session on the legacy account is ever needed** (C4). The legacy
account's link has no request row: it is created by the carry, not by a
start and confirm.

### 3.4 Instance-enrolled tenants

The V81 link refuses a device with no `onboarding_invites` row (C5's
function, `V81__legacy_invite_link.sql:247-252`). The recommended extension
(owner decision 3): a sibling function, `trace_link_legacy_instance_device`,
with the same statement, signature check, challenge, pooled check and
one-link-per-tenant rule, that

- accepts an `invite`-origin, unrevoked device of the tenant with **no**
  `onboarding_invites` row;
- writes the `trace_legacy_invite_links` row, with `invite_subject_hash`
  from `device_keys` as stored;
- grants **no** invite trust and changes no admission state, because there
  is no invite to carry over;
- audits `legacy_instance_linked`, hash-only, like `legacy_invite_linked`.

The client offers the move to instance-enrolled devices too
(`offered_for`, `legacy_migration.rs:204`), and the statement names the
device's stored subject hash, which the issuer already returns
(`POST /v1/device/invite-subject`). The issuer answers
`device_not_invite_onboarded` only for NEAR and NEAR AI devices, so an
instance device reads its hash today; to be confirmed in the
implementation's first test.

### 3.5 Credit with no account yet

A batch line is keyed to `account:{id}` only when its principal was linked
to an account when the batch was written; otherwise to the bare principal,
and the accrual stays unowned until that principal is linked
(`V132__payout_rails.sql:190-193`, payout spec section 4). A legacy device
that never minted a login link has no account, so its credit is owed to
nobody and a carry has no account to land on.

Owner decision 5 (2026-11-03, option A): when the V81 link is made, link
the signing device's **signed-upload principals** to one account in the
legacy tenant, and do the same for each device that later attests under
the link (V91). These are both forms that
`account_trust::device_upload_principal_refs` returns for each actor the
server can derive from the device key: the app's actor, whose subject is
`user_subject_hash(cfg.user_subject)` and, for an invite device without a
contributor label, `user_subject_hash(device_key_id)` (the actor
`repair_native_device_upload_principal` already builds); each instance
enrollment's subject for the tenant (`trace_instance_enrollments`); and
the bare device-key actor of device-JWT uploads. (The first version of
this section said the app's uploads carry the bare actor; the review of
#1865 showed they carry the subject form.) The accrual pass then claims their unowned
accruals the normal way, once, with its own `accrual_claimed` audit row.
Only proven devices are linked; nothing is inferred for the tenant's other
principals. If either principal already belongs to a live account in the
tenant (the device minted a login link), the other joins that account; a
principal that was unlinked is left unlinked.

Why not the device principal (`device_principal_ref`): the first
implementation test showed that linking it claims nothing. A legacy
device writes no V135 upload alias rows (`postgres.rs:2011-2018`: "Legacy
registered devices use signed claims as independent login principals"),
so its credit is keyed by the signed-upload principal itself.

**Known limit.** An upload whose subject cannot be derived from the
device key (an invite device enrolled with a contributor label, whose
`user_subject` is the label) has a principal the claim cannot name. Its
credit stays unowned and is not paid. Pilot query Q5 measures how much
credit is unowned in non-pooled legacy tenants, which bounds this; check
it against real `auth_principal_ref`s before turning payouts on.

### 3.6 Existing credit and attestations

- **Credit:** nothing moves. Settlement batches, ledger rows and accruals
  stay in the legacy tenant with their owners. The legacy account's `owed`
  is what the carried link pays. Credit holds and payout holds on the legacy
  account apply as they do now. The NEAR AI account's own credit is
  unchanged.
- **Ceilings:** per-send, per-account and credential ceilings apply per
  account, so the person's NEAR AI account and each legacy account have
  their own (owner decision 7).
- **Referral payouts** (`payout_referral_owed`) follow the account the same
  way.
- **Attestations:** contribution attestations and receipts already issued
  under the legacy tenant stay valid and are not reissued; they name the
  legacy tenant's hashes, which do not change. The countersigned V81 link
  record is untouched. A carry is not an attestation and is not
  countersigned; it is a server-side row with an audit trail.
- **Coexistence:** a legacy device that keeps contributing under its legacy
  tenant keeps earning there, and that credit is paid through the carry
  too (owner decision 8).

### 3.7 Mismatch and conflict

| Case | Outcome |
| --- | --- |
| NEAR AI login on the payout link is not the NEAR account's anchor | Refused by the gate (`subject_mismatch`), before any carry |
| NEAR account is wallet-origin with no login anchor | Its own link is refused, so there is nothing to carry |
| A second NEAR account claims an already-linked legacy tenant | V81 refuses (`legacy_link_tenant_claimed`) and records a conflict; the first account's carries stand |
| The operator resolves the conflict for the second account | Revoke the first link (trigger releases its carries), resolve, the right account links and carries |
| Tenant marked pooled after linking | The carry skips it; existing carries are released by the same check on the next call, and the runbook says to release them when marking |
| Legacy account already has its own live link | Skipped, audited `legacy_carry_skipped_own_link`; operator decides (owner decision 6) |
| Legacy account closed | Not carried; an existing carry is released |
| Carry function fails (database error) | The whole confirm or V81 link transaction rolls back; the person retries. No partial state |

## 4. Data model: V146

V145 is the latest migration on main and no open PR adds V146.

```sql
-- V146__legacy_payout_carries.sql
CREATE TABLE trace_legacy_payout_carries (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,          -- the legacy account
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    link_id UUID NOT NULL,             -- the carried link, in this tenant
    legacy_link_hash TEXT NOT NULL CHECK (legacy_link_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_link_hash TEXT NOT NULL CHECK (source_link_hash ~ '^sha256:[0-9a-f]{64}$'),
    carried_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    released_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, link_id),
    CHECK (tenant_id !~ '^(near-|nearai-)'),
    FOREIGN KEY (tenant_id, link_id, account_id, rail_id)
        REFERENCES trace_payout_links(tenant_id, link_id, account_id, rail_id)
);
CREATE INDEX trace_legacy_payout_carries_source
    ON trace_legacy_payout_carries (source_link_hash) WHERE released_at IS NULL;
ALTER TABLE trace_legacy_payout_carries ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_payout_carries FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_payout_carries
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
```

- The source link and the V81 link live in the NEAR account's tenant, so
  they are referenced by hash (domain-separated SHA-256 of the link id, as
  `trace_payout_audit.link_id_hash` already is), never by id or tenant: a
  legacy-tenant row cannot be joined to the NEAR account's tenant.
- The guard role gets narrow, role-scoped policies, as the readiness guard
  does (`V81__legacy_invite_link.sql`, `account_readiness_pooled`): SELECT
  on `trace_legacy_invite_links`, `trace_legacy_invite_pooled_tenants`,
  `trace_accounts` and `trace_payout_links`; INSERT and a column-limited
  UPDATE (`unlink_requested_at`) on `trace_payout_links` and
  `trace_legacy_payout_carries`, each `USING`/`WITH CHECK
  (tenant_id !~ '^(near-|nearai-)')` for the legacy side. The function
  never writes the NEAR account's tenant. EXECUTE is granted to the role
  the payout store already runs as (`payout_tenant_client`), and to the
  invite runtime for the V81 handler's call.
- The migration asserts the guard role is NOLOGIN, NOBYPASSRLS and not a
  superuser, as V132 does for its roles (`V132__payout_rails.sql:80-89`).
- If owner decision 3 is taken: the sibling link function and its grants
  go in the same migration.
- No change to `trace_near_account_anchors`, to readiness, or to any
  existing CHECK. `trace_payout_audit.action` keeps its list: carries use
  `link_created` and `unlink_requested` with new labels (section 5).
- No backfill is planned: #1841's count found 0 live payout links across
  the 22 tenants it listed on 2026-11-02, but that set may have left out
  legacy-routed tenants. Q4 re-counts. If it finds a NEAR account with a
  live link and a live V81 link, the rollout makes one carry call per such
  account; nothing is done in SQL.

`PayoutStore` (`crates/trace-commons-server/src/db/mod.rs`) gains no new
read for the gate: `payout_account_login_anchors` on the gate branch stays
as it is. The carry runs inside `confirm_payout_link_request`. The V81
handler's store call (`postgres.rs`, link) gains the carry call after a
`linked` outcome.

## 5. Audit (hash-only)

| Where | Row | Content |
| --- | --- | --- |
| Legacy tenant, `trace_payout_audit` | `link_created`, label `legacy_tenant_link` | `account_id_hash`, `link_id_hash`, `actor_ref_hash` of the NEAR account's actor ref hashed again (no id) |
| Legacy tenant, `trace_payout_audit` | `unlink_requested`, label `legacy_tenant_link_released` | Same, when a source unlink, a V81 revocation, a pooled mark or a closed account releases a carry |
| Legacy tenant, `trace_payout_audit` | `link_created` refused, label `legacy_carry_skipped_own_link` | Account hash only |
| NEAR tenant, `trace_account_audit` | `legacy_payout_carried`, `success` | `{"carried": <count>, "skipped": <count>}`; no tenant, account or link ids |
| Legacy tenant, `trace_account_audit` | `legacy_instance_linked` (decision 3) | `legacy_tenant_hash`, `device_key_id`, as `legacy_invite_linked` writes |

Logs carry the same labels and counts. No tenant id, account id, org id,
subject, token or URL appears in any row or log line.

## 6. Fail-closed behaviour

- The gate is unchanged: a NEAR account with no matching login anchor links
  nothing, so carries nothing.
- A legacy account's own start is still refused (`NoLoginAnchor`). The
  carry is the only way a legacy account gets a link.
- The carry function refuses, as a whole, a call whose tenant context is
  not the NEAR account's, a source that is not that account's live link,
  or a `p_tenant` outside `near-` / `nearai-`.
- A pooled or closed target is skipped, never carried.
- A database failure rolls back the caller's transaction (confirm, unlink
  or V81 link). A NEAR account's link and its carries commit together or
  not at all.
- With the payout rail off, the confirm route answers 503 as today, so no
  carry is written. With the V81 switch off
  (`TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED`), no tenant link is made, so
  nothing is carried.
- Production payouts stay disabled (`docs/operator/automatic-credit-settlement.md`,
  "Before production payouts") until the owner turns them on after rollout.

## 7. Devfolio and other pooled tenants

Excluded three times over: the V81 link refuses a pooled tenant
(`tenant_pooled`, `V81__legacy_invite_link.sql:256-262`), the instance
sibling repeats that check, and the carry re-checks
`trace_legacy_invite_pooled_tenants` for every target. The app already
stops offering the move after a pooled refusal (`legacy_migration.rs`,
"A refusal that is final"). Pooled tenants' credit stays where it is, as
the cutover runbook says. No change to the pooled marker, its runbook or
the Devfolio onboarding path.

## 8. Rollout on the pilot

Read-only first. The owner runs these; each returns counts only. RLS is
forced, so run them as the migration owner or once per tenant.

- **Q1.** Open, non-pooled accounts in tenants outside `near-` / `nearai-`:
  ```sql
  SELECT count(*) FROM trace_accounts a
   WHERE a.closed_at IS NULL AND a.tenant_id !~ '^(near-|nearai-)'
     AND NOT EXISTS (SELECT 1 FROM trace_legacy_invite_pooled_tenants p
                      WHERE p.tenant_id = a.tenant_id);
  ```
- **Q2.** Of those tenants, how many are invite tenants and how many
  instance-enrolled:
  ```sql
  SELECT EXISTS (SELECT 1 FROM onboarding_invites i
                  WHERE i.tenant_id = d.tenant_id
                    AND i.invite_subject_hash = d.invite_subject_hash) AS has_invite,
         count(DISTINCT d.tenant_id)
    FROM device_keys d
   WHERE d.revoked_at IS NULL AND d.tenant_id !~ '^(near-|nearai-)'
     AND NOT EXISTS (SELECT 1 FROM trace_legacy_invite_pooled_tenants p
                      WHERE p.tenant_id = d.tenant_id)
   GROUP BY 1;
  ```
- **Q3.** Live V81 links, and unresolved conflicts:
  `SELECT count(*) FROM trace_legacy_invite_links WHERE revoked_at IS NULL;`
  and `SELECT count(*) FROM trace_legacy_invite_link_conflicts WHERE resolved_at IS NULL;`
- **Q4.** Live payout links in legacy tenants (expected 0):
  `SELECT count(*) FROM trace_payout_links WHERE tenant_id !~ '^(near-|nearai-)' AND state IN ('waiting','active');`
- **Q5.** Unowned accruals in non-pooled legacy tenants, per tenant count
  only: `SELECT count(DISTINCT tenant_id), count(*) FROM trace_payout_accruals WHERE account_id IS NULL AND tenant_id !~ '^(near-|nearai-)';`
  (pooled tenants then subtracted with the marker join).
- **Q5b.** Whether there is anything to pay at all: finalized settlement
  batches and owned accruals in non-pooled legacy tenants, counts only:
  ```sql
  SELECT count(*) FROM trace_credit_settlement_batches b
   WHERE b.tenant_id !~ '^(near-|nearai-)'
     AND NOT EXISTS (SELECT 1 FROM trace_legacy_invite_pooled_tenants p
                      WHERE p.tenant_id = b.tenant_id);
  SELECT count(*) FROM trace_payout_accruals a
   WHERE a.account_id IS NOT NULL AND a.tenant_id !~ '^(near-|nearai-)'
     AND NOT EXISTS (SELECT 1 FROM trace_legacy_invite_pooled_tenants p
                      WHERE p.tenant_id = a.tenant_id);
  ```
  If both are 0, legacy credit was never settled on the pilot, and the
  owner may prefer to defer this work.
- **Q6, a question, not a query.** Whether each pilot legacy holder uses
  the app with the same NEAR AI login. Anchor overlap cannot be computed
  without the index pepper, and this design does not need it; the answer
  only tells us how many people have a NEAR AI account already.

- **Q7, an owner fact.** Whether the pilot has every pooled tenant marked
  and `TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED=true`. The V81 runbook
  requires the marks before linking is enabled
  (`legacy-invite-migration.md`, "Enabling linking"); this design carries
  nothing until both hold.

Then:

0. If Q7 says linking is off: review the pooled candidates
   (`legacy_invite_tenants|review_before_linking` in
   `legacy-invite-counts.md`), mark them, confirm the attestation key,
   and enable linking, in the runbook's order.
1. Land the server half (V146, the carry, the trigger; decision 3's
   sibling if approved) behind the existing switches. Nothing changes until
   a NEAR account confirms a link or a tenant links.
2. Staging: one invite tenant and one instance tenant, each moved and
   payout-linked, a settlement run, and the staging rail paying both the
   NEAR AI account and the legacy account to one org.
3. Client half: offer the move to instance-enrolled devices (decision 3).
   No new screens.
4. Pilot: deploy with the payout rail still in its current mode. The pilot
   holders move and link (the owner first, on their own pilot tenant).
   Re-run Q3 and Q4; check the audit rows of section 5.
5. Only then, the owner decides whether to turn production payouts on; the
   "Before production payouts" section of the settlement runbook is
   updated to point here.

## 9. Test plan

TDD: each test below is written first and seen red.

**PostgreSQL, server** (`crates/trace-commons-server/tests/legacy_payout_carry_pg.rs`,
run in `database suites against a real PostgreSQL`; the carry called as a
login holding only the payout runtime role, never a superuser):

- a NEAR account's confirmed link is carried to every open account of its
  linked legacy tenant, with the same seal and indexes, and the pass
  activates it;
- a recipient change carries the new generation and marks the old carry
  for unlink; an unlink marks every carry;
- a V81 revocation releases every carry under it (trigger), and nothing
  else;
- a pooled tenant, marked before or after linking, is never carried;
- a closed legacy account is not carried;
- a legacy account with its own live link is skipped and audited;
- a NEAR account with no login anchor carries nothing;
- the function refuses a wrong tenant context, a `tenant-` caller, and a
  source link that is not the caller's;
- the guard role cannot read or write the NEAR account's tenant through the
  function's policies, and the runtime cannot read the legacy tenant
  directly (RLS forced);
- a database failure inside the carry rolls back the confirm;
- readiness (`trace_account_admission_linkage_ready()`) stays true after a
  move, a carry and a later NEAR AI sign-in, and no anchor or `near_ai`
  device is written in a legacy tenant (C1, C2 regression);
- the legacy account's accruals are paid by the carried link and the NEAR
  account's are not double-counted;
- decision 3: an instance-enrolled device links, gets no invite grant and
  no trust change; a pooled or already-claimed instance tenant is refused;
- decision 5: the signing device's unowned accruals are claimed once, by
  the legacy account, with an `accrual_claimed` row;
- every audit row of section 5 is present and contains no raw identifier.

**Ingest** (`trace_commons_ingest_internal/tests/payout_link_tests.rs`,
`legacy_invite_link_tests.rs`, against PostgreSQL in
`ingest-bin-postgres`): confirm and unlink carry end to end; the V81
handler carries for an account that already had a link; a legacy
account's own start is still refused with `subject_mismatch`.

**Contributor:** the move is offered to an instance-enrolled device and
still not after a pooled refusal; the instance statement round-trips with
the server's verifier.

Red evidence for review: remove the pooled re-check, the tenant-context
check, or the trigger, and the matching test fails.

## 10. Wording

No user-facing text is added (owner decision 9, 2026-11-03). The move and
the payout link use their existing, approved strings. No notice, banner
or status line.

## 11. Alternatives considered

- **(a) Move the legacy account onto a NEAR AI tenant.** Re-keys
  submissions, batches, accruals and credit across tenants against C3,
  rewrites history that the payout spec says is never re-keyed, and breaks
  every hash that names the legacy tenant. Large and irreversible.
- **(b) Record a NEAR AI login on the legacy account and relax
  readiness.** C2 refuses it for anyone who already has a NEAR AI account,
  which is the common case. Where it succeeds, C1 makes every later sign-in
  land in the legacy tenant, and readiness must be weakened for all
  deployments. C4 leaves no session to record it with.
- **(d) Transfer accruals across tenants.** Write a "transfer out" accrual
  in the legacy tenant and a "transfer in" in the NEAR tenant, so one link
  pays everything. Touches the accrual kinds, `owed`, reversals and
  ceilings in the payout core, for a population of a handful of accounts.
  Possible later if the owner wants one payout per person; not needed to
  turn payouts on.

## 12. Owner decisions

**Decided 2026-11-03.** The owner approved every recommendation below as
written ("looks good"): (C) carry; a carried link satisfies "NEAR AI
login only"; a device-signed instance link; carry to all open accounts;
link proven devices' principals, subject to the section 3.5 check; skip
and audit an account with its own link; own ceilings; coexistence credit
is paid; **no new user-facing text at all** (the section 10 line is not
added); a database trigger on revocation; the section 8 rollout.

1. **Approach.** (C) carry the NEAR AI account's payout link to its
   linked legacy accounts (this spec); (a) move accounts across tenants;
   (b) in-place anchor plus a readiness change; (d) cross-tenant accrual
   transfer. **Recommended: (C).**
2. **Does a carried link satisfy "NEAR AI login only"?** The login proved
   is the NEAR AI account's recorded anchor; the legacy account is reached
   through the device-signed V81 tenant link. **Recommended: yes.**
3. **Instance-enrolled tenants** (likely the pilot accounts). (i) a
   device-signed sibling of the V81 link with no invite grant, offered in
   the app; (ii) an operator-attested link; (iii) leave them out.
   **Recommended: (i)**, with an operator-attested link only for a tenant
   whose device key is lost, approved by the owner case by case.
4. **Several open accounts in one non-pooled legacy tenant.** Carry to all
   (a non-pooled tenant is one person, V81's rule), or only to the account
   of the device that signed. **Recommended: all.**
5. **Credit on a principal with no account.** Link the signing device's
   principal (and later attested devices) to an account at link time so
   its unowned accruals are claimed. **Recommended: yes, proven devices
   only**, subject to the principal check in section 3.5.
   **Decided 2026-11-03: option (A).** The check failed for the device
   principal; the owner chose to link the device's signed-upload
   principals of its bare actor instead. Per-user-subject credit stays
   unowned, a known limit (section 3.5) measured by Q5.
6. **A legacy account that already has its own live link** (none expected,
   Q4). Skip and audit, or replace with the carry. **Recommended: skip and
   audit; the operator decides.**
7. **Ceilings.** Each account keeps its own per-account ceiling, or the
   person's accounts share one. **Recommended: own ceilings for now**;
   revisit with (d) if needed.
8. **Coexistence.** Credit a legacy device keeps earning in its legacy
   tenant after linking is paid through the carry. **Recommended: yes.**
9. **Wording.** No new text, or the one DRAFT status line in section 10.
   **Recommended: no new text** for the first release.
10. **V81 revocation releases carries** by a database trigger or by a
    runbook step. **Recommended: the trigger**, so a forgotten step cannot
    leave a revoked tenant being paid.
11. **Rollout.** Staging, then the pilot with the owner's tenant first,
    with production payouts off throughout (section 8). **Recommended: as
    written.**

## 13. Implementation notes (2026-11-03)

Where the implementation differs from, or adds to, sections 3 to 5:

- **The carry runs in the payout store, not in a definer function.**
  `crates/trace-commons-server/src/db/postgres_legacy_payout_carry.rs`,
  called from `confirm_payout_link_request` in the confirm's transaction
  for both a link and an unlink. It reads the account's tenant links under
  the account's own tenant context, then sets the context to each legacy
  tenant in turn and restores it, so forced RLS applies to every statement.
  The rail's program for a carried link's own id comes in through
  `PayoutLinkConfirmWrite::carry_programs`. The guard role
  `trace_legacy_payout_carry_guard` owns only the decision-5 claim and the
  revocation trigger, which run under logins that cannot otherwise reach a
  legacy tenant.
- **After a new tenant link**, the ingest route calls
  `PayoutStore::carry_legacy_payout_links` for each enabled rail, in its
  own transaction. Best effort: a failure carries nothing (no one is paid
  wrongly), is logged as `legacy_payout_carry_failed`, and the next confirm
  carries again.
- **Release.** `trace_legacy_payout_release_carry` (SECURITY INVOKER,
  shared by the store and the trigger): an active carried link gets an
  unlink request; a waiting one, which never sent, closes at
  `final_paid = 0`. V146 widens the link guard trigger for exactly that
  move (waiting to closed, at zero, with no line), so a released carry
  never pays once before its unlink takes effect. A superseded active carry
  (a recipient change) is only marked released; the pass closes it as any
  change.
- **Audit.** Carried links: `link_created` / `legacy_tenant_link`.
  Releases: `link_closed` or `unlink_requested` /
  `legacy_tenant_link_released`. Skipped accounts (decision 6) are counted
  in the NEAR AI tenant's `legacy_payout_carried` account audit row and
  logged as `legacy_carry_skipped_own_link`; no payout audit row, since
  nothing changed on a link. The claim writes `legacy_link_principals_claimed`
  in the legacy tenant. Actor refs are hashes.
- **Instance links** (decision 3) go through the existing route: when
  `trace_link_legacy_invite` answers `device_not_eligible`, the store calls
  `trace_link_legacy_instance_device` in the same transaction, which takes
  the challenge that function spent. The response's `authority` is the
  account's current one (`none` without a trust row) and its trust version
  is unchanged. Only the device that linked an instance tenant is proven: a
  second device of that tenant is refused, not attested.
- **Tests.** `crates/trace-commons-server/tests/legacy_payout_carry_pg.rs`
  (CI step `legacy_payout_carry_pg`) and one ingest route test,
  `a_confirmed_link_reaches_a_linked_legacy_account_and_opens_to_the_same_org`.

### Changes after review (2026-11-03, #1865)

- **A superseded active carry is unlinked** at the change, not only marked
  released, so a revocation, unlink or pooled mark before the new carry
  activates cannot leave the old one paying.
- **The carry takes the tenant link's lock** (`trace_legacy_invite_link:`
  per legacy tenant) and re-reads `revoked_at` under it; the revocation
  trigger takes the same lock. A carry and a revocation can no longer
  interleave into a live carry on a revoked link.
- **Send-time check.** `trace_legacy_payout_carry_invalid` (guard-owned)
  runs inside `payout_account_standing`, which the pass checks before every
  send. A live carry is held (`payout_legacy_carry_invalid`) when its tenant
  is pooled, its tenant link is revoked or deleted, the NEAR AI account is
  closed or has an open account credit hold, or the source link is no
  longer live. Section 7's exclusion now holds where money moves.
- **Only a carried waiting link** may close at zero; every other link
  keeps V132's rule.
- **Audit actors are re-hashed** under `legacy-payout-carry-actor-v1:` on
  the legacy tenant's rows, as section 5 says.
- **Decision 5 claims the subject-form principals** (section 3.5).
- **Instance links require the tenant's earliest live device.** A device
  enrolled into the tenant later, for example by whoever holds the
  instance key, cannot link it. If every earlier device was revoked, a
  later one becomes earliest. No operator approval (owner, 2026-11-03).
- **The app does not offer the move to instance-enrolled devices yet.**
  The existing offer and notice say "invite", which is untrue for them, and
  showing them needs approved wording (copy guide rule 9). The server
  accepts instance links; the client offer waits for that wording.

### Owner decisions after review (2026-11-03)

1. **Decision 5 confirmed with the corrected premise.** The claim links
   the bare device key, the app's `user_subject_hash(device_key_id)` and
   the instance-enrollment subjects. Contributor-label subjects stay
   unowned, measured by Q5.
2. **Instance links: the tenant's earliest live device signs, with no
   operator approval.** An operator is involved only for a lost device key,
   case by case.
3. **No wording yet for offering the move to instance devices.** The
   client offer stays limited to invite devices. Follow-up: wording for
   instance devices, then widen `offered_for`.

### Changes after the second review (2026-11-03, #1865)

- **The send-time check reads the source account's standing as broadly as
  `payout_account_standing` does:** a credit hold on its `account:` key in
  any spelling settlement accepts (case, braces, compact, `urn:uuid:`), on a
  principal it owns, or matched through its accruals, and an open payout
  hold on the carried rail.
- **The check refuses misuse** (`42501` under another tenant's context)
  instead of answering "valid".
- **Legacy-side audit actors** derive from the carried link's own random
  id, so they cannot be joined to the NEAR AI tenant or across legacy
  tenants.
- **Runbook:** revoke one tenant link per transaction (a multi-row revoke
  can deadlock with a concurrent carry; PostgreSQL aborts one side).

### Owner decisions on #1879 (2026-11-03)

1. **A credit hold on the NEAR AI account pauses its carried payouts and
   does not release the carry.** The send-time standing check holds
   payment while the hold lasts, and payment resumes when the hold lifts.
   A close, a pooled mark, a revocation and a tenant-link delete still
   release.
2. **Carry only at events, never on every pass, and only to proven
   accounts.** The events are the NEAR AI account's confirm, the tenant
   link, and an account opening in a linked legacy tenant (the login-link
   path), each in its own transaction. The payout pass does not carry. At
   every event the carry reaches only accounts whose principals a signed
   device proved: the linking device or a V91-attested one
   (`trace_legacy_payout_proven_accounts`, V148, a join over what the
   claims recorded; see decision 3). Only devices still live
   in the tenant prove (unrevoked, `invite` origin, not NEAR-provisioned).
   An account opened by an unproven device waits. A live carry on an
   account that stops qualifying is released at send time (decision 6);
   so is any V146-era carry on an unproven account, at the first pass or
   carry event after deploy, which the pre-deploy checks count.
3. **Instance-enrollment subjects prove only if enrolled by the link**
   (owner, 2026-11-03, review of #1879 at V148:253). A subject counts
   toward the proof set only if it was enrolled no later than the tenant
   link (for the linking device) or the V91 attestation (for an attested
   device). The claim records the principals it links, once per device
   under the tenant link (`trace_legacy_link_claimed_principals`, V148),
   and reuses that record on any repeat; the proof set is a join over those
   records, so the claim and the proof cannot differ and no two clocks are
   compared (review of #1879). An account under a subject the instance key
   enrolls after the link is not proven and is not carried. Links made
   before V148 are recorded once by V148, bounded by the stored link and
   attestation times.
4. **A legacy account whose own payout link V147 (#1868) revoked** gets its
   carry at the next of those events, if it is proven, as the waiting
   successor of the revoked link. The pilot had 0 links on 2026-11-02;
   the pre-deploy count below confirms it.
5. **Carry failures never fail a sign-in** (review of #1879). The
   account-opening carry runs in a savepoint of the login-link mint, for
   the opened account only and only when the call opened it; a failure is
   logged by class and carries nothing.
6. **A carry whose account stops being proven is released at send time**
   (owner, 2026-11-03, review of #1879 at V148:444). Before reading a
   legacy tenant's links, the payout pass calls
   `trace_legacy_payout_release_unproven_carries`: under the tenant-link
   lock, every live carry on an account no longer proven (its proving
   principal unlinked, its device revoked) is released, an active one with
   an unlink request and a waiting one closed at zero, audited hash-only as
   `legacy_carry_unproven`. The account's standing then no longer holds it,
   so its other streams resume. The pass only ends carries; it never
   creates one. `trace_legacy_payout_carry_unproven` stays in the standing
   as the backstop if the release fails.

### Known limits, not fixed here

- **Accounts created after a carry by a later login link** are carried
  only at the NEAR AI account's next confirm or tenant link. Their credit
  accrues but is not paid until then. (A claim at a V91 attestation goes
  through the tenant-link route, which carries after it.) **Fixed by V148
  (#1873):** opening an account in a legacy tenant linked through V81 (the
  login-link path, `PayoutStore::create_or_reuse_account_carrying`)
  carries the NEAR AI account's live link to it in the same transaction,
  through the guard-owned `trace_legacy_payout_source_account`. The
  payout pass never carries. A closed source account carries nothing.
- **An invalid carry is held, not released.** A pooled mark, a closed or
  held NEAR AI account, or a deleted tenant link holds the legacy
  account's sends at once (fail closed), but the carried link is released
  only at the next carry or a revocation. The hold is per account, so it
  also pauses that account's referral stream. **Fixed by V148 (#1872):**
  a pooled mark, an account closing (a legacy account's own carries, or a
  NEAR AI account's carries in every tenant it linked) and a tenant link
  deleted rather than revoked each release the carries they invalidate, in
  their own transaction, under the tenant link's lock, audited as
  `legacy_tenant_pooled`, `legacy_account_closed`,
  `legacy_source_account_closed` and `legacy_tenant_link_deleted`. The
  send-time check stays as the backstop and still holds a live carry while
  its source account is on a credit hold or its source link is unlinked
  outside a confirm. A credit hold pauses and does not release (owner
  decision below).
- **The hash formulas** (`legacy-invite-link-v1:`, `payout-link-v1:`,
  `payout-account-v1:`) are written inline in V146 and mirrored in Rust;
  the tests catch a drift that breaks a match. Follow-up, #1874: small
  `IMMUTABLE` SQL helpers.
- **Links proved before #1862.** The carry requires the source account to
  hold a `near_ai_login` anchor, not that the source link was proved by
  one. A link proved through a wallet anchor before #1862 would be carried
  once that account records a login. Covered by the rollout gate below
  (owner, 2026-11-03).
- **Offering the move to instance-enrolled devices** waits for approved
  wording (owner decision 3 after review). Tracked in #1875.

### Production-payout gates (owner, 2026-11-03)

Four read-only checks run on the pilot, and the owner reviews their
results, before the `near_ai` rail goes `live` for production. They sit
beside F10 and F11 in the settlement runbook's "Before production payouts"
(`docs/operator/automatic-credit-settlement.md`).

1. **Source provenance.** A read-only count of live payout links that may
   have been proved by a wallet anchor before #1862: links created before
   #1862's deploy, and links whose request audit row is labelled
   `near_wallet` (#1841). Any it finds are handled by #1868's V147, which
   revokes payout links that have no recorded NEAR AI login, at deploy;
   the check is re-run after V147 and must then find none on accounts
   with a live tenant link.
2. **Legacy own links (V147).** Q4 (live payout links in legacy tenants)
   is re-run before deploy and reported with the source-provenance count.
   Each such link is revoked by V147, and its account gets a carry at the
   next carry event only if a signed device proved it. Expected 0 (pilot,
   2026-11-02).
3. **Carries on unproven accounts.** A count of live carries whose
   account no signed device proves (the query is in the settlement
   runbook). The first carry event after deploy releases each. Expected 0;
   if not, stop and ask the owner.
4. **Q5 is run and reviewed.** Q5 (section 8) must be run and its result
   reviewed by the owner. It need not come back zero: credit from
   contributor-label subjects stays unowned and unpaid by design
   (section 3.5).


## 14. Follow-ups outside this PR

- The 2026-10-21 payout spec, section 5, "Who can link", still has the row
  "account in a legacy or pooled tenant: as the wallet row", and section 6
  says the NEAR AI proof replaces the V81 link for legacy tenants. Both are
  out of date after #1841 and the gate branch; correct them when this is
  decided.
- The one-connected-account spec's F11 row names the two candidates of
  section 11; point it here once decided.
