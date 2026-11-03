# One connected account: credentials that change together

Status: DRAFT design, for owner review. Nothing here is implemented.
Date: 2026-11-02. Companion to
`2026-11-02-change-nearai-account-design.md` (same PR).

## Owner direction

> "We need to generally make sure that credentials for Private AI sessions,
> settlement, trace commons all work and if they get changed they change
> together." -- Zaki, 2026-11-02

Code references are to `origin/main` at a35c3b1ab (#1833 merged), unless a
branch is named. Paths under `crates/trace-commons-contributor/src/` are
written without that prefix.

## 1. Inventory

"The connected account" is one NEAR AI identity plus the Trace Commons
account bound to it (the change-account spec, section 1.1). The daemon
holds these credentials for it:

| # | Credential | Stored | Written by | Refreshed by | Cleared by | Bound to (checked?) | Missing or stale |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | NEAR AI inference key (`NearAiInferenceCredential`: key, key id, organization, workspace; `daemon/settings.rs:234`) | One OS keychain bundle with #2, service `trace-commons.near-ai.credentials` (`channel.rs:97`); metadata and reference in `daemon-settings.json` `cloud_credentials` (`settings.rs:411`) | `CloudCredentialLifecycle::replace` (`daemon/cloud_credential_lifecycle.rs:402`), called by the sign-in persist (`daemon/nearai_credential/ceremony.rs:731`, `persist_attempt`) | Never rotated; replaced by a new sign-in | NEAR AI forget (`nearai_credential/mod.rs:362` `forget_local` -> `ceremony::forget_locked`, `ceremony.rs:929`, over `cloud_credential_lifecycle.rs:508`), local only: the key is **not** revoked at NEAR AI | The NEAR AI identity and organization it was minted in. Not checked against the Commons account on main; #1837 adds the check at sign-in | Private AI has no key; the shell shows Sign in |
| 2 | NEAR AI refresh session (`NearAiSession`: refresh token, expiry, creating User-Agent; `settings.rs:295`) | Same bundle as #1 | Same as #1 | `session::exchange` (`nearai_credential/session.rs:123`) rotates the refresh token for the same login | Same as #1 | The NEAR AI identity. Not checked against the Commons account on main | Balance and payout linking fail; the Commons renewal fails `near_ai_sign_in_required` |
| 3 | Spend-organization choice and pending key revocations | `near-ai-spend-org.json` (`nearai_credential/spend_org.rs:51`, `pending_revocations` `:87`) | The organization chooser; the sign-in (#1833 `forget_choice_left_behind`) | -- | Nothing clears the file: not forget, not unenroll, not logout (it is in neither `ENROLLMENT_FILES`, `config.rs:121`, nor `wipe_files`, `config.rs:1557-1579`) | An organization id of whichever NEAR AI identity chose it. Pending revocations are swept with the *current* session, so revocations from another identity can never succeed (#1833 body, follow-up) | A stale choice prompts again (#1833); stale revocations stay `revoke_pending` |
| 4 | Remembered login (`stored_login`, `commons_login`) | `near-ai-login.json` (#1835, `nearai_credential/login_record.rs:33` on `claude/nearai-browser-login-chooser`; not on main) | Sign-in (`record_stored_login`), Commons acceptance (`accepted_by_commons`, reading the accepted login from the attempt's progress object) | -- | An enrollment file in #1835, so logout and unenroll remove it | A provider tag only, no subject. `stored_login` is read with the session's key (`stored_login(store, key_id)`): a tag from another sign-in, or after a spend-org switch, reads as unknown | The chooser shows no mark |
| 5 | Commons account session and record (access token, expiry, `account_id`, origin) | `account-session.json` record plus OS entry, service `trace-commons.account.credentials` (`channel.rs:98`; `daemon/commons_credentials.rs:22-49`, `Kind::Account`) | NEAR AI enrollment (`nearai_onboarding.rs`), renewal `store_renewed_session` (`nearai_onboarding.rs:883`) | `renew_account_session` (`:818`) with the stored NEAR AI login (#2); `renew_account_session_by_device_key` (`:1002`) for a device that joined by code | Native sign-out (`daemon/native_identity.rs:127`, Account only); unenroll and logout (all kinds) | Its record is bound to the config's "authority" (ingest, issuer, tenant, device key id; `commons_credentials.rs:113`). Renewal checks tenant and `account_id` (`nearai_onboarding.rs:894-899`) | Uploads and account reads stop; renewal is attempted |
| 6 | Contributor config (`contributor.json`: tenant, device key id, ingest and issuer) | File (`config.rs:22`) | Enrollment, the identity switch | -- | Unenroll (`ENROLLMENT_FILES`), logout | The Commons tenant | Not enrolled |
| 7 | Device key | `Kind::Device` record plus OS entry, account service (`commons_credentials.rs:37-41`) | Enrollment; the legacy migration's staged key (`Kind::StagedDevice`, promoted by `switch_identity`, `daemon/commons_credentials_switch.rs:197`) | Never | Unenroll, logout | Registered to one tenant on the server (`DEVICE_KEY_REGISTERED_ELSEWHERE`, change-account spec 1.1) | Not enrolled; `device_key_revoked` is remembered |
| 8 | Payout link cache and organization names | `daemon-payout-link.json`, `daemon-payout-org-names.json` (`config.rs:62`, `:68`) | `daemon/payout_link.rs` | Re-read from the server | Unenroll, logout | Server side: `trace_payout_links` carries `subject_index`, the NEAR AI subject that proved the org (`migrations/V132__payout_rails.sql:108`) | Shows no link |
| 9 | Payout link proof (a NEAR AI access token) | Not stored: exchanged from #2 for one request and dropped (`daemon/payout_link.rs:308-339`) | -- | -- | -- | Server checks the subject against the account's live `near_ai_login` anchors, **only when the account has one** (`crates/trace-commons-server/src/payout_recipient_proof.rs:224-227`) | `subject_mismatch`, `near_ai_login_unverified` |
| 10 | Inference connection choice | `daemon-inference-connection.json` (`config.rs:57`), server table `trace_account_inference_connections` (V79) | Account reads | Account reads | Unenroll, logout | The Commons account. A choice, not a credential (V79 header) | Default connection |

**Settlement holds no contributor credential.** Credit and referrals are
keyed by the Commons account. Payouts go to the organization in
`trace_payout_links`, sent with the operator's rail credentials. The only
contributor-side input is the proof token (#9) at link time; the link is not
re-verified at payout time. The starter advance does not send anything
yet: `live` mode refuses startup
(`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/starter_advance.rs:8-15`).

**Not part of the connected account:** the Insights digest key
(`trace-commons.insights.digest-key`, `channel.rs:99`), the Claude usage
token cache (`trace-commons.claude-usage.token-cache`, `:100`), and the
saved agent model accounts (`managed-accounts.json`,
`managed/accounts.rs:11`). They are about this Mac and the person's own
agent providers, not about NEAR AI or the Commons account. Nothing here
changes them.

**What already changes together.** #1 and #2 are one keychain bundle,
replaced and forgotten as a unit under a cleanup journal
(`cloud_credential_lifecycle.rs:402-460`). #5 and #7 use the same storage
and commit locks (`nearai_credential::session::coordination`, shared by both
modules) and their own journal. `switch_identity` already replaces config,
device key and account session as one staged change with rollback
(`commons_credentials_switch.rs:1-35`). The gap is between the two sets:
nothing ties the NEAR AI bundle to the Commons record.

## 2. The invariant

> Every account-bound credential (#1-#5, #8, #9) names the same NEAR AI
> identity and the same Commons account, or the app is in an explicit,
> visible "needs sign-in" state.

"Names the same NEAR AI identity" is checkable only where the identity is
recorded. Today the Commons knows it (the anchor) and the daemon does not.
Section 3 adds a local record.

### Paths that break it

| # | Path | What diverges | Status |
| --- | --- | --- | --- |
| B1 | **Sign-in with a different NEAR AI identity on an enrolled device** (the owner's case) | `persist_attempt` replaces #1 and #2 before the Commons is asked; #5 stays bound to the old identity and stops renewing | **#1837** checks the Commons between mint and persist and stores nothing on refusal |
| B2 | Sign-in on a **signed-out** device (config and device key kept, no account record) | #1837 does not check (its table: "signed out ... not checked"). Any identity is stored; the next Commons sign-in with it is refused (`DEVICE_KEY_REGISTERED_ELSEWHERE`), and the device is stuck until the person signs in with the old identity | Open |
| B3 | Sign-in on a device that **joined by code** | No Commons check is possible today: the device renews by device key (`nearai_onboarding.rs:1002`) and never presents a NEAR AI login. Private AI can bill any NEAR AI identity while the Commons account belongs to another | Open; #1837 does not check these devices |
| B4 | **CLI logout** (`commands.rs:752` -> `ConfigStore::wipe`, `config.rs:1464`) | Deletes `daemon-settings.json` (holding #1/#2's reference) without journaling the keychain bundle, so the bundle is orphaned in the keychain; the key is not revoked at NEAR AI; #3 and #4 survive | Open |
| B5 | **Unenroll** (`daemon/unenroll.rs:130-174`) | Clears #5-#8; keeps #1-#3. Private AI keeps billing the NEAR AI identity. Not a mismatch, since there is no Commons account, but a later enrollment from a *different* login inherits #3's choice and revocations | Open; whether unenroll should keep Private AI is a product decision (Q1) |
| B6 | **NEAR AI forget** (`mod.rs:362`, `:435`) | Clears #1/#2 locally; #5 keeps working until it expires, then renewal needs a sign-in. Visible (`near_ai_sign_in_required`), so it meets the invariant. But the key stays live at NEAR AI | Open (revocation) |
| B7 | **Native sign-out** (`native_identity.rs:127`) | Clears #5 only; #1/#2 keep billing. Then B2 applies to the next sign-in | Open |
| B8 | **Spend-org switch** across identities | #3's pending revocations belong to the old identity and are swept with the new session | #1833 keeps them; #1837 drops a 403 as "not this account". Partly covered |
| B9 | **Payout link** for an account with no `near_ai_login` anchor (wallet-origin `near-` tenants) | Before #1841, the subject check was skipped (`payout_recipient_proof.rs:224` on main at the time), so an organization owned by any NEAR AI identity could be linked | Fixed by #1841 and its follow-up (F6). Only a recorded NEAR AI login proves a link, on staging and production alike (owner, 2026-11-02). A wallet-origin account with no recorded NEAR AI login is refused with the generic answer, whatever it signs in with; its path to payouts is the change-account flow (F10, the companion spec), which records a NEAR AI login. No step requires a NEAR wallet. A principal-linked legacy or pilot account cannot link until F11 |
| B10 | **Payout link after a change of account** (the change-account flow) | The link keeps the organization proved by the retired identity | Owner question in the change-account spec (Q5) |
| B12 | **Locally broken Commons account, after #1837**: the account record is malformed, or the enrolled Commons address is now refused by this build's allowlist | #1837 treats both as "not bound" so NEAR AI sign-in is never blocked for good: the new NEAR AI login is stored with no Commons check, and no `commons_login` is recorded. The Commons session cannot renew either. Nothing tells the person; the app looks signed in to Private AI while uploads and account reads quietly fail | Known break once #1837 lands. Owner-approved fix (2026-11-02): a visible "needs attention" state in every shell (F1, section 3.2a) |
| B11 | First run, legacy migration, renewal | Consistent by construction: each provisions or renews from the stored login (`legacy_migration.rs:186-189`; `renew_account_session`). Safe while #2 cannot change underneath them, which #1837 provides for B1 | Covered by B1's fix |

## 3. Design: change them together

### 3.1 A connected-account record

One small record, `connected-account.json`, written last under the shared
commit lock:

```json
{
  "version": 1,
  "generation": 7,
  "tenant_id": "nearai-...",
  "account_id": "...",
  "near_ai_subject_digest": "sha256:...",
  "login": "github",
  "cloud_reference": "<reference of the #1/#2 bundle>",
  "account_reference": "<reference of the #5 record>"
}
```

- `near_ai_subject_digest` is a digest of the NEAR AI subject id under a
  per-install key, read with the daemon's own token from `GET
  /v1/users/me`. Local only, never sent, never logged (change-account spec
  3.7).
- `login` replaces #1835's `commons_login`; `stored_login` remains a
  property of #2, keyed to its inference key as #1835 reads it.
- The record holds references, never secrets, like the switch journal.

**Status reads it.** On every status build and at startup, the daemon
compares the record with the live references. Any of: a missing record
with both sets present, a reference that does not match, or a #2 whose
subject digest differs, gives one state, `account_state:
"needs_sign_in"`, with a reason label. Private AI, payout linking and
uploads keep their own existing refusals. Nothing new is shown beyond the
existing sign-in prompt (section 5).

### 3.2 One commit point

Every writer of either set goes through one function:

```text
commit_connected(staged: { cloud: Option<bundle>, account: Option<record> },
                 verified: CommonsAcceptance, expected: generation)
```

1. **Stage.** Prepare new OS entries for whichever halves change
   (`CloudCredentialLifecycle` and `commons_credentials` already prepare
   before publishing). Journal both references for cleanup.
2. **Verify, outside the lock.** The Commons must have accepted the NEAR AI
   login that the staged #2 came from: a provisioning or renewal answer
   naming this tenant and account, or a rebind answer (change-account
   flow). For B3 devices this needs a server check that does not exist
   yet (fix F4). No acceptance, no commit.
3. **Commit, under the commit lock.** Re-read the generation. A sign-out or
   another commit since `expected` wins. Write a commit journal holding the
   old references, then the cloud metadata, the account record and the
   connected-account record, in that order. Advance the generation.
4. **Roll back on a partial failure.** Any failure before the
   connected-account record is written restores the old references from the
   commit journal, in-process or at the next start, as
   `roll_back_switch` does (`commons_credentials_switch.rs:367`). The
   staged entries are left for the cleanup journal.
5. **Clean up after commit.** Retire the old OS entries. Revoke the old
   NEAR AI key at NEAR AI with the old session when the identity changed;
   a failed revoke stays pending, tagged with its identity's digest, so the
   sweep never runs it with another identity's session (fixes B8).

#1837's ceremony is steps 1-3 for sign-in, without the record. It moves
onto this function unchanged in behaviour.

### 3.2a The "needs attention" state (B12)

The surface and its wording are owner-APPROVED (2026-11-02), with the
button "Set up again":

| Key | APPROVED text |
| --- | --- |
| `account.attention.title` | This app's account needs attention |
| `account.attention.record_unreadable` | This app can't read its account details on this Mac. Set it up again to keep sending traces. |
| `account.attention.address_refused` | This app can no longer reach the service it was set up with. Update the app, or set it up again. |
| `account.attention.private_ai` | Private AI still works with your NEAR AI account. |
| `account.attention.set_up_again` | Set up again |

`account.attention.private_ai` is shown only when a NEAR AI login is
stored. The block is a status in the account section, not a banner or
notification. "Set up again" runs unenroll and then first run (below).

- **Daemon.** When the account record fails to parse, or the enrolled
  Commons address is refused by the allowlist, status carries
  `account_state: "needs_attention"` with a fixed reason label:
  `account_record_unreadable` or `account_address_refused`. These are the
  two #1837 cases it calls "not bound" (its table: "malformed account
  record, an enrolled address the allowlist refuses"). The state is
  computed on every status build from the same `load_account_record` and
  allowlist check #1837 uses, so it appears whether or not a sign-in has
  happened. It is distinct from `needs_sign_in`: signing in again does not
  fix it.
- **What it says about Private AI.** While the state holds, a NEAR AI
  login stored without the check is marked `near_ai_login_unconfirmed:
  true`, so the shells do not present it as the account's login (no
  "Connected with {provider}" line, no chooser mark).
- **The way out.** `account_record_unreadable`: "Set up again"
  (unenroll, then first run). Under owner decision 1 (section 6),
  unenroll clears the whole connected set, the NEAR AI login included, so
  first run starts with a fresh NEAR AI sign-in and provisions from it;
  consistent by construction (B11). Until F9 makes unenroll clear the
  NEAR AI login, first run provisions from the login still stored. `account_address_refused`: the
  same, or update the app if the address was dropped from this build by
  mistake. The daemon offers the action; it never unenrolls on its own.
- **Every shell.** The state and reason cross IPC and the C ABI as two
  status fields (both header copies). macOS renders it in the account
  section where "Connected with" would be; GTK, Windows and Tauri render
  the same strings in their account sections. This is the one place in
  this design where all four shells must change in the same release, since
  a shell that ignores the field shows a working account.

### 3.3 Sign-out clears the whole set

One verb, `account_disconnect`, replaces the three partial clears (B4, B6,
B7):

- Under the commit lock: clear #1, #2 and #5, the connected-account record,
  #3 and #4. Journal every reference.
- After it: revoke the NEAR AI key at NEAR AI, best effort, with the
  session being cleared (B6 today does not revoke at all).
- The device key and config stay. Unenroll is disconnect plus removing
  those (#6, #7) and the enrollment files.
- CLI `logout` calls the same path before its wipe, so the keychain bundle
  is never orphaned (B4).
- `forget` (NEAR AI only) and native sign-out (Commons only) become
  disconnect. Whether to keep a NEAR AI-only "forget" for people who use
  Private AI without the Commons is Q1.

### 3.4 How the change-account flow uses it

The change-account flow's rebind answer is the `CommonsAcceptance` for the
new identity. The flow stages the new NEAR AI bundle from the held-aside
sign-in, calls `commit_connected` with both halves (the account record
keeps its `account_id`; its session is unchanged), and step 5 revokes the
old identity's keys with the old session. A failure anywhere leaves the old
set and the old record in place.

## 4. Fix list, ranked by risk

Each fits in one PR.

| Rank | Fix | Covers | Size | Existing work |
| --- | --- | --- | --- | --- |
| F0 | Gate the sign-in persist on Commons acceptance | B1 | -- | **#1837** (open); prerequisite, not counted |
| F1 | The "needs attention" state for a locally broken Commons account: daemon status fields, C ABI, and the surface in all four shells | B12 | Small daemon change, one surface per shell | Owner-approved 2026-11-02; follows #1837, whose "not bound" cases it reports |
| F2 | CLI logout and unenroll stop orphaning the NEAR AI keychain bundle: journal its reference before the wipe; add `near-ai-spend-org.json` to the wipe list (`near-ai-login.json` is already an enrollment file in #1835) | B4, part of B5 | Small, client | **In progress** (separate PR) |
| F3 | Check a sign-in on a signed-out but enrolled device against the config's tenant (the provisioning ceremony with the existing device key is the check); refuse to store on refusal | B2, B7's follow-on | Small, client; extends #1837's table | **In progress** (separate PR); #1837 marks it "not checked" |
| F4 | Server route for a device that joined by code to ask "does this NEAR AI login belong to my account?" (native session plus token; answers yes or no, names nothing), and the client gate using it | B3 | Medium, server and client | None |
| F5 | Tag pending revocations with the identity's digest; the sweep skips other identities' entries | B8 | Small, client | **In progress** (separate PR); #1833 kept them, #1837 drops 403s |
| F6 | Payout link: the proving NEAR AI login must be one of the account's recorded `near_ai_login` anchors, on every deployment. A `wallet` anchor proves nothing ("It should just be NEAR AI login on both staging and production", owner, 2026-11-02). An account with none is refused before the token is spent. The request's audit row is labelled `near_ai_login` | B9 | Small, server | **#1841**, then its follow-up that removed the wallet-anchor match; refuse, per decision 3 |
| F7 | The connected-account record and the `needs_sign_in` status check | Invariant detection | Medium, client and IPC | #1835's `commons_login` folds into it |
| F8 | `commit_connected`, with #1837's ceremony moved onto it | Single commit point | Medium, client | #1837 is its first caller |
| F9 | `account_disconnect` replacing forget, native sign-out and CLI logout's partial clears, with NEAR AI key revocation | B4, B6, B7 | Medium, client and every shell's sign-out | None |
| F10 | The change-account flow (companion spec, slice 1) on top of F7 and F8 | Deliberate change | Large | This PR's other spec |
| F11 | Give non-pooled, principal-linked legacy and pilot accounts (tenants outside `near-` / `nearai-`) a recorded NEAR AI login, the only anchor a payout proof can match. (Wallet-origin `near-` accounts get theirs through F10.) Two candidate designs: **(a)** move each account onto a NEAR AI tenant, carrying its history, devices and credit, so it holds a `near_ai_login` anchor like any NEAR AI account; or **(b)** change `trace_account_admission_linkage_ready()` (V81) so a `near_ai` device in a legacy tenant does not fail readiness for the whole deployment, then let such an account record its NEAR AI login. Pooled Devfolio tenants (`trace_legacy_invite_pooled_tenants`) are excluded from payouts by design and need nothing | B9's remainder | Large, server and migration | None. **Prerequisite for turning production payouts on** (owner, 2026-11-02); not a blocker for #1841. See `docs/operator/automatic-credit-settlement.md`, "Before production payouts" |

F1 should land with or straight after #1837, since #1837 is what makes B12 reachable silently. F2, F3 and F5 are independent and can land in any order. F8 depends on
F7. F9 and F10 depend on F8. F11 must land before production payouts are
turned on.

## 5. Wording

No new user-facing text is required. `needs_sign_in` uses the existing
sign-in prompt, and #1837 keeps "Sign-in didn't finish. Try again." until
its DRAFT wording is approved. Two DRAFT strings, if the owner wants them:

| Key | DRAFT text |
| --- | --- |
| `account.disconnect.button` | Sign out |
| `account.disconnect.body` | Signs out of NEAR AI on this Mac. Private AI and uploads stop until you sign in again. |

The "needs attention" strings are APPROVED and listed in section 3.2a.

The two sign-out strings are not notices. A notice that the app found mismatched credentials
would need approval (copy guide rule 9); this design does not propose one.

## 6. Owner decisions (2026-11-02)

The four questions this section asked were answered by the owner on
2026-11-02:

1. **Private AI does not outlive the account.** Sign-out, unenroll and
   logout clear the whole connected set, the NEAR AI login included.
2. **A device that joined by code may use only the account's own NEAR AI
   login.** F4 enforces it later.
3. **Payout links on accounts with no NEAR AI login anchor are refused**
   until the account has its login recorded.
4. **The inference key is revoked at NEAR AI on sign-out**, best effort.
