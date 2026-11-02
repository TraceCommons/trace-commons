# Changing the NEAR AI login a Trace Commons account uses

Status: DRAFT design, for owner review. Nothing here is implemented.
Date: 2026-11-02.

## Owner direction

> "It would be good if the app could remember what account is associated
> with tracecommons and then encourage users to sign in again with that
> account and then have a flow if the user wants to change the account."
> -- Zaki, 2026-11-02

The app now offers every NEAR AI cloud login (NEAR wallet, GitHub, Google;
`channel::NEAR_AI_SIGN_IN_PROVIDERS`,
`crates/trace-commons-contributor/src/channel.rs:114`), chosen on a browser
page the daemon serves, with near.com OAuth expected later. Each of those is
a separate NEAR AI identity unless NEAR AI itself links them.

### The field case

The owner enrolled with the NEAR AI account behind the NEAR wallet login,
then later chose GitHub, which is a different NEAR AI account. The NEAR AI
sign-in succeeded; the Commons renewal that follows it was refused
(`account-enrollment-mismatch`,
`crates/trace-commons-contributor/src/daemon/nearai_onboarding.rs:898`), and
the app showed "Sign-in didn't finish." PR #1833's body traces the sequence
and its side effect: the browser leg had already replaced the stored NEAR AI
session and inference key with ones minted in the GitHub account.

Two pieces of work in flight cover the immediate defect and are assumed by
this spec rather than repeated:

1. A browser chooser page that marks the login the Commons account uses.
2. A daemon change so that a login the Commons refuses does not replace
   working credentials.

This spec covers what neither does: a deliberate flow that moves a Trace
Commons account from one NEAR AI identity to another.

## 1. The binding today

### 1.1 Server

**A NEAR AI identity is an anchor, and a new anchor is a new tenant.** This
one fact decides most of the design.

- NEAR AI's `GET /v1/users/me` is read into `IntrospectedUser { id,
  auth_provider }` and nothing else
  (`crates/trace-commons-server/src/near_ai_login.rs:75`). `id` is the
  subject; it is never stored. What is stored is a blind index over it, the
  anchor (`NearAiAnchorMaterial::for_login`, called at
  `crates/trace-commons-server/src/db/postgres_account_onboarding.rs:567`).
- `trace_near_account_anchors` (`migrations/V58__near_account_provisioning.sql:17-25`)
  maps anchor to `(tenant_id, account_id)`. Keys: `PRIMARY KEY (tenant_id,
  anchor_hash)`, `UNIQUE (anchor_hash)`, `UNIQUE (tenant_id, account_id,
  anchor_hash)`. V62 dropped the check that tied the tenant id to the anchor
  (`migrations/V62__near_account_anchor_tenant_binding_drop.sql`). V63 added
  `identity_source` (`wallet` | `near_ai_login`) and `auth_provider`
  (`migrations/V63__near_ai_login_provisioning.sql`). Forced RLS on
  `tenant_id = trace_current_tenant_id()`.
- Sign-in and sign-up are one route pair, `POST
  /v1/account/near-ai/provision/{start,finish}/v2`
  (`crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:9002`,
  `:9010`). `near_ai_finish`
  (`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/near_provisioning.rs:1386-1462`)
  takes the single-use ceremony, verifies the device's signature, introspects
  the NEAR AI token, then calls `provision_near_ai_login`.
- `provision_against_anchor` (`postgres_account_onboarding.rs:414-444`) asks
  `near_anchor_tenant` (`:336-348`, the login-resolver pool, the one
  cross-tenant read) which tenant holds the anchor. A known anchor keeps its
  tenant; an unknown one gets a fresh `nearai-<random>` tenant
  (`random_near_ai_tenant_id`,
  `crates/trace-commons-server/src/near_account_identity.rs:499-503`).
- In that tenant, `near_ai_login_provision_in_tenant` (`:594`) looks the
  anchor up among open accounts (`:619`), creates an account and claims the
  anchor if there is none (`:620-637`), attaches the device and its session
  (`attach_near_ai_login_device`, `:207-231`), and writes a hash-only
  `near_ai_login_provisioned` audit row (`:651`).
- A device key belongs to one tenant: `claim_provisioned_device_key`
  (`:42-80`) refuses a key registered under another tenant with
  `DEVICE_KEY_REGISTERED_ELSEWHERE`, and `near_ai_finish` turns that into
  the uniform deny (`near_provisioning.rs:1444`). So a device enrolled under
  identity A that signs in with identity B gets nothing; the transaction,
  including any tenant and account it minted, rolls back.

Everything the account owns is keyed by `(tenant_id, account_id)`, not by
anchor: traces, credit, referrals (`referral_routes`), inference-connection
choice (`migrations/V79__inference_connection.sql`), devices and sessions
(`migrations/V139__account_devices.sql`), payout links, accruals and lines
(`migrations/V132__payout_rails.sql:97-257`). The anchor is read in only a
few places:

| Reader | Where | What it asks |
| --- | --- | --- |
| tenant lookup | `postgres_account_onboarding.rs:336` | which tenant holds this anchor |
| account lookup | `postgres_account_onboarding.rs:493` (wallet), `:619` (login) | which open account holds this anchor |
| payout proof | `db/postgres_payout.rs:1420-1436`, used at `payout_link.rs:284` | the account's `near_ai_login` anchors, which a payout proof's subject must match (`payout_recipient_proof.rs:114-124`) |
| strong authenticators | `db/postgres.rs:2010-2036` | whether the account has any `near_ai_login` anchor |
| admission readiness | `near_provisioned_row_for_principal`, `postgres_account_onboarding.rs:713` | the device's anchor and account, through `trace_near_provisioned_devices` |

**Several anchors per account is already the model.** V82's header
(`migrations/V82__near_account_merge.sql:9-11`) states it: "A survivor that
already holds an anchor ends up holding both. Every reader of the table asks
whether an account has an anchor, or which account an anchor belongs to;
none assumes one anchor per account." `active_strong_authenticator_count`
says "one however many a merge left", and `payout_account_login_anchors`
returns a `Vec`.

**Merges do not cross tenants.** `trace_near_account_merge` refuses unless
`p_tenant` is the caller's tenant (`V82__near_account_merge.sql:59`). Two
NEAR AI identities that each signed up are two tenants, and nothing today
joins them.

**Passkeys are gone.** V144 removed them on the owner's 2026-10-31 decision
(`migrations/V144__remove_passkeys.sql:1-3`), along with
`trace_account_bindings`. A passkey cannot be the second factor here. The
Commons-native factors that remain are the enrolled device key and its
native session (`renew_account_session_by_device_key`,
`nearai_onboarding.rs:1002`; devices list and revoke, V139; join by code,
V140).

**`auth_provider` is recorded, not read.** No code outside the insert reads
it (grep of `crates/`, 2026-11-02). Changing it on a rebind changes no
decision today.

### 1.2 Client

- The Commons record holds the session and the account id. A renewal
  (`renew_account_session`, `nearai_onboarding.rs:818-866`) spends the
  stored NEAR AI login and requires the answer to name the enrolled tenant
  and the stored `account_id` (`store_renewed_session`, `:883-917`).
  Otherwise `account-enrollment-mismatch`, classed `commons_answer_mismatch`
  (`account_session_renewal.rs:513`).
- The NEAR AI side is one slot: `DaemonSettings.near_ai_session` (a refresh
  token, its expiry, the creating User-Agent;
  `crates/trace-commons-contributor/src/daemon/settings.rs:295-330`, `:494`)
  and `near_ai_inference` (the minted inference key, `:478`).
- **Nothing on the client records which NEAR AI identity the Commons
  account uses.** The session carries no subject and no provider label.
  The daemon cannot tell, before asking the Commons, whether a new login is
  the same identity. PR #1833 infers "different account" after the fact
  from the organization a minted key lands in.

## 2. Threat model

Assets: the Trace Commons account (traces, credit, referral standing, payout
destination), the device's enrolled key, and the NEAR AI keys the daemon
minted.

| # | Threat | What must hold |
| --- | --- | --- |
| T1 | Someone holding only a NEAR AI identity (new or stolen) captures an existing account. | A NEAR AI login alone never moves an account. Today it cannot: an unknown anchor makes a new tenant. A rebind must keep that. |
| T2 | Someone holding a live native session or the device key (a stolen laptop, malware on it) rebinds the account to their own identity and retires the owner's, locking the owner out and then relinking payouts. | The rebind requires a fresh, interactive sign-in to the current identity, not only the device. The device's stored NEAR AI refresh token does not count: it lives in the same keychain as the device key, so a thief with the device holds it too, and a silent refresh would reduce (a) to (b). This is the main reason for option (a) and for its step 1 (section 3.7). |
| T3 | The old NEAR AI login is stolen or expired. | Expired: no effect, since step 1 is a new sign-in anyway. Stolen (the NEAR AI login alone): the thief has no device or native session, and gets nothing from the old identity alone (T1). Stolen together with the device: the thief can pass step 1 and move the account. A cooldown with a revert window (Q6) is the remaining defence. |
| T4 | The owner lost the old identity (lost wallet, closed GitHub). | No self-service path in the first slice; see option (d). Anything weaker than option (a) is a takeover path for T2, so it needs delay and wide revocation, not just a different check. |
| T5 | Sybil laundering: rebind to free an identity, then sign that identity up again as a fresh account to reclaim per-identity sign-up grants or admission. | A retired anchor keeps its row, so `UNIQUE (anchor_hash)` keeps it from claiming a new account. |
| T6 | Two accounts merged by the back door: rebind account A onto an identity that already has account B. | Refused. The new anchor must be unclaimed. Merging across tenants is out of scope. |
| T7 | Payout redirection. | Covered by T2. Existing payout links stay as they are (they pay an org the owner proved); new links verify against live anchors only. |
| T8 | Keys minted in the old NEAR AI account keep working after the move. | The daemon revokes them with the old session during the ceremony; a failed revoke stays pending. The Commons holds no NEAR AI key and has nothing to revoke there. |
| T9 | Audit or logs reveal which identities a person holds. | Audit rows are label-only: no subject, no anchor pair in one row, no provider label (it narrows who the person is). See section 3.6. |

Out of scope: NEAR AI's own account linking. If NEAR AI later links GitHub
and the wallet into one subject, `users/me` returns one `id` and this flow
is never needed for that pair.

## 3. Options

### (a) Prove both identities in one ceremony, then rebind (recommended)

The signed-in device starts a ceremony; the person signs in again,
interactively, with the current identity, and the daemon presents that
token and one for the new identity; the server checks
the first resolves to this account's live anchor and the second to no
anchor, then adds the new anchor and retires the old one in one
transaction. Tenant and account id do not change, so traces, credit,
referrals, payouts and devices stay where they are, and the client's
mismatch check (`nearai_onboarding.rs:898`) passes unchanged: it compares
tenant and account only, and `validate_finished` (`:760-778`) checks the
answer's `anchor_hash` for shape, never against a stored value.

- Pros: defeats T1, and T2 short of a thief who can also complete a NEAR
  AI sign-in as the owner; no data moves; reuses the provisioning
  ceremony's shape (single-use row, device signature, server-side
  introspection); the schema already allows two anchors.
- Cons: needs the old identity to be reachable now (T4 is not served); two
  NEAR AI tokens on the client at once.

### (b) Rebind with a Commons-native factor plus the new login

The original idea used a passkey. Passkeys were removed (V144). The
remaining native factor is the device key and its session, which is exactly
what T2's attacker holds. As a sole factor this is a takeover path. It is
useful only as the base of (d), with a delay.

### (c) Link several NEAR AI identities to one account

Add identities rather than switch; any linked identity signs in.

- Pros: no "which one" problem; the field case would just work after one
  link step.
- Cons: the account is as weak as its weakest linked login; the payout
  subject check accepts any of them; unlinking needs the same ceremony as
  (a) anyway. It changes the product promise ("your account is your NEAR
  AI login") and the copy everywhere.

The mechanism of (a) is (c)'s add step followed by a retire step in the
same transaction. Offering (c) later is a policy change on the same schema.
Recommendation: do not expose (c) now.

### (d) Recovery when the old identity is gone

Shape only, for the owner to decide whether to build:

- Start from a signed-in device (device key + live session) plus the new
  login.
- The server records a pending rebind and does not apply it for a delay
  (for example 72 hours).
- Any session of the account, or a sign-in with the current identity,
  cancels it.
- On apply: rebind as in (a), revoke every other session and device
  (`revoke-all`), and close active payout links so the owner relinks under
  the new identity.
- Telling the owner a rebind is pending is a notice; it needs owner
  approval (section 5.4).

An operator-assisted path (support verifies out of band and runs an admin
route) is the alternative. It adds an admin credential scope and a drill.

### Comparison

| | T1 | T2 | T4 served | Data moves | Server size |
| --- | --- | --- | --- | --- | --- |
| (a) both identities | yes | yes, with a fresh sign-in to the current login | no | no | one route pair, one migration |
| (b) device + new | yes | **no** | yes | no | same as (a) |
| (c) multi-link | yes | yes, if link needs (a) | partly | no | (a) plus unlink |
| (d) delayed recovery | yes | delayed, cancellable | yes | no | (a) plus pending state, scheduler |

### 3.5 Server changes for (a)

**Migration** (next free number; V145 is claimed by open PR #1826, so this
is V146 at the earliest; check the `PgBackend` migration list at
implementation time):

```sql
ALTER TABLE trace_near_account_anchors
    ADD COLUMN retired_at TIMESTAMPTZ;
GRANT UPDATE (retired_at) ON trace_near_account_anchors TO trace_ingest_runtime;
-- The login resolver's cross-tenant read must see the column it filters on.
-- How the resolver currently reads this table (grant or definer function)
-- decides the exact form; see docs/operator/login-resolver-role.md.
GRANT SELECT (retired_at) ON trace_near_account_anchors TO trace_login_resolver;
-- Re-keying provisioned devices onto the new anchor (below).
GRANT UPDATE (anchor_hash) ON trace_near_provisioned_devices TO trace_ingest_runtime;
```

- The old anchor row is never deleted: a retired row is what keeps the old
  identity from claiming a fresh account (T5).
- **Provisioned devices are re-keyed to the new anchor in the rebind
  transaction**, as V82 does for merges. Without it they would point at the
  retired anchor forever: `attach_near_ai_login_device` inserts its row
  with `ON CONFLICT (tenant_id, principal_ref) DO NOTHING`
  (`postgres_account_onboarding.rs:228`), so a later sign-in never updates
  it. And `near_provisioned_row_for_principal` (`:713`) is what admission
  reads as the account's anchor (`admission::anchor`,
  `trace_commons_ingest_internal/admission.rs:246`). Admission reservations
  and per-anchor limits are keyed by that anchor (`AdmissionReservation`,
  `admission.rs:638-646`). Left alone, existing devices would count
  against the retired identity and new devices against the new one: two
  admission windows for one account, and a retired identity still
  granting admission. Re-keying gives one window. The cost is that the
  window restarts once per move; a cooldown between moves (Q6) bounds
  that. The foreign key to `(tenant_id, account_id, anchor_hash)`
  (V58:55) is satisfied because the new anchor row is inserted first. The
  rebind refuses while an admission reservation for the old anchor is
  in progress (`AdmissionDecision::Busy`), so no lease straddles the move.
- Column-scoped grants, V100's pattern: a compromised runtime can retire or
  revive an anchor of its own tenant and point its own devices at its own
  anchors (the foreign key holds), but cannot re-key an anchor.
- Forced RLS is unchanged. Every write runs in the account's own tenant.

**Readers to update** (each must treat a retired anchor as not an identity
of the account):

| Reader | Change |
| --- | --- |
| `near_anchor_tenant` (`:336`) | Return the tenant with a `retired` flag, so the caller can refuse by name rather than mint a tenant. |
| account lookup (`:619`, and the wallet twin `:493`) | A retired anchor resolves to "retired", not to the account: the old identity can no longer sign in to the account it left. `near_ai_finish` answers a named `near_ai_login_retired` (409, `no-store`), shown only to someone who just proved that identity. |
| `payout_account_login_anchors` (`postgres_payout.rs:1420`) | `AND retired_at IS NULL`. A retired identity cannot link a new payout org. |
| `active_strong_authenticator_count` (`postgres.rs:2010`) | `AND retired_at IS NULL`. The count stays 1 after a rebind because the new anchor is live. |

**Routes** (native session plus device signature, the shape of the
provisioning pair):

- `POST /v1/account/near-ai/rebind/start` -- authenticated by the native
  session; body: device public key and a PKCE challenge. Stores a
  single-use ceremony (the `trace_near_provisioning_ceremonies` table, V58,
  keyed and RLS-guarded by ceremony hash) carrying the session's tenant,
  account and device. Answers the nonce and expiry.
- `POST /v1/account/near-ai/rebind/finish` -- body: `ceremony_id`, the
  device signature over the nonce, ceremony id, PKCE verifier and both
  token digests, `current_access_token`, `new_access_token`. Steps, refusing
  with the uniform deny at each:
  1. Take the ceremony (single use); check it names this session's tenant,
     account and device; verify the device signature.
  2. Introspect both tokens with `introspect_login`
     (`near_ai_login.rs:141`). Two calls to NEAR AI; both bounded by the
     existing timeout.
  3. Anchor both. `current` must be a live anchor of this account. `new`
     must differ from `current`.
  4. In one transaction in the account's tenant: advisory locks on both
     anchors in sorted order (the provisioning lock is
     `pg_advisory_xact_lock(hashtextextended(anchor, 0))`, `:610`, so a
     concurrent sign-up with the new identity serializes against this);
     insert the new anchor row (`identity_source = 'near_ai_login'`, its
     `auth_provider`, the account's tenant) with `ON CONFLICT (anchor_hash)
     DO NOTHING`. Zero rows means the new identity already has an account:
     roll back and answer `near_ai_login_in_use` (409). If the new anchor
     exists as a retired anchor of this same account, clear its
     `retired_at` instead (switching back). Then set the old anchor's
     `retired_at`, re-key the account's `trace_near_provisioned_devices`
     rows from the old anchor to the new one, write the audit row, commit.
  5. Answer `{account_id, tenant_id, anchor_hash, auth_provider}` (the new
     anchor's). The current session stays valid; no new session is minted.
- Neither token is stored, logged or put into an error; both are dropped
  when introspection returns, as in `near_ai_finish`.
- Rate-limited under its own action labels (`limited(&headers,
  "near-ai-rebind-start")`, `"near-ai-rebind-finish"`).
- Published in `capabilities_v2` (`near_provisioning.rs:421`) as
  `near_ai_rebind_start_path` / `near_ai_rebind_finish_path`, so a client
  offers the flow only to a server that has it. Fail closed: with the NEAR
  AI login path not ready (`near_ai_login_ready`), the routes deny.

Optional, owner's call (Q4): revoke every other session of the account in
the same transaction, as the passkey-removal migration did for passkey
sessions.

**Sign-in answer.** `near_ai_finish` adds `auth_provider` (the stored,
normalised label of the anchor that signed in) to its answer. It is the
person's own fact, already stored, and it is what lets the app remember the
login (section 3.7) without reading identity data the server deliberately
drops.

### 3.6 Audit

One row, `trace_account_audit`, action `near_ai_login_rebound`, actor the
device principal, outcome `success`, `safe_metadata`
`{"identity":"near_ai_login","switched_back":false}`. No anchor hashes (a
pair of anchors in one row links two identities), no provider labels, no
subject. Refusals are counted by label in logs only (`near_ai_login_in_use`,
`near_ai_login_retired`, `uniform_deny`), never with an account or tenant
id. The anchors table itself holds the mapping, under RLS, which is where
it already lives.

### 3.7 Client changes for (a)

**Remember the login (daemon).** On every successful Commons sign-in or
renewal, store beside the Commons record:

- `near_ai_provider`: the `auth_provider` the server answered (`near`,
  `github`, `google`, `unknown`).
- `near_ai_subject_digest`: a keyed digest of the NEAR AI subject id, read
  by the daemon from `GET /v1/users/me` with the token it already holds,
  keyed by a per-install secret kept with the Commons record. Local only,
  never sent, never logged. It answers "is this login the same NEAR AI
  account?" before anything is replaced or sent to the Commons. The digest
  is a convenience; the Commons is still the authority.

Exposed over IPC as part of the account status: `commons_login: {provider}`
and, after a browser sign-in, `login_matches_commons: true | false |
unknown`. The chooser page marks the remembered provider; this spec does
not change how the chooser renders that mark.

**Hold a non-matching login aside.** When a browser sign-in completes and
`login_matches_commons` is false, the daemon keeps the new NEAR AI session
in a staging slot (`near_ai_session_pending`) and leaves `near_ai_session`
and `near_ai_inference` alone. This is the in-flight change (2) from the
top of this spec, extended with a slot the change flow can use.

**Change flow (one IPC verb).** `near_ai_account_change { confirm: true }`:

1. Ask the person to sign in with the current login: the chooser page
   opens with only the remembered provider offered. That sign-in's access
   token is held in memory for this ceremony only and never stored. Its
   subject digest must match the remembered one, or the flow ends with
   `current_login_required`. This is the owner's "sign in again with that
   account", and it is what makes (a) stronger than (b): silently
   refreshing the stored `near_ai_session` would prove only that the
   device's keychain was readable (T2). The silent refresh is the weaker
   variant, and the owner can choose it (Q1) if the extra sign-in is too
   much friction, accepting T2 short of a revert window.
2. Take `near_ai_session_pending`'s access token. Absent: `new_login_required`.
3. Run rebind start and finish.
4. On success, in order: revoke the inference keys minted in the old NEAR AI
   account with the old session (failures stay as pending revocations);
   promote the pending session to `near_ai_session`; mint an inference key
   in the new account (PR #1833's organization rules apply); store the new
   `near_ai_provider` and subject digest.
5. On `near_ai_login_in_use`, `near_ai_login_retired` or a deny, discard
   nothing: the pending slot is kept until the person signs in again or
   cancels, and the current login keeps working.

Cancel: `near_ai_account_change_cancel` drops the pending slot (and revokes
any key minted in it).

**Shells.** Native macOS is the lead client (CLAUDE.md, #1173 D1); the flow
is built there first, as a sheet from the account section. GTK, Windows and
Tauri match it later. Tauri is frozen on macOS and is low priority. The
C ABI gains the two verbs and the status fields; the duplicated header
must change in both copies.

## 4. Recommendation and first slice

Build option (a), with (c) and (d) deferred. Nothing moves between tenants;
the account keeps its id, its data and its devices, and gains a new login
while the old one retires.

**Slice 1 (minimum):**

1. Server: the `retired_at` migration and grants; the four reader changes;
   the rebind route pair with its capability flags; `auth_provider` in the
   sign-in answer; the audit row. PostgreSQL tests for: rebind success,
   `in_use` with a concurrent sign-up of the new identity, old identity
   refused after rebind, switching back, a retired anchor rejected by the
   payout proof, the strong-authenticator count, devices re-keyed to the
   new anchor (and admission reading it), refusal while an admission
   reservation is in progress, and RLS (a session in tenant A
   cannot rebind B).
2. Daemon: remember provider and subject digest; `login_matches_commons`;
   the pending slot; the change and cancel verbs; FFI.
3. macOS: the sign-in sheet's "use your Trace Commons login" mark (owned by
   the chooser work) and the change sheet below.

**Slice 2:** the other shells. **Later, if the owner wants them:** recovery
(d), multi-link (c), cross-tenant merge for a new identity that already has
an account.

## 5. UX flow and strings

Every string below is **DRAFT** and needs owner approval of wording. Copy
follows `docs/ui-copy-guide.md`: one- or two-word buttons, consequences
beside the action, separate title and button keys.

### 5.1 Signing in again with the remembered login

When the Commons asks for a sign-in, the account section's Sign in uses the
remembered provider and the chooser page marks it.

| Key | DRAFT text |
| --- | --- |
| `account.login.used_by` | Signed in with {provider} |
| `account.sign_in.button` | Sign in |
| `chooser.mark.commons_login` | Your Trace Commons account uses this login |

`{provider}` is NEAR wallet, GitHub or Google. An `unknown` provider shows
no provider line.

### 5.2 After signing in with a different NEAR AI account

The browser sign-in finished, but the login is not the account's. Nothing
was replaced. Shown in the account section where the sign-in was started.

| Key | DRAFT text |
| --- | --- |
| `account.login.mismatch.title` | This is a different NEAR AI account |
| `account.login.mismatch.body` | Your Trace Commons account uses {provider}. Sign in with {provider} to keep using it, or move your account to the login you just used. |
| `account.login.mismatch.sign_in` | Sign in |
| `account.login.mismatch.change` | Move account |
| `account.login.mismatch.cancel` | Cancel |

### 5.3 The change sheet

| Key | DRAFT text |
| --- | --- |
| `account.change.title` | Move to another NEAR AI login |
| `account.change.body` | Your traces, credit, referrals and payout settings stay with your account. After the move, sign in with {new_provider}. {old_provider} will no longer sign in to this account. |
| `account.change.private_ai` | Private AI will use {new_provider}'s NEAR AI account. Keys made in the old account are removed. |
| `account.change.confirm` | Move |
| `account.change.cancel` | Cancel |
| `account.change.done` | Your account now uses {new_provider} |
| `account.change.error.in_use` | That NEAR AI login already has its own Trace Commons account. |
| `account.change.error.retired` | That NEAR AI login was moved off its Trace Commons account. |
| `account.change.error.current_login` | Sign in with {old_provider} first, to confirm it's you. |
| `account.change.error.unavailable` | Couldn't move your account. Try again. |

Entry points: the mismatch block (5.2), and a "Change" control beside
`account.login.used_by` in account settings (`account.login.change`:
"Change"), which opens the chooser and then the sheet.

### 5.4 Notices this design implies (need explicit approval)

Rule 9 of the copy guide applies. None of these is built until the owner
approves both the notice and its wording.

1. **The mismatch block (5.2).** It appears unprompted after a sign-in. The
   owner's "encourage users to sign in again with that account" asks for
   something like it; that is direction, not approval of this block or its
   words.
2. **`account.change.done`** as a confirmation after the sheet closes. It
   could instead be the updated `account.login.used_by` line, which is not
   a notice.
3. **Recovery (d) only:** telling the account's other devices that a rebind
   is pending. Not part of slice 1.

## 6. Open questions for the owner

1. **Option (a) as the only self-service path?** It needs the old login to
   still work, and the recommendation asks for a fresh interactive
   sign-in with it during the move (not a silent refresh of the stored
   login, which a device thief also holds). Is that friction acceptable?
   Someone who lost the old login (lost wallet) cannot move. Build
   recovery (d) with a delay, offer operator-assisted recovery, or neither
   for now?
2. **Multi-link (c)?** Should an account ever accept more than one NEAR AI
   login at a time, or always exactly one?
3. **A new login that already has its own account.** Slice 1 refuses
   (`near_ai_login_in_use`). Should a later slice merge the two (a
   cross-tenant merge, which does not exist today), or let the person close
   the empty one first?
4. **Other sessions on rebind.** Keep every device signed in (they hold the
   device key, not the NEAR AI login), or sign out all other devices as a
   precaution?
5. **Payout links.** Recommendation: keep an active link through a rebind
   (it pays an org the owner proved). The alternative is to close it and
   ask for a relink under the new login.
6. **Switching back and cooldown.** Allowed in slice 1 (the retired
   anchor is revived). Should there be a cooldown between moves, and a
   revert window in which the retired login can undo a move? Each move
   restarts the account's admission window once, so a cooldown also bounds
   that.
7. **What "remember" shows.** Provider only ("Signed in with GitHub"), or
   also a hint such as a masked email, read locally from NEAR AI and kept
   only on the device? Two GitHub logins look the same with provider only.
8. **The notices in 5.4**, and every DRAFT string in section 5.
9. **The field case.** Before this is built, the owner's
   `account-session-renewal.jsonl` (state directory) shows the failure class
   of the GitHub sign-in. It says whether the GitHub identity already
   provisioned an account somewhere (which slice 1 would refuse as in use).
