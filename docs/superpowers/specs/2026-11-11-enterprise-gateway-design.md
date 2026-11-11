# Near AI Gateway for organizations: admin-set credentials, an org ledger, and servers -- design

Date: 2026-11-11. Owner: Zaki. Status: DRAFT for owner review. Nothing here
is built, and nothing here is approved. Every table, route, role, file and
preference key in sections 3 to 11 is **proposed** unless the sentence
cites a path. Section 2 is what exists today. Section 14 lists the choices
the code could not settle.

Owner direction, 2026-11-11:

> "Design an enterprise version of the Gateway app where an administrator
> could set credentials for an entire org and traces get credited in a
> central place."

> "can we also imagine integrating servers as well into the app"

Code references are to `origin/main` at 70b046e5f. Abbreviations:

- **ING** = `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs`
- **INT/** = `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/`
- **SRV/** = `crates/trace-commons-server/src/`
- **C/** = `crates/trace-commons-contributor/src/`
- **P/** = `crates/trace-commons-protocol/src/`
- **M/** = `macos/Sources/`

Citations marked (r) come from a research pass over the tree for this
spec and were not re-opened line by line while writing; unmarked citations
were re-read. Treat an (r) line number as a pointer to check, not as
proof.

## 0. Summary

1. An organization is **one tenant** (new `org-` namespace) holding one
   account per member and one per server, plus one **holder account** that
   owns the org's settled credit. Members never share an account.
2. Credit is **issued to the holder account at settlement**, by one new
   arm in `settlement_group_key`. The ledger rows keep the submitting
   principal, so per-member and per-server attribution is a read over rows
   that already exist. Nothing is transferred, so "non-transferable" holds.
3. What an admin sets once is an **authority, never a replicated secret**:
   the org's enrolment authority and its NEAR AI organization. Each device
   still generates its own Ed25519 key and gets its own inference key.
   Configuration profiles carry non-secret settings and at most a bounded,
   revocable enrolment credential.
4. The org admin is a **new narrow seat role on an account session**, not a
   bearer token role and not the existing tenant `admin`,
   which can read any member's quarantined traces today.
5. **Policy only narrows.** An admin can cap or switch off sharing and
   pick the ceiling of data uses. No admin path arms a folder, widens a
   member's chosen uses, or turns on Automatic for a person. Servers are
   the stated exception: the org consents for them by policy.
6. The MVP reuses fixed-tenant invites, the tenant policy and access-grant
   ceilings, the device-key upload claim, the NEAR AI org ownership check
   and the payout rails. New surface: three tables, one role, one
   settlement arm, an org console on ingest, a managed-configuration
   reader, and an unattended mode for the Linux daemon.
7. **One app, one download** (9.1). Enterprise is a managed state the
   app enters only from a configuration profile or an org's enrolment
   link; people who download it themselves never see org surfaces.

## 1. Corrections to the brief

The request that produced this spec assumed several things the tree no
longer has or never had. The design uses the corrected facts.

- **Passkeys, account bindings and step-up are gone.**
  `migrations/V144__remove_passkeys.sql:1-3` (r) drops
  `trace_account_bindings` and `trace_webauthn_credentials`. Login is a
  NEAR AI sign-in; a NEAR wallet login and device-minted login links also
  exist (ING:9191-9278 (r)). There is no strong second factor for any
  account today: a native session is "weak" and only `client_kind ==
  "near"` counts as strong (`SRV/account_session.rs:155-158` (r)).
- **"Redeemable by holder only" is not a stated property of the
  instrument.** The phrase appears once, as the principle decision D6 of
  the payout spec departs from
  (`docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md:69-76`).
  What the documents state is **non-transferable**
  (`docs/trace-commons.md:72-74`, `README.md` "Trace Credit").
- **The modes are Ask, Automatic, Never** in shipped copy
  (`C/project_copy.rs:361-392` (r)); "Ask me" survives in first-run state
  and one spec. Modes are per folder, with a global override
  (`C/daemon/policy.rs:67-76, 426-483` (r)).
- **There is no upload allowance.** `GET /v1/account/upload-allowance`
  answers `{"allowance": null}` (`INT/upload_allowance.rs:58-67` (r)).
- **V139 adds columns, not a table**, and V141's table is
  `trace_consent_grants`, the add-only model-training grant
  (`docs/trace-commons.md:244-300`), not a record of `GrantTerms`.
- **`GrantTerms` never reaches the server.** It is a local record in
  `daemon-projects.json` (`C/daemon/grant_terms.rs:78-106` (r)). The
  server sees consent scopes and allowed uses on the claim and the
  envelope, not which mode sent a trace. Section 5 depends on this.
- **"Qualify before install" is a server-deploy spec**, not a first-run
  step.
- **No concept of org, team, MDM, SSO, OIDC or SAML exists** in `crates/`,
  `macos/`, `migrations/` or `docs/` (r). "Organization" in the tree
  always means a NEAR AI cloud organization (spend org, payout recipient).
  "Managed" always means managed coding sessions.

## 2. What exists today that this builds on

### 2.1 Tenants, roles and policy

- `trace_tenants`, `trace_tenant_policies` (allowed consent scopes and
  uses) and `trace_tenant_access_grants` (per-principal role, allow-lists,
  issuer/audience/subject binding, expiry, revocation) are in
  `migrations/V1__trace_commons_schema.sql:11-60` (r). Policy and grants
  are described in `docs/trace-commons.md:673-720`: grant allow-lists
  **intersect** with claim allow-lists and cannot raise a role.
- RLS is forced; predicates go through `trace_current_tenant_id()`
  (`migrations/V18__trace_central_rls_tenant_predicate.sql:1-7` (r)), set
  per transaction by `PgBackend::begin_trace_tenant_transaction`
  (`SRV/db/trace_corpus_pg.rs:1334-1345` (r)).
- Tenant ids are made three ways: derived per user
  (`P/onboarding.rs:132-138` (r)), fixed by an invite, or random per
  account (`near-`, `nearai-`; `SRV/near_account_identity.rs:486-503`
  (r)). Only the last two prefixes are "anchored"
  (`P/admission.rs:206`).
- `TokenRole` (ING:4015-4031) has no tenant-admin distinct from the
  operator. `Admin` passes `can_review()` and `can_export()`
  (ING:4058-4072), and `can_access_submission` is `can_review() ||
  principal_owns_submission` (ING:63220-63221). So **a tenant `admin`
  credential reads every quarantined trace in its tenant** through
  `/v1/review/*` and reaches about sixty `/v1/admin/*` route families
  (drills, settlement, exports, PII-backstop operations).

### 2.2 Accounts, devices and identity

- An account is `(tenant_id, account_id)`
  (`migrations/V30__trace_accounts.sql:26` (r)); a principal belongs to at
  most one account per tenant (`V30:43-53` (r)). A tenant can hold many
  accounts.
- Each NEAR AI login is provisioned into **its own new `nearai-` tenant**
  (`SRV/near_account_identity.rs:499-503` (r)). The login is stored only
  as a blind-index anchor, `UNIQUE (anchor_hash)` across the deployment
  (`migrations/V58__near_account_provisioning.sql:17-27` (r)), one live
  anchor per account (V150 (r)). A login that already has an account is
  refused (`near_ai_login_in_use`, F10 spec (r)).
- A device key is Ed25519, generated on the device, stored in the OS
  credential store, and registered to **one tenant**
  (`C/identity.rs:64-93`, `C/daemon/commons_credentials.rs:22-48`,
  `migrations/V28__device_keys.sql:8-18` (r)). It signs the upload-claim
  request body (`C/identity.rs:270-292` (r)); envelopes are not signed.
- The issuer mints a 300-second EdDSA claim with `role = "contributor"`
  from a device signature, a device JWT, or a **workload JWT** whose
  `tenant_id` is authoritative (`SRV/trace_upload_claim_issuer.rs:1944-2030`).
  The workload path is EdDSA-only and refuses a device-shaped subject.
- Fixed-tenant invites exist: `onboarding_invite_grants` with
  `tenant_mode = 'fixed'`, `fixed_tenant_id`, per-invite scope and use
  ceilings, `max_uses`, `expires_at`, `revoked_at`, stored by hash
  (`migrations/V42__onboarding_invite_grants.sql:13-45`). The app already
  handles `tracecommons://enroll?invite=...` (`C/deep_link.rs:40`), and
  the CLI reads `TRACE_COMMONS_INVITE` (`C/commands.rs:304` (r)).
- "Instance" enrolment exists: an allow-listed instance key signs a
  per-user attestation and the issuer derives a per-user tenant
  (`SRV/trace_upload_claim_issuer.rs:2580-2809`,
  `migrations/V35__trace_instance_enrollments.sql` (r)). It vouches for
  users but puts each in a separate tenant.
- Device codes (`migrations/V140__account_device_codes.sql` (r)) join the
  redeeming machine to the **minting person's account**, three
  outstanding at most. They are the wrong tool for a fleet.
- A tenant admin can already revoke a device key in its tenant:
  `/v1/admin/device-keys/{id}/revoke` (ING:15067-15084 (r)).

### 2.3 Credit and payout

- Ledger rows are keyed by the submitting principal:
  `trace_credit_ledger.credit_account_ref` with no account column
  (`V1:300-322` (r); pipeline path `SRV/versioned_pipeline/service.rs:5368-5379`
  (r)).
- Settlement groups rows by `settlement_group_key`: `account:{account_id}`
  when the principal is linked to an account, else the principal
  (`SRV/db/mod.rs:2313-2321`). The mapping reads the
  `trace_account_owned_principals` view
  (`migrations/V135__device_upload_ownership.sql:61-69`).
- **Settling into an account and seeing through an account are separate
  code paths.** Visibility widens only through
  `caller_credit_account_scope` (ING:63818-63843), which requires the
  caller's own active membership. This is what lets a settlement-level
  holder exist without giving anyone a view of anyone else's traces.
- Payout rails are per `(tenant_id, account_id, rail)`
  (`migrations/V132__payout_rails.sql:97-140` (r)). The `near_ai` rail
  pays a **NEAR AI organization** the account links, and "many accounts
  may name one recipient" (D7). The link proof is a NEAR AI access token
  whose `/users/me` lists the org as active with role `owner`
  (`SRV/payout_recipient_proof.rs:8-25, 44, 281-282`); the server reads
  only the subject id and each org's `id`, `role` and `is_active`.
- Units: Trace Credit has 6 decimals, `AtomicUnits(u128)`, payout columns
  `NUMERIC(39,0)` (`crates/trace-commons-gate-api/src/pipeline.rs:22-30,
  222` (r)). A NEP-141 rail is named as future work in the payout spec
  (line 13) and its descriptor exists
  (`SRV/versioned_pipeline_compat.rs:253-256` (r)).
- Pooled fixed tenants (Devfolio) are marked by hand in
  `trace_legacy_invite_pooled_tenants` and are excluded from payouts and
  linking (payout spec section 6; legacy payout spec section 7 (r)).

### 2.4 The client

- State is in `contributor.json` (endpoints, tenant, device key id,
  scopes) and `daemon-settings.json` (`C/config.rs:22-30, 150-287`,
  `C/daemon/settings.rs:412-841` (r)). Channel origins are compiled in
  (`C/channel.rs:88-160` (r)).
- The macOS app reaches everything through `tc_call` and starts the
  daemon with `tc_daemon_start_with_settings`
  (`crates/trace-commons-contributor-ffi/include/trace_commons.h:314,
  2318` (r)). Bundle id `ai.tracecommons.shell`
  (`macos/channels/production.env:16-18` (r)).
- **Nothing reads managed preferences.** The only MDM reference is a
  refusal to launch OpenCode when
  `/Library/Managed Preferences/ai.opencode.managed.plist` exists
  (`C/managed/opencode.rs:96-118` (r)).
- One person signs in to NEAR AI once; the daemon mints an `sk-` inference
  key in a NEAR AI org and workspace and keeps a refresh session, both in
  the OS store (`C/daemon/settings.rs:231-330` (r)). Private AI is the
  embedded IronWire proxy (`C/daemon/private_inference.rs:1-48` (r)).
  IronWire backends are declared in `$IRONWIRE_HOME/config.toml` with
  kinds including `nearai`, `openai-compatible` (needs `base_url` and
  `api_key_env`) and `local` (`crates/ironwire_core/src/config.rs:589-630,
  1246-1276` (r)).
- Redaction is on the device: `DeterministicTraceRedactor`
  (`C/envelope.rs:98-148` (r)). The server re-scrubs every upload
  (`docs/trace-commons.md:149-168`).

### 2.5 Headless today

- The CLI runs on Linux with `--json`; `daemon install` writes a systemd
  **user** unit (`C/daemon/install.rs:22-51` (r)). There is no system
  unit and no contributor container image.
- The only Linux credential backend is Secret Service, with no plaintext
  fallback (`crates/trace-commons-contributor-platform/src/os_secret_store.rs:120-154`,
  `docs/operator/commons-credential-storage.md:21-24` (r)). A box with no
  D-Bus session cannot store a device key.
- Sends hold on `consent-scopes-not-chosen` until
  `config complete --scopes` (`C/config.rs:396-414` (r)); hold on an
  unaccepted terms version, which only the IPC `terms_accept` clears
  (`C/daemon/terms_state.rs:93-104, 323-330` (r)); and a session flagged
  by the scrub check waits for a person (`C/daemon/settings.rs:615-638`
  (r)). Arming a folder works from the CLI (`daemon project <path> --mode
  auto`); the global Automatic override has no CLI command.
- `SessionTranscript` carries no author, git or user field; `cwd` is sent
  only as a hash (`crates/trace-commons-contributor-sources/src/source/mod.rs:470-492`
  (r)). The envelope's only free-form map, `feature_flags`, is redacted
  by the server re-scrub (`P/trace_contribution.rs:6026-6033` (r)).
- No client-side attestation or workload identity exists (r).

## 3. Actors and trust

| Actor | What it is | Can see | Cannot |
|---|---|---|---|
| **Member** | A person whose Mac runs Near AI Gateway enrolled in an org | Their own traces, previews, history, and the credit their traces earned for the org | Other members' traces or figures |
| **Server** | An org machine running agents with nobody present; a non-human member | Nothing interactive | -- |
| **Org admin** | A person holding the proposed `org_admin` role | Seats and machines, policy, org credit, per-seat counts and credit, the org audit trail | Any trace body, preview, redacted envelope, review queue, export, or another org |
| **Org (holder)** | The holder account that owns settled credit | -- | -- |
| **Operator** | Trace Commons | As today | As today |

Trust statements:

- The Commons trusts the org to say who its people and machines are, and
  nothing else. Every upload is still treated as untrusted and re-scrubbed.
- A member trusts the Commons with scrubbed traces under the terms they
  accepted. A member does **not** have to trust the admin with trace
  content: the org surface never serves it.
- The org trusts the Commons to attribute and settle credit.
- Orgs do not trust each other; isolation is the tenant boundary.

### 3.1 The member and the admin

The member is the person whose work leaves their machine. The design
answer has four parts.

1. **The consent act stays the member's.** Arming a folder, the Automatic
   grant, and the choice of data uses are the member's acts, exactly as
   today. Org policy is a ceiling handed to the client and enforced again
   on the server through the tenant policy and the seat's access grant.
   Policy can force Never, remove a data use, or forbid Automatic. It
   cannot select Automatic, add a use, or arm a folder.
2. **Joining an org voids standing grants.** `GrantTerms` already includes
   `tenant_id`, `ingest_url`, `issuer_url`, `consent_scopes` and the
   witness terms, and a widening change voids the grant
   (`C/daemon/grant_terms.rs:78-106` (r)). The proposal adds the org
   policy digest and the credit holder to those terms, so moving a Mac
   into an org, or the org loosening policy, returns every folder to Ask
   until the member arms again with the new facts in front of them.
3. **The admin has no content path.** `org_admin` is not a `TokenRole`
   at all (4.2), so it cannot reach a route guarded by `can_review()`,
   `can_export()` or `can_admin()`. The org console reads counts, sizes,
   states, credit and hashed labels.
4. **Managed state is visible.** The member can always see that the Mac
   is managed, by whom, where credit goes, and what policy holds. How it
   is shown is an open decision (section 14, decision 9), because rule 9
   of `docs/ui-copy-guide.md` requires approval of any new notice.

What this does **not** promise, stated plainly:

- The server cannot tell a member-approved trace from a policy-armed one.
  No grant or mode reaches it today. Section 5.3 adds a `consent_basis`
  on the claim, but a client asserts it, and the client runs on a machine
  the org administers. It supports audit and refuses a category error (a
  human seat claiming org consent); it is not proof of a human's intent.
- An admin with device management has root on the Mac and can read raw
  session files without this product. The guarantee is about the Gateway
  channel and the org console: nothing a member did not send reaches the
  Commons through Gateway, and nothing a member sent reaches the admin as
  content.
- A managed profile cannot redirect uploads. Endpoints stay the compiled
  channel origins plus what the commons publishes at enrolment
  (`C/channel.rs:440` (r) refuses cross-channel origins). No managed key
  sets `ingest_url`, the issuer, the witness or the allowed hosts.

## 4. Data model

### 4.1 An org is one tenant

One tenant per org, in a new `org-` namespace: `org-` plus 32 random
bytes hex, made the way `nearai-` tenants are.

Why not group the members' existing `nearai-` tenants: credit cannot
cross tenants (legacy payout spec, C3 (r)), account merges are
same-tenant only (`migrations/V82__near_account_merge.sql:57-62` (r)),
RLS would need a second predicate on every table, and the tenant policy
and access-grant ceilings are already the admin's policy lever at tenant
grain.

Why not one account for the whole org with every device as a principal:
account membership widens visibility (`caller_credit_account_scope`) and
makes every member's trace "the caller's own" for withdrawal and the
model-training grant (`docs/trace-commons.md:262-270`). Members would see
and withdraw each other's traces.

How an org tenant differs from today's pooled fixed tenants: it has an
org row, every seat has an account from the moment it enrols, it has a
holder account with a payout link, and it is never listed in
`trace_legacy_invite_pooled_tenants`.

`org-` is deliberately **not** added to `ANCHOR_NAMESPACES`
(`P/admission.rs:206`). Today a non-anchored tenant falls through
`reserve` with no account-admission gate (`INT/admission.rs:486-510`), so
the org namespace needs its own admission arm (section 4.5) and must fail
closed when the org row is missing.

One org is one tenant in this design. Several tenants per org (regions,
subsidiaries with separate ledgers) is not designed; the org row's key is
the tenant id so that a later parent table can group org tenants without
re-keying anything.

### 4.2 Proposed tables

All carry `tenant_id`, forced RLS, and the standard policy
`USING (tenant_id = trace_current_tenant_id()) WITH CHECK (...)`.
Migration numbers are not assigned here.

**`trace_orgs`** -- one row per org tenant.

| Column | Purpose |
|---|---|
| `tenant_id` PK | The org tenant. |
| `status` | `active`, `suspended`, `closing`. Suspended refuses new claims. |
| `holder_account_id` | FK to `trace_accounts`; the account settled credit is issued to. |
| `credit_mode` | `org` in the MVP. Reserved: `member`, `split` (decision 3). |
| `near_ai_org_index` | Blind index of the bound NEAR AI organization id, keyed under its own domain so it cannot be joined to a login anchor or a payout `subject_index` (the precedent is `V132:108` (r)). `UNIQUE`: one Commons org per NEAR AI org. |
| `policy_version`, `policy_digest` | The current org policy (4.4). |
| `seat_limit`, `server_limit` | Operator-set ceilings. |
| `display_label_sealed` | The org's display name, KMS-sealed like `sealed_account_name` (V61 (r)). Never in a log or audit row. |

**`trace_org_seats`** -- one row per member or server account.

| Column | Purpose |
|---|---|
| `tenant_id`, `account_id` PK | FK to `trace_accounts`. |
| `kind` | `member`, `server`, `holder`. |
| `org_role` | `member`, `org_admin`. `server` and `holder` seats have none. |
| `identity_index` | Blind index of the proof that admitted the seat (NEAR AI subject, or later an OIDC `iss`+`sub`), under an org-membership domain. `UNIQUE (tenant_id, identity_index)`. Not an anchor: a person's personal `nearai-` account keeps its anchor. |
| `label_sealed` | Admin-supplied label ("build-runner-7", a staff id), sealed. Shown only in the console. |
| `consent_basis` | `member_grant` for members, `org_policy` for servers. |
| `status`, `joined_at`, `removed_at`, `removed_by_ref` | Lifecycle. |

**`trace_org_enrolment_authorities`** -- how seats are admitted.

| Column | Purpose |
|---|---|
| `tenant_id`, `authority_id` PK | |
| `kind` | `near_ai_org_membership`, `enrolment_token`, later `oidc_issuer`, `workload_issuer`. |
| `seat_kind` | `member` or `server`; an authority admits one kind. |
| `binding_hash` | For `oidc_issuer` / `workload_issuer`: hash of issuer, audience and subject pattern. Null otherwise. |
| `max_seats`, `expires_at`, `revoked_at` | Bounds. |

`enrolment_token` authorities do not get a new credential format. Each
one fronts an `onboarding_invite_grants` row with `tenant_mode = 'fixed'`
and `fixed_tenant_id` the org tenant (V42), so issuance, hashing,
`max_uses`, expiry and revocation are the existing code. The proposal
adds two columns to that table, `seat_kind` and `org_authority_id`, and
makes redemption of such an invite create the `trace_accounts` row and
the seat in the same transaction. Whether today's fixed-invite onboarding
creates an account or leaves a bare principal was **not traced to the
end** (`SRV/trace_upload_claim_issuer.rs:2343` onward); the payout spec's
mention of bare principals in pooled tenants suggests it can leave one,
which an org seat must never be.

**Audit.** Org actions append to `trace_account_audit` (`V30:125-135`
(r), hash-only and label-only by its own header) with new action labels:
`org_seat_admitted`, `org_seat_removed`, `org_device_revoked`,
`org_policy_changed`, `org_authority_created`, `org_authority_revoked`,
`org_payout_link_changed`, `org_admin_granted`, `org_admin_revoked`,
`org_report_read`. `safe_metadata` carries counts, the policy digest, a
hashed seat reference (`sha256:` over a domain tag, tenant and account
id, in the manner of `credit_account_hash`), and hashed labels. Never an
email, a name, a URL, a token, an org name, a NEAR AI org id, or a raw
tenant or account id: the payout spec's hash-only rule (lines 1001-1008)
excludes those from audit rows, and this follows it.

**Role.** `org_admin` is a **seat role on an account session**
(`trace_org_seats.org_role`), resolved on every request. It is
deliberately **not** a new `TokenRole` and not a new value in the role
CHECK on `trace_tenant_access_grants` (`V1:27-60` (r)). A `TokenRole`
can be minted as a static `tenant:role:token` entry or a signed claim
(`docs/trace-commons.md:605-625`), which would make org administration a
bearer string; and the existing `Admin` role must never be issued in an
org tenant, because of what it reaches (2.1). The `/v1/org/*` routes of
section 8 take the account-session resolver, which already rejects
device upload claims on `/v1/account/*` (ING:19962-19976 (r)), and
`AccountCtx` gains the seat's role (`SRV/account_session.rs:122-153` (r)
has no role field today).

### 4.3 Identities and the one-login problem

A NEAR AI login can be anchored once in the whole deployment, and a
device key belongs to one tenant. So today a person is either in their
personal `nearai-` tenant or somewhere else, per device, and a login that
already has a personal account cannot be re-provisioned.

The proposal does not anchor org members. An org seat is admitted by a
proof checked at enrolment and recorded as `identity_index`, under a
different HMAC domain from the anchor. A person with a personal account
on their own Mac and an org seat on a work Mac holds two unrelated
accounts in two tenants; nothing links them and nothing needs to.

One Mac holding both a personal and an org enrolment is **not** in this
design (decision 4). The state directory is one enrolment, and
`contributor.json` names one tenant.

### 4.4 Org policy

The policy is a small versioned document, served to enrolled devices and
mirrored into `trace_tenant_policies` for the server-side ceiling.
Proposed wire type in `P/org_policy.rs` (permissive crate):

| Field | Values | Enforced |
|---|---|---|
| `sharing` | `allowed`, `off` | Client: `off` applies the existing global Never override (`C/daemon/policy.rs:426-466` (r)). Server: `off` revokes the seats' contributor grants **and** the issuer refuses org seats that have no active grant. The second half is new: today a device with no grant, or a revoked one, falls back to a hardcoded floor of scopes (`SRV/trace_upload_claim_issuer.rs:2030-2077`: "a revoked grant still leaves the floor"), so revocation alone would not stop claims. |
| `allowed_consent_scopes`, `allowed_uses` | Subsets of the existing enums | Server: tenant policy and seat grants (existing intersect rule). Client: the uses screen offers only these. |
| `member_automatic` | `allowed`, `forbidden` | Client, and the issuer: an org claim carries the policy digest and how the trace was sent, and the issuer refuses an Automatic claim the org forbids (5.3, flow change A). There is no `required` value. |
| `scrub_check` | `automatic`, `manual` | Client: `manual` holds every session for the member (`C/daemon/settings.rs:614-641` (r)). An org can make review stricter, not looser. |
| `near_ai_spend_org` | `pinned` | Client: the spend-organization chooser is fixed to the bound org. |
| `private_ai` | `allowed`, `required_for_listed_tools`, `off` | Client. |
| `upstreams` | List of IronWire backend declarations (section 7.4) | Client. |
| `server_sources`, `server_roots` | For server seats: which tools and roots to watch | Client (server profile). |
| `member_may_sign_out`, `member_may_unenrol` | bool | Client only; a convenience lock, not a control. |

`policy_digest` is the SHA-256 of the canonical JSON. It is added to
`GrantTerms`. A change that widens (a use added, `sharing` off to
allowed, `member_automatic` forbidden to allowed, `scrub_check` manual to
automatic) voids standing grants on every member device. A narrowing
change takes effect without voiding.

### 4.5 Admission and anti-abuse at org grain

Per-account assumptions that break when many people settle to one line,
and what the design does about each:

| Mechanism today | Key | In an org tenant |
|---|---|---|
| Account admission budget (`V77:8-18` (r)) | account, anchored tenants only | New org arm in `reserve`: budget per org per period, plus a per-seat sub-limit so one runaway server cannot spend the org's period. Missing `trace_orgs` row for an `org-` tenant refuses with `org_record_missing`. |
| Invite-free admission evidence (`docs/operator/invite-free-admission.md`) | per submission | Not required. An org is an operator-vetted party, the equivalent of `invited` authority. |
| Settlement cap `TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` (ING:30853-30895 (r)) | settlement group key | Would cap a whole org at one person's cap. Add an org cap; when the holder arm is active and no org cap is configured, settlement reports a blocker in the style of `settlement_account_cap_missing`. |
| Starter advance (one per account, subject and recipient org) | account | Off for org tenants. |
| Referral (`V142` (r)) | account | Off for org tenants; a shared payout org already trips self-referral detection. |
| Earned trust tiers | account | Not evaluated for org seats. |
| Same-tenant duplicate handling (`docs/trace-commons.md:226-232`) | tenant | Unchanged, and it bites: colleagues on one repository are in one tenant, so their near-duplicate traces dedup against each other. That is the right answer for paying an org once, but it lowers the org's yield per trace. Measure it in the first pilot before quoting any org an expected figure. |
| Contributor concave cap (shadow) | principal | Unchanged. |

## 5. Credential provisioning

### 5.1 The rule

**The admin configures an authority. Each device derives its own
credentials from it.** Nothing that authenticates a device is replicated
across devices.

| Material | Scope | Why |
|---|---|---|
| Device key (Ed25519) | Per device, generated locally, never exported | It is the unit of revocation and of attribution. A shared key would make offboarding one person a rotation for everyone, would let any member submit as any other, and would break `trace_device_upload_principals`, which ties upload principals to a verified device for "financial ownership" (`V135:1-25` (r)). |
| Commons account session | Per seat | As today. |
| Upload claim | Per request, 300 s | As today. |
| NEAR AI inference key (`sk-`) | Per device, minted in the org's NEAR AI organization | Per-device revocation and per-device spend visibility. |
| Org enrolment token | Org-wide, bounded | The one shared secret. It admits a seat; it is not a credential for anything else. Bounded by `max_uses`, expiry, seat kind, and revocation. |
| NEAR AI organization binding | Org-wide, not a secret | An id, blind-indexed on the server. |
| Org policy | Org-wide, not a secret | Served to enrolled devices. |
| Provider API keys (Anthropic, OpenAI) | **Not distributed** | See 7.4: reached through an org-run upstream instead. |

### 5.2 Binding the org (admin, once)

1. The operator creates the org tenant, the holder account and the
   `trace_orgs` row, and issues the first `org_admin` seat. (Self-serve
   org creation is later; decision 12.)
2. The admin signs in to NEAR AI on the org console and names the NEAR AI
   organization. The server runs the existing ownership check: a fresh
   access token, one `/users/me`, the org must be listed, active, role
   `owner` (`SRV/payout_recipient_proof.rs:281-282`). This is the same
   proof that links a payout recipient today, used here to bind
   `near_ai_org_index`.
3. The same proof links the holder account's `near_ai` payout rail to
   that organization. This needs one new verifier arm: today the verifier
   requires the linking account to carry a `near_ai_login` anchor
   (`payout_recipient_proof.rs:8-17`), and the holder account has none.
   The arm accepts an `org_admin` seat whose `identity_index` matches the
   token's subject, for the holder account only. A link-time arm is not
   enough on its own: the payout spec has one account predicate "checked
   at link time and on every send" (its section 6), and
   `migrations/V147__revoke_payout_links_without_near_ai_login.sql:1-30`
   revokes every live `near_ai` link on an account with no
   `near_ai_login` anchor. The holder account is exactly such an account,
   so the shared predicate, and any later migration written in V147's
   style, need an explicit org-holder arm ("an org holder account whose
   link was proved by a live `org_admin` seat"), or the holder's link is
   refused at send or revoked at the next deploy.
4. The admin sets policy and creates enrolment authorities.

### 5.3 Admitting a member

Three paths, in the order the design recommends building them.

**Path A: NEAR AI organization membership (MVP for people).** The member
installs Near AI Gateway and signs in to NEAR AI, as every user does
today. The client learns the org from managed configuration (5.5) or
from an org enrolment link. The enrolment call carries the device public
key, the device signature and the NEAR AI access token, exactly the
shape of today's `NearAiFinishRequest` (`INT/near_provisioning.rs:1274-1292`
(r)). The server:

1. introspects the token (`SRV/near_ai_login.rs:207` (r));
2. requires the bound NEAR AI organization to appear in `/users/me` as
   active (any role);
3. computes `identity_index`, finds or creates the seat and its account
   in the org tenant, registers the device key to the org tenant, and
   writes the seat's contributor access grant with the policy ceilings;
4. returns a native session for the org account.

The daemon then mints its `sk-` key in the pinned organization. The
member performs **one sign-in and configures nothing**: tenant, credit
destination, spend organization, data-use ceiling and upstreams all come
from the org. Membership of the NEAR AI organization is managed where the
org already manages it.

This is "the admin set it for everyone" without the Commons holding any
org secret. It does keep one interactive sign-in per person.

**Membership must be re-proved, and today's sessions would not do it.**
A native session is 12 hours from its last rotation, rotates on use
every hour, and slides up to an absolute 30 days
(`SRV/account_native_auth.rs:66-94`). A Mac in daily use therefore keeps
one session for a month without ever repeating a sign-in. And the
existing renewal re-runs the provisioning ceremony for an anchored
`nearai-` account (`C/daemon/account_session_renewal.rs:12-16` (r)); an
org seat has no anchor, so that route does not apply to it.

Proposed: an org seat's session does **not** slide. It has a fixed
lifetime (12 hours suggested, an org policy value with an operator
ceiling) and is replaced only through a new org renewal route that takes
a fresh NEAR AI access token and repeats step 2 above. A person removed
from the NEAR AI organization then loses the Commons session within one
lifetime without the admin touching the Commons console. The issuer
must also refuse a device claim for a member seat whose last membership
proof is older than that lifetime, since upload claims are minted from
the device key, not the session (2.2); without that rule the session
bound would not bound uploads.

**Path B: enrolment token (MVP for servers; available for people).** A
fixed-tenant invite fronted by an `enrolment_token` authority. Delivered
by configuration profile, by `tracecommons://enroll?invite=`, or by
`TRACE_COMMONS_INVITE`. Admits a seat with no NEAR AI sign-in, so it
gives contribution and org credit but not Private AI. For a person, the
`identity_index` is derived from the device key, so the seat is "this
Mac", not "this person".

**Path C: SSO (later).** An `oidc_issuer` authority. The member signs in
with the org's IdP through the system browser on the existing loopback
redirect (`SRV/account_native_auth.rs:112-133` (r)); the server verifies
the ID token against the registered issuer and audience and uses
`iss`+`sub` as `identity_index`. This replaces the NEAR AI sign-in as
the Commons identity. It does **not** produce a NEAR AI inference key;
that needs either NEAR AI accepting the org's IdP, or the brokered
issuance of decision 2. Two facts to settle before building it: the
issuer's JWT paths are EdDSA-only (`SRV/trace_upload_claim_issuer.rs`,
`docs/trace-commons.md:650-656`) while IdPs sign RS256 or ES256, so this
is a new verifier, not a reuse; and SCIM or group claims for automatic
offboarding are a further piece. SAML is not proposed: OIDC covers the
IdPs an org of this kind runs.

**Consent basis on the wire.** Add `consent_basis` to
`TraceUploadClaimRequest` and to the minted claim (`member_grant` or
`org_policy`). The issuer refuses `org_policy` from a `member` seat and
`member_grant` from a `server` seat. Ingest stores it on the submission
row. The envelope is unchanged, so no envelope digest pin moves.

**Policy on the claim (flow changes A and C, 11.1).** A member seat's
claim request also carries the `policy_digest` the Mac acted on and how
the trace was sent (`submit` or `automatic`). The issuer refuses a
digest that is not the org's current one, and an `automatic` claim when
`member_automatic = forbidden`; the trace goes back to Held and nothing
leaves the Mac. Without this, Automatic is enforced only from the policy
the Mac last fetched, and an admin's "forbid Automatic" does not stop a
send that races the fetch. Ingest re-checks the seat and the membership
freshness when the upload lands, so nothing is credited after removal or
after a lapsed membership, even from an upload already in flight. The
Mac requests no claim while its org session has expired; traces stay
Held until the member signs in again.

### 5.4 What replaces what

| Today, per person | In an org |
|---|---|
| NEAR AI sign-in | Path A: still one sign-in. Path B: none. Path C: IdP sign-in. |
| Invite code | None; the org is the authority. |
| Choose spend organization | Pinned by policy. |
| Link a payout organization | Not offered to members; the holder account's link is the org's. |
| Choose data uses | Still the member's choice, within the org ceiling. |
| Arm folders / grant Automatic | Still the member's act. |
| Accept terms | Still each member's act (decision 8). |
| Claude / Codex logins, IronWire subscription consent | Unchanged and personal, unless the org provides an upstream (7.4). |

### 5.5 Managed configuration on macOS

A configuration profile with a managed-preferences payload for
`ai.tracecommons.shell`. Proposed keys, all non-secret except the last:

| Key | Type | Meaning |
|---|---|---|
| `OrgEnrolmentURL` | string | The org's enrolment link on the compiled channel origin. A link to any other origin is refused. |
| `OrgTenantHint` | string | The expected org tenant id. The client refuses an enrolment answer naming another tenant. |
| `EnrolmentPath` | `near_ai_membership`, `token`, `sso` | Which admit path first run uses. |
| `LockSignOut`, `LockUnenrol` | bool | Hide the two actions. |
| `EnrolmentToken` | string | Path B only. A bounded invite. |

Everything else is server policy, fetched after enrolment. The profile is
a bootstrap; it is not where policy lives, because a preference file is
readable by local processes and cannot be revoked.

Reading it: the Swift app reads forced values with `CFPreferences` (no
new dependency) and passes them through the existing
`tc_daemon_start_with_settings` settings JSON. The daemon treats them as
a `managed` block that the IPC `set_settings` cannot override. The CLI on
a managed Mac reads the same values through a small reader in
`trace-commons-contributor-platform`. **Not verified:** whether that
reader needs a plist dependency the workspace does not already have. If
it does, it goes through the dependency policy first; the fallback is
that only the app honours the profile on macOS and the CLI refuses
enrolment changes when the profile file exists, which is the pattern
`C/managed/opencode.rs:96-118` already uses.

An `EnrolmentToken` in a profile is readable by any local user. That is
acceptable only because of what it can do: admit one more seat of one
kind into one org, counted against `max_seats`, visible in the console,
and revocable. It cannot read anything.

### 5.6 Revocation and offboarding

| Event | Action | Existing piece |
|---|---|---|
| Member leaves | Admin removes the seat: revoke its device keys, its sessions, its contributor grant; close the account. | `/v1/admin/device-keys/{id}/revoke` (ING:15067 (r)); device removal revokes key, grants and native sessions and "a revoked key is never revived" (`INT/account_devices.rs:164-202` (r)). |
| Member removed from the NEAR AI org only | The next org renewal fails; session and upload claims stop within one fixed lifetime (path A). | **Proposed** org renewal route and issuer freshness rule (5.3). Not today's sliding session. |
| Lost Mac | Revoke that device key; the seat stays. | As above. |
| Enrolment token leaked | Revoke the authority; review seats admitted since. | `onboarding_invite_grants.revoked_at`. |
| Admin leaves | Another admin revokes the role. | New. A sole admin needs the operator. |
| Org closes | `status = closing`: no new claims; settle; pay out; retain per policy. | Tenant status. |

The NEAR AI inference key must also be revoked at NEAR AI. The owner
already decided sign-out revokes it (one-connected-account spec, decision
4 (r)); an admin-side removal cannot reach into the member's keychain, so
the org admin revokes it in NEAR AI, or the Commons does it with the
brokered credential of decision 2. In the MVP the console shows the key
id to revoke.

**What a departed member keeps.** Their credited traces stay in the
register and stay credited to the org. Their ability to withdraw goes
with the session. That is a loss of a contributor right the product
gives everyone else, so the design keeps a path: removal starts a
**closing window** during which the seat's session can still call
`POST /v1/account/traces/{id}/withdraw` and nothing else, and after it
the member withdraws through the operator. The length of the window, and
whether there is one, is decision 6.

## 6. Central crediting

### 6.1 The mechanism

One rule, applied where settlement keys are made: in an org tenant with
`credit_mode = org`, the settlement key is
`account:{holder_account_id}`; otherwise, as today,
`account:{account_id}` or the bare principal.

`settlement_group_key` (`SRV/db/mod.rs:2313`) is a pure function of a
principal and a principal-to-account map; it takes no tenant. It has
four call sites: the settlement route's grouping (ING:30609), the
per-account cap and its exclusion set (ING:30868, ING:30885), and the
versioned pipeline's `settlement_key_and_hold`
(`SRV/versioned_pipeline/service.rs:5423-5434`). So the rule goes in one
tenant-aware resolver that all four use, not in four places. It must
**not** go into `resolve_principals_to_accounts` or the
`trace_account_owned_principals` view, because other readers use those
to mean "this principal belongs to this account".

What else has to know about the holder, found by reading the paths:

- The pipeline path asks `resolve_payout_near_account_id` for an account
  key and records a hold when the account has no unambiguous NEAR payout
  target (`service.rs:5435` onward). The holder account needs a defined
  answer there, or every org line is held.
- The payout account predicate and V147's rule (5.2, step 3).
- The payout pass releases unproven legacy carries for every
  non-anchored tenant (`SRV/payout_pass.rs:801-806`). An `org-` tenant
  is non-anchored, so that branch runs for it; it should find nothing,
  and a test should say so.
- The per-account settlement cap (4.5).

Past those, the account-line machinery is reused as it is: batch line
items, the NEAR outbox row per credit account, holds,
`trace_payout_accruals`, and one live link per `(tenant, account,
rail)`. This was checked by reading the call sites above, not by
running anything; the settlement and payout PostgreSQL suites are where
"unchanged" gets proved.

The holder account is an ordinary row in `trace_accounts`; it has no
principal, no session and no device, so nothing can sign in as it and
`caller_credit_account_scope` can never resolve to it. Credit visibility
for members stays exactly as wide as it is today: their own seat.

The ledger is not touched. Rows keep `credit_account_ref` = the
submitting principal, so the ledger still records who earned what.

### 6.2 Why this is issuance, not transfer

Trace Credit is non-transferable. Under `credit_mode = org` the credit is
never the member's: the award's first and only holder is the org's
account, by a rule the member accepted before sending (the credit holder
is in `GrantTerms`, section 3.1). No row moves from one holder to
another. If the owner later wants members to hold credit and assign it
to the org, that is a transfer and a different instrument question; it
is not proposed.

For the planned NEP-141 rail the holder account is one token account
like any other: amounts are already `u128` atomic units with a decimals
field per instrument, and nothing here adds a float or a second unit.
The org needs a NEAR account to receive on that rail, which is a rail
link on the holder account when the rail exists.

### 6.3 Attribution and reporting

Attribution is a read, not a second ledger:

`trace_credit_ledger` (principal) joined to
`trace_account_owned_principals` (principal to seat account) joined to
`trace_org_seats` (kind, sealed label).

The org console reads, per seat and per period: traces sent, accepted,
quarantined, withdrawn; pending points; settled atomic units attributed;
last upload time. Per org: settled, paid per rail, held. Seats are shown
by their admin-supplied label, which the admin wrote and which is
unsealed only for an `org_admin` session of that org.

This is consistent with the hash-only rule: stored rows and logs still
carry no contributor identity. The label is sealed at rest like
`sealed_account_name`, and the report is an authenticated read by the
party that supplied the labels.

The settled figure per seat is an attribution of a shared line and must
be computed from the batch's `source_credit_event_ids`, not by dividing
the line, so that caps and holds applied to the org line are reflected
proportionally. Reversals attribute to the seat whose event was
reversed.

A member sees their own attributed figures through the existing
`GET /v1/account/credit-summary` (ING:8817 (r)), with a field saying the
holder is the org. Members do not see each other or the org total.

Reporting on people is a workplace matter as well as a data one. Whether
the per-member report exists at all, or only an org total plus per-server
rows, is decision 5.

### 6.4 Do members earn anything personally?

Not in the MVP. `credit_mode` reserves `member` (the org provides
credentials and policy; each seat settles to itself) and `split`. `split`
is not designed: it needs a ratio, two lines per event, and an answer on
whether the member's share needs the member's own payout link, which
needs the member's own NEAR AI organization.

### 6.5 Settlement, payout, redemption

- The holder account links `near_ai` to the bound organization (5.2). The
  payout pass pays it as it pays any account. D7 already says other
  members of that organization can spend the credit; for an org that is
  the point.
- A NEAR AI payout is parallel to Trace Credit and consumes none of it
  (D6). The org keeps its Trace Credit.
- Changing the payout recipient is the highest-value admin action. With
  no strong factor available (section 1), the MVP gates it on a fresh
  NEAR AI ownership proof for the **new** organization plus a cooldown,
  following the 7-day cooldown on login change (F10 spec (r)), and
  audits it. Decision 10 asks whether that is enough.
- Holds apply to the holder account, so a hold freezes the org's payout,
  not one seat's. A per-seat problem (one compromised server flooding)
  is handled by revoking the seat and, if needed, a reviewer abuse
  penalty on its events, which stays available without the central
  issuer allowlist (`docs/trace-commons.md:1026-1030`).

## 7. Servers

### 7.1 Identity

A server is a seat of kind `server`: its own account, its own device key,
no person, no session a human uses. Device keys stay per machine for the
reasons in 5.1; a fleet sharing one key could not be revoked one box at a
time and would attribute everything to one row.

Enrolment, in build order:

1. **Enrolment token.** `TRACE_COMMONS_INVITE` (exists) with a
   server-kind token. The machine generates its key and redeems once.
   Good for long-lived boxes.
2. **Workload identity.** A `workload_issuer` authority naming an
   issuer, audience and subject pattern: a cloud instance identity
   document, or a CI OIDC token such as a GitHub Actions token whose
   subject names the repository and workflow. The machine presents the
   token with its freshly generated public key and a signature; the
   server admits a seat bound to the token's subject. No secret is
   provisioned to the machine at all. The issuer's existing workload path
   (`SRV/trace_upload_claim_issuer.rs:1944-2030`) is the shape, but it
   verifies one operator-configured EdDSA key and takes `tenant_id` from
   the token; cloud and CI tokens are RS256 with their own claims. So
   this is a new verifier that maps a registered external issuer to an
   org tenant, and it must keep the existing rule that a workload subject
   can never be device-shaped.
3. **TEE attestation.** Where the workload runs in a TEE, a quote binding
   the device public key in report data, checked with
   `trace-commons-attestation` (TDX DCAP and measurement pins exist
   there, for verifying *servers*; no client attestation exists today
   (r)). Later, and only where an org actually runs agents in CVMs.

**Ephemeral runners.** A CI job that lives ten minutes should not leave a
permanent device row. For workload-identity seats the seat is the
*subject* (the repository and workflow, or the runner pool), and each job
registers a short-lived device key with an expiry under that seat. Device
rows past expiry are swept. Attribution is per seat, so it survives the
keys.

### 7.2 Consent

There is no person to click Ask. For a server seat the **org consents by
policy**: `consent_basis = org_policy`, recorded on the seat, on each
claim and on each submission. The org's admin accepts the terms for the
org's servers once, in the console, and that acceptance is what the
server daemon's terms hold reads.

This is the one place an admin's setting sends traces, and it is fenced:

- It applies only to seats of kind `server`, admitted by an authority
  whose `seat_kind` is `server`. The issuer refuses `org_policy` from a
  member seat.
- A Mac enrolled as a member cannot be switched to a server seat by
  policy. Re-enrolment as a server is a new device registration, visible
  in the console and in the member's app as a different state.
- The risk that an org enrols laptops as "servers" to get unattended
  sharing is real and is not preventable in software on machines the org
  controls. The terms an org accepts must forbid it, the console shows
  seat kinds, and the operator can see the ratio. Say this in the terms;
  do not pretend the software stops it.

**Whose traces are a shared runner's?** A CI runner acting on pull
requests by several people produces sessions prompted by text those
people wrote. The design's position:

- The traces are the org's. The org is the contributor of record and
  warrants it has the right to contribute them, including notice to its
  people. This is a terms question (decision 8), not a software one.
- The ledger records the **machine seat**, not a person. The source
  parsers carry no author field and the redactor exists to remove
  identity, so per-author attribution would mean deliberately carrying a
  human identifier through a pipeline built to strip it. Not proposed.
- An org that wants "which pipeline earned this" can set a label on the
  seat (one seat per runner pool or per repository under workload
  identity). A finer per-job label would need a new attribution field
  outside the redacted envelope; it is deferred until someone asks.
- A person whose prompt ran on a shared runner cannot withdraw that
  trace themselves, since they have no seat on it. The org admin can
  withdraw a server seat's traces (new route, server seats only). Admin
  withdrawal of a *member's* traces is not proposed.

### 7.3 Packaging

What runs on the server is the existing MIT/Apache contributor CLI and
daemon. **Redaction still happens on the server before anything is
uploaded**; the unattended mode changes who consents, not what is
scrubbed, and the commons still re-scrubs.

What is missing for an unattended, admin-managed install:

| Gap | Today | Proposal |
|---|---|---|
| Credential storage | Secret Service only; no fallback (r) | A sealed-file backend for server seats: the device key encrypted under a key from systemd credentials or a TPM where present, else a root-only 0600 file enabled only by an explicit `server` profile. This reverses "no plaintext fallback" for one profile, on purpose; decision 13. |
| Service unit | systemd user unit, needs linger (r) | A system unit with a dedicated user, `StateDirectory`, `LoadCredential` for the enrolment token. |
| Configuration | Flags, env and interactive steps | `/etc/trace-commons/managed.json`, root-owned, same keys as 5.5, plus `TRACE_COMMONS_*` env for containers. |
| Terms | IPC `terms_accept` only (r) | Server seats read the org's acceptance from policy. |
| Data uses | `consent-scopes-not-chosen` hold (r) | Server seats take scopes from policy at enrolment. |
| Arming | Per folder, or an IPC-only global override (r) | Policy `server_roots` arms the listed roots; nothing else. |
| Scrub-check holds | Wait for a person (r) | On a server, a held session is dropped after a period and counted, never sent. An org may route holds to an admin queue later; not in the MVP, because that queue would show content to an admin. |
| Container | None for the contributor (r) | A minimal image and a sidecar pattern: the daemon reads the agent's session directory from a shared volume. |
| Sources | Claude Code, Codex, Gemini CLI, Cline, OpenCode, trajectory (r) | Unchanged. There is no IronClaw session parser in this repository and the tree says it must not gain an Ironclaw tree (`crates/trace-commons-contributor-sources/src/routing/mod.rs:34-36` (r)); IronClaw would contribute through the protocol crate itself, as designed. |
| `daemon install` refusals | Refuses when the NEAR AI notice was never shown (r) | The server profile does not use the hosted classifier unless policy names it. |

### 7.4 Org-run inference (secondary)

The second reading of "servers": the org points its members' Private AI
at inference the org runs or pays for.

IronWire already models this. A backend is a declaration with a kind;
`openai-compatible` takes a `base_url` and an `api_key_env`
(`crates/ironwire_core/src/config.rs:1075-1094, 1246-1276` (r)). So org
policy can carry `upstreams`, a list of backend declarations the daemon
writes into the embedded proxy's configuration, and can pin which tools
route to which.

Two constraints:

- **Credentials.** A static key in policy would be a secret replicated
  to every Mac. The upstream should accept a per-device credential
  instead: the device presents a short-lived token minted by the Commons
  for the seat, and the org's gateway verifies it against the issuer's
  published keyset. That is new work on both sides and is why this is not
  in the MVP. An org that insists on a static key can set it by MDM as an
  environment value at its own risk; the product should not distribute
  it.
- **Attestation.** Receipt verification is NEAR AI specific
  (`crates/trace-commons-attestation/src/receipt.rs:1-25` (r)). Requests
  to an org-run upstream produce no receipt, so traces from them are
  uncertified for any purpose that reads `attested_inference`, and the
  app's Private AI copy, which is specific about NEAR AI, does not
  describe them. An org upstream needs its own name in the UI.

## 8. Admin surface

**Where.** Web pages and `/v1/org/*` JSON routes on the ingest service.
Ingest already serves account web pages and enforces a production CSP in
CI (the `wallet SDK and production CSP` job). No separate service: the
console needs the same database role, RLS context and audit writer.

**Auth.** An account session in the org tenant whose seat has
`org_role = org_admin`. Routes resolve the seat on every request; a
revoked role takes effect at once. The role is tenant-scoped by
construction: the session names the tenant, RLS does the rest.

With passkeys removed, the admin's session is as strong as their NEAR AI
sign-in. Sensitive actions (payout recipient, policy widening, creating
an authority, granting `org_admin`) require a NEAR AI token fresher than
a few minutes and are individually audited. See decision 10.

**Routes (proposed).**

| Route | Does |
|---|---|
| `GET /v1/org` | Org status, limits, policy digest. |
| `GET/PUT /v1/org/policy` | Read; write a new version. A widening change is flagged in the response and the audit row. |
| `GET /v1/org/seats`, `PATCH`, `DELETE /v1/org/seats/{account_id}` | List, relabel, remove. |
| `GET /v1/org/seats/{account_id}/devices`, `DELETE .../{device_key_id}` | Per-seat devices; revoke. |
| `POST/GET/DELETE /v1/org/authorities` | Enrolment authorities and tokens. A token is shown once. |
| `GET /v1/org/credit`, `GET /v1/org/credit/seats` | Org totals; per-seat attribution. |
| `GET/POST /v1/org/payout-rails/{rail}` | The holder account's link, with the ownership proof. |
| `POST /v1/org/seats/{account_id}/traces/withdraw` | Server seats only. |
| `GET /v1/org/audit` | The org's own audit rows. |

**What the console never has:** a preview, a redacted body, a review
queue, an export, a submission's text, another org.

**Audit.** Every route above appends one row (4.2). Reads of the
per-seat report are audited too (`org_report_read`), since it is a read
about people.

**In the Gateway app.** An admin signed in on a Mac sees the same fleet
read-only in the app: members and servers in one list, state, last
upload, attributed credit, with Remove and Revoke leading to the web
console for the confirmation. Building full management natively is
later; the lead client should not grow a second admin implementation
before the first one settles.

## 9. Client changes (macOS first)

macOS is the lead client and the parity target. Windows, Linux GTK and
Tauri are frozen and get nothing here; the Linux CLI and daemon get the
server profile (7.3).

**Managed mode.** The daemon has an `org` block: tenant, policy, policy
digest, credit holder, seat kind. Present means managed.

**First run** (`M/TCShellCore/FirstRun/FirstRunNavigation.swift:14` (r):
welcome, tools, privateAI, traces, uses, terms, join):

- The earnings step shows an estimate today. In an org the credit is not
  the member's, so that step needs different content or removal
  (decision 9 covers the wording).
- The uses step offers only the uses the org allows.
- Join becomes the org's admit path; there is no invite field and no
  payout step.

**Locked by policy** (shown as fixed values, not hidden, so the member
can see what was decided for them): spend organization; data-use
ceiling; `sharing = off`; Automatic unavailable; scrub check manual;
payout settings absent; sign out and unenrol per the two lock flags.

**Still the member's:** which tools and folders are watched; arming each
folder; which allowed uses to grant; reviewing, approving, dismissing,
keeping; withdrawing their own traces; Insights, which stays on the Mac
and is never reported to the org.

**New surfaces that need the owner's approval before they exist.** Rule
9 of `docs/ui-copy-guide.md` forbids a new banner, card, one-time
message, notification or status channel without explicit approval of
both the notice and its wording. This design needs the member to learn
three things, and proposes none of the surfaces as settled:

1. that this Mac is managed by an org and credit goes to the org;
2. that standing Automatic grants were voided because the Mac joined an
   org or the org loosened policy (the existing grant-void notice exists
   for term changes (r); whether it may be reused with new wording is the
   question);
3. that the member has been removed and has a window to withdraw.

Each is listed in decision 9. The "not silent" answer in 3.1 depends on
at least the first, so that decision gates the feature, not just its
polish.

**Copy.** All new strings go in the Rust copy tables (guide rule 8), use
"trace", "Private AI" only for NEAR AI routing, and the mode names Ask,
Automatic, Never.

### 9.1 Distribution: one app, one download

Owner question, 2026-11-11: "how do we distribute this app without making
it confusing for individual prosumer users". Proposed answer: the
enterprise version is **a state the app enters, not a second product**.

- **One app, one name, one download.** The same signed `Near.ai.app`, the
  same release feed, the same `/download/` page on the marketing site. A
  person who downloads it for themselves sees today's first run and never
  meets an org concept: no "personal or team?" fork, no admin wording, no
  org pricing on the download path.
- **An org puts a Mac into managed mode in one of two ways, and only
  these:**
  1. **Device management.** IT deploys the same installer plus a
     configuration profile (5.5) through Jamf, Kandji, Intune or similar.
     The profile carries the enrolment link and locks; first run starts
     at the org's admit path.
  2. **An enrolment link** for orgs without device management, sent by
     the admin. It opens the app's join step through the existing
     `tracecommons://enroll?invite=` handler (`C/deep_link.rs:40` (r)) or
     an https link on the channel origin that hands off to it.
  There is no in-app "Join your organization" entry for a person who has
  neither. (The flow model in `docs/flows/enterprise-seat/` draws that
  control on the personal screen; read it as "reached through a link or
  profile".)
- **Servers** get the Linux CLI and daemon (7.3), not the Mac app, with
  an enrolment token. They have their own install page for admins.
- **Admins** get a separate "For teams" page: the org console, a profile
  template, the server install. It is linked from the site's footer and
  from the console, not from the hero or the download page.
- **No separate "for Teams" bundle.** A second bundle id would read more
  clearly on an IT allow-list, but it doubles release and update
  channels, and the two would still collide on one Mac, because the
  state directory is one enrolment (decision 4).

**The overlap is the confusing moment, and it needs owner-approved
wording.** A person already using Gateway personally whose employer then
manages their Mac, or who follows an org link, crosses from personal to
managed. The design's answers are fixed elsewhere (one enrolment per Mac;
joining returns every folder to Ask; credit earned personally stays
theirs), but the person must be told, before they join, that new credit
goes to the org and their folders reset. That is a rule-9 surface and
belongs to decision 9.

The flow model (`docs/flows/enterprise-seat/`, promise 8) found one gap
at this crossing: a trace credited to the personal account, including one whose
upload was still in flight at Join, has no Withdraw in the app once the
Mac is managed. The owner's fix is the "Before you joined" list
(decision 4, 11.1 change B).

## 10. Licensing

| Piece | Crate | Licence |
|---|---|---|
| Org tables, store functions, settlement arm, admission arm, verifiers | `trace-commons-server` | AGPL-3.0-or-later |
| `/v1/org/*` routes and console pages | `trace-commons-ingest` | AGPL-3.0-or-later |
| Org enrolment in the issuer | `trace-commons-server` (issuer binary) | AGPL-3.0-or-later |
| `OrgPolicy`, enrolment request and response types, `consent_basis`, managed-configuration schema | `trace-commons-protocol` | MIT OR Apache-2.0 |
| Managed-configuration reader, sealed-file credential backend | `trace-commons-contributor-platform` | MIT OR Apache-2.0 |
| Managed mode, policy enforcement, server profile, system unit | `trace-commons-contributor` | MIT OR Apache-2.0 |
| C ABI additions | `trace-commons-contributor-ffi` | MIT OR Apache-2.0 |
| macOS UI | `macos/` | MIT OR Apache-2.0 |
| IronWire upstream declarations | `ironwire_*` (copyright NEAR AI) | MIT OR Apache-2.0 |

The boundary holds: every shared type is in the protocol crate and the
AGPL side depends on it. No permissive crate gains a dependency on
`-server`, `-ingest`, `-gate-api` or `-gate-enclave`, and
`crates/trace-commons-server/tests/license_boundary.rs` needs no change
to its expected sets. New `.rs` files in the AGPL crates carry the
two-line header.

**AGPL section 13.** The org console is part of the ingest service, so
`GET /v1/source` (unauthenticated, outside every gate) already covers it.
Org admins and members are network users of the service and are owed the
Corresponding Source by the same route. Nothing new is needed as long as
the console ships from this tree.

**An enterprise edition and the proprietary seam.**

- Offered as a hosted service by the copyright holder, the enterprise
  features can ship from this tree under AGPL with no further question.
- A customer that wants to run its **own** commons for its org gets AGPL
  terms: it must offer its users the source of what it runs, including
  its modifications. The alternative is a commercial licence from the
  copyright holder, which the inbound MIT/Apache contribution term keeps
  possible.
- If any enterprise feature were to be closed (an SSO or SCIM connector,
  say), only the copyright holder could do that, and it would need a
  seam like `trace-commons-gate-api`'s: a trait in an AGPL crate that a
  proprietary backend implements. This design proposes **no** such seam
  and keeps everything in tree. If the owner wants one, the enrolment
  authority verifier is the natural place (decision 11).
- Everything installed on a member's Mac or an org's server stays
  permissive, so an org can embed or repackage the client.

## 11. Security and threat model

Deltas against the checklist in `docs/trace-commons.md:1236-1280`.

| Threat | Outcome with this design | Residual |
|---|---|---|
| **Compromised admin session** | Can read seat labels and figures; remove seats; loosen policy up to the operator-set ceilings; create authorities and admit attacker machines as seats; redirect future payouts after proof and cooldown. Cannot read any trace content, export, review, or reach another org. Loosening voids member grants, so it does not start uploads from people. | Attacker-admitted **server** seats can upload attacker-chosen content and earn the org credit, or pollute its record. Bounded by `server_limit`, the org admission budget, the gates, and the audit trail. Payout redirection is the money risk; see decision 10. |
| **Malicious admin** | Same powers, used on purpose. Cannot arm a member's folder, widen a member's chosen uses, or see content through the product. | Can enrol laptops as servers (7.2); has root via MDM regardless. Contractual, and visible in the console to the operator. |
| **Leaked enrolment token** | Admits seats of one kind into one org until `max_seats`, expiry or revocation. Grants no read. A member-kind seat admitted this way can upload its own content toward the org's credit. | Same as attacker-admitted seats. Prefer path A or workload identity, which have no token. |
| **Leaked device key** | One seat's uploads until revoked. | As today. |
| **Member leaves** | Seat removed; keys, sessions and grant revoked; the NEAR AI key revoked at NEAR AI. Under path A with the proposed non-sliding org session and issuer freshness rule (5.3), removal from the NEAR AI org alone stops the session and upload claims within one fixed lifetime. | Up to one lifetime of uploads after NEAR AI removal if the admin forgets the Commons side. Without the two proposed rules the bound is today's 30-day sliding cap, which is not acceptable for offboarding. Raw traces on the Mac are the org's device-management problem. |
| **Member against the org** | A member can dismiss or never arm; cannot see colleagues' traces or figures; cannot redirect credit. A member can withdraw their own traces, which reverses org credit. | Intended. |
| **Cross-org isolation** | The tenant boundary and forced RLS. `near_ai_org_index` is unique, so two Commons orgs cannot bind one NEAR AI org. A person in two orgs holds two unrelated seats on two devices. | Dedup and novelty are per tenant, so org A's content does not suppress org B's. The gates' cross-tenant behaviour is unchanged from today. |
| **Org console as an oracle** | Seat routes are tenant-scoped; unknown ids answer like absent ones, following the existing rule for submission status (`docs/trace-commons.md:206-208`). | -- |
| **Envelope fields as authority** | Unchanged: `consent_basis`, tenant and seat come from the claim, never the envelope. | -- |
| **Consent widening** | Unchanged: the only path that adds a use to a stored trace is the contributor's own model-training grant. For a member seat that stays the member's. For a server seat the caller is the org; whether an `org_admin` may make that grant for server seats is decision 7. | -- |

**Fail-closed rules.**

- A tenant in the `org-` namespace with no `trace_orgs` row, or a
  non-`active` status, refuses claims and admission with
  `org_record_missing` / `org_suspended`. It never falls through to the
  ungated legacy path.
- An org with `credit_mode = org` and no `holder_account_id`, or a holder
  account that is closed, blocks settlement for that tenant with a named
  blocker. It never settles to the seats instead.
- The holder arm active with no org settlement cap configured is a
  blocker (4.5).
- A device that is managed (profile present, or an `org` block in
  config) and cannot fetch or verify its policy does not upload. It does
  not fall back to unmanaged behaviour or to the last looser policy; a
  cached policy may be used only until a stated age.
- A claim with no `consent_basis` from a seat in an org tenant is
  refused.
- **No scope floor in an org tenant.** A device in an `org-` tenant with
  no active contributor grant gets no claim. The issuer's hardcoded
  floor (`SRV/trace_upload_claim_issuer.rs:2040-2077`) never applies
  there, and neither does the NEAR AI sign-up grant backfill. Without
  this rule, removing a seat's grant or setting `sharing = off` would
  leave it able to upload under the floor.
- A member seat whose last membership proof is older than the org
  session lifetime gets no claim (5.3).
- A managed profile naming an enrolment origin outside the compiled
  channel is refused, with no prompt to override.
- An upstream declared by policy whose credential is missing is not
  routed to; the tool keeps its previous route, and nothing is sent to a
  default.

**Hash-only.** No org name, admin identity, seat label, NEAR AI org id,
IdP issuer URL, enrolment token or workload subject in any log line or
unsealed column. Labels are sealed; issuers and subjects are hashed;
audit metadata is counts, digests and hashed seat references, never a
raw tenant or account id.

### 11.1 Flow model and its promises

`docs/flows/enterprise-seat/` holds a Quint model of one member's Mac and
one server seat through the life of an org, checked by TLC on all
1,217,568 of its states, with mocks of each state and the chart. Eleven
promises, in the owner's words, all hold:

1. Joining never carries your Automatic over.
2. Your traces leave only by your act.
3. If the org forbids Automatic, nothing of yours goes automatically.
4. Sharing off means no new credit.
5. Removal stops uploads, including one already in flight.
6. Leaving the NEAR AI org stops uploads by your next sign-in.
7. You can take back what you gave the org.
8. What you earned personally stays withdrawable.
9. Servers share only under the org's consent.
10. The org closes cleanly.
11. A submitted trace eventually lands, if you stay a member, keep
    signing in, sharing stays on, the network works, the org stops
    changing policy and your sign-in eventually lasts long enough.

Three promises broke against this spec as first written, and each
counterexample changed the design (owner's picks, 2026-11-11):

- **A (promise 3).** The claim carries the policy digest and how the
  trace was sent; the issuer refuses a stale digest or a forbidden
  Automatic (5.3, 4.4).
- **B (promise 8).** A "Before you joined" list keeps Withdraw for
  personally credited traces (decision 4).
- **C (promise 11).** The Mac sends nothing while its org session has
  expired (5.3).

The owner also added the last two conditions of promise 11; the README
gives the paths, what the model leaves out, and the command that checks
it again.

## 12. Phasing

**Phase 1 -- MVP: both capabilities, least new surface.**

- `trace_orgs`, `trace_org_seats`, `trace_org_enrolment_authorities`;
  `org_admin` seat role; org arms in admission and settlement (one
  tenant-aware settlement-key resolver); org cap; no scope floor and a
  membership-freshness rule in the issuer for org tenants.
- Operator-created orgs. Admin binds the NEAR AI org and the holder's
  payout link with the existing ownership proof.
- Member admit path A (NEAR AI org membership). One sign-in, nothing
  else to configure.
- Org policy with the server-enforced parts first: `sharing`, use
  ceilings, pinned spend org. `consent_basis` on the claim.
- Console: seats, devices, policy, credit totals, per-seat attribution
  (subject to decision 5), audit.
- macOS managed mode, the managed-preferences bootstrap, locked settings.
  The three surfaces of section 9 only as approved.
- Distribution as in 9.1: the same app and download, a configuration
  profile template, org enrolment links, and an admin-facing "For teams"
  page.
- Servers by enrolment token on long-lived Linux boxes: server profile,
  system unit, sealed-file credential backend, policy-armed roots,
  org terms acceptance.

**Phase 2 -- fleets and zero-touch.**

- Workload-identity enrolment (CI OIDC, cloud instance identity) and
  ephemeral device keys. Container image.
- SSO (path C) and automatic offboarding from the IdP.
- Fleet view in the Gateway app for admins.
- Self-serve org creation, if wanted.

**Phase 3 -- inference and instruments.**

- Org-run upstreams with per-device credentials (7.4).
- Brokered NEAR AI key issuance (decision 2), if NEAR AI offers an org
  credential that makes it safe.
- `credit_mode = member` and `split`.
- NEP-141 rail link for the holder account, when that rail exists.
- TEE-attested server enrolment.

Each phase is its own plan. Phase 1 is roughly: one migration set, the
settlement and admission arms with their PostgreSQL suites, the issuer
enrolment arm, the console, the daemon's managed mode, the macOS screens,
the server profile.

## 13. What this does not do

- No on-premises commons. Orgs use the hosted Commons.
- No admin view of trace content, and no admin review queue.
- No per-author attribution on shared machines.
- No SAML.
- No change to the gates, the envelope, or the redactor.
- No Windows, GTK or Tauri work.
- No migration of a person's existing personal account into an org.

## 14. Open decisions for Zaki

Each has a recommendation. The first five change the shape of the
design; the rest change a section.

1. **Is the NEAR AI organization the org's root of identity?**
   Recommendation: yes for the MVP. It gives admin proof (role `owner`),
   member proof (active membership), central inference billing and the
   payout recipient from one check the server already makes, with no
   IdP integration and no org secret held by the Commons. Trade-off: the
   product inherits NEAR AI's membership model and whatever SSO it has,
   members still do one sign-in, and an org that does not want NEAR AI
   inference still needs a NEAR AI org. The alternative is SSO first,
   which is a new verifier, new dependencies to vet, and no answer for
   inference keys.

2. **Zero sign-in for members: may the Commons hold an org NEAR AI
   credential and mint per-device inference keys?** Recommendation: no,
   not now. It would make the Commons a custodian of a credential that
   spends the org's money, a larger change to the threat model than
   everything else here together. Revisit if NEAR AI offers a scoped
   org credential that can only mint workspace keys. Trade-off: without
   it, "the admin sets credentials for everyone" still means each member
   signs in once. **Not verified:** whether NEAR AI's API offers any
   org-level service credential; the tree only shows keys minted with a
   person's session.

3. **Who holds the credit: the org only, or the org and the member?**
   Recommendation: org only (`credit_mode = org`). It is one settlement
   arm, and it keeps the instrument non-transferable without argument.
   Trade-off: a member has no personal stake, which may lower how many
   folders people arm. `member` mode (org sets credentials, people keep
   credit) is cheap to add; `split` is not.

4. **One Mac, personal and org at once?** Recommendation: no. One
   enrolment per Mac; personal contribution happens on a personal
   machine. Trade-off: people with one laptop for both lose their
   personal account on it while it is managed. Supporting both means two
   state directories, two device keys and a per-folder choice of
   destination, and a way to keep org-policy folders from being sent
   personally. It is a design of its own.
   One exception, from the flow model (11.1, change B): the Mac keeps a
   withdraw-only credential for the personal account, so traces credited
   to it, including one whose upload was in flight at Join, keep Withdraw
   under "Before you joined". Without it, joining an org removes a
   contributor right.

5. **Does the admin see per-member figures, or only the org total and
   per-server rows?** Recommendation: per-seat counts and attributed
   credit, visible to the member as exactly what the admin sees, and
   switchable off per org. Trade-off: a per-person activity report is
   workplace monitoring in some jurisdictions and will meet works-council
   and privacy objections; org-total-only avoids that but removes the
   org's ability to see which seats are misconfigured or idle.

6. **A departed member's right to withdraw.** Recommendation: a 30-day
   closing window in which the seat can only withdraw, then the operator.
   Trade-off: a window keeps a session alive for someone the org has
   removed; no window removes a right every other contributor has.

7. **May an org admin withdraw traces, or add the model-training use,
   on the org's behalf?** Recommendation: both for server seats, where
   the org is the contributor; neither for member seats. Trade-off: an
   org cannot recall a member's trace that leaked something of the
   org's; it must ask the member or the operator. Allowing admin
   withdrawal of member traces is a deletion, not an exfiltration, so it
   is the safer of the two to grant later.

8. **Terms and consent of record.** Recommendation: the org accepts org
   terms once (covering servers, the warranty that it may contribute
   machine traces, the prohibition on enrolling people's machines as
   servers); each member still accepts the member terms on their own
   Mac. Trade-off: the per-member step is the one piece of friction an
   admin cannot remove. This needs counsel, and the current terms are a
   placeholder "not yet in effect" (1.0 announcement).

9. **The three member-facing surfaces and their wording** (managed
   indicator, grants voided on joining or loosening, removal window).
   Recommendation: approve the first as a permanent row in Settings and
   on the Traces tab rather than a banner, reuse the existing grant-void
   surface for the second with new copy, and decide the third with
   decision 6. None is designed as settled; rule 9 applies to all three.
   Trade-off: without the first, the "not silent" answer in 3.1 has
   nothing behind it and the feature should not ship.

10. **How strong must admin authentication be?** Recommendation for the
    MVP: NEAR AI session, a fresh ownership proof for sensitive actions,
    a cooldown on payout-recipient change, and at least two admins
    required before a recipient change takes effect once an org has two.
    Trade-off: passkeys were removed on the owner's instruction, so there
    is no phishing-resistant factor anywhere; an org console is where its
    absence costs most. Reintroducing a passkey for `org_admin` only
    would be the strong answer and reverses a recent decision.

11. **Open or closed enterprise features?** Recommendation: all in tree
    under AGPL, sold as a hosted service, with a commercial licence for
    self-hosting. Trade-off: no technical moat around SSO/SCIM. If a
    closed connector is wanted, define the enrolment-authority verifier
    as a trait in an AGPL crate now so it can be one later.

12. **Who creates orgs?** Recommendation: the operator, by hand, for the
    first orgs, with `seat_limit`, `server_limit`, use ceilings and the
    settlement cap set per org. Trade-off: no self-serve sign-up; in
    exchange every org is vetted before it gets `invited`-class
    admission with no attested-inference requirement.

13. **A file-backed device key on servers.** Recommendation: allow it
    for the `server` profile only, sealed with systemd credentials or a
    TPM where available and a root-only file otherwise. Trade-off: it
    reverses the documented "no plaintext fallback" rule
    (`docs/operator/commons-credential-storage.md`) for one profile.
    Without it, headless Linux needs a keyring daemon inside a D-Bus
    session, which is not how servers or containers are run.

14. **Unattended scrub-check holds on servers.** Recommendation: drop
    and count them. Trade-off: some share of server traces is lost,
    probably the more unusual ones. The alternative, an admin queue,
    puts trace content in front of an admin and breaks the central
    promise of section 3.1.

## 15. Not verified

Stated so nobody builds on them as facts.

- Whether fixed-tenant invite onboarding creates an account or a bare
  principal (4.2).
- Whether NEAR AI `/users/me` reports non-owner members with a role this
  design can read as "active member". The verifier's struct has `role`
  and `is_active` per org and the code tests only `owner`
  (`SRV/payout_recipient_proof.rs:176-186, 281-282`); the role values
  for ordinary members were not seen.
- Whether NEAR AI offers org-level service credentials or org-managed
  SSO (decisions 1, 2).
- Whether reading managed preferences from Rust needs a new dependency
  (5.5).
- Whether `jsonwebtoken` as built here can verify RS256/ES256 for OIDC,
  or whether features or a dependency must change (5.3 path C, 7.1).
- The measured effect of same-tenant dedup on an org of people working
  in the same repositories (4.5).
- Every citation marked (r).
- A client comment says a native session "is never extended"
  (`C/daemon/account_session_renewal.rs:4` (r)); the server code slides
  it to a 30-day cap (`SRV/account_native_auth.rs:66-94`, re-read). This
  spec follows the server code. The comment looks stale and was not
  chased further.
- Whether every settlement and payout path honours the holder key was
  checked by reading four call sites and three downstream readers (6.1),
  not by a test. Other readers of settlement keys may exist.
