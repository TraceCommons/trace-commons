# Pilot cutover to main, 2026-09

This runbook moves the pilot from build `5f239be4` at schema V74 to current
`main` plus the server PRs landing with it (#1071, #1073, #1077, #1078,
#1084, #1085, #1086, #1087), schema V89. It records a go/no-go for released
contributor clients, what was tested and how, and the order of operations.

It was written from a local rehearsal, not from the pilot. Nothing here was
run against the pilot or any remote system.

## Decision

**Go, with conditions.** Released clients (0.12.x, latest `contributor-v0.12.6`)
keep enrolling, submitting, reading status and withdrawing after the cutover,
**provided the runtime grants in [Required grants](#required-grants) are
applied in the same step as the migrations**. Without them, every new upload
and every withdrawal from a released client fails with a 500. That was
reproduced; see [Evidence](#evidence).

All new feature switches stay off at cutover. Two of them must not be turned
on until later work lands:

- **Account admission** (`TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED`) refuses
  every new upload from a released client on a `near-`/`nearai-` tenant.
  Legacy `tenant-…` invite identities, including pooled Devfolio devices, are
  unaffected (tested).
- **Witness certificate v2** is not understood by 0.12.x clients.

The rehearsal found two operator-side problems. Neither affects clients, and
both are fixed when the target build includes #1095 and #1100:

- **Drills after the cutover.** Before #1095, the audit-chain and
  db-reconciliation drills reported `ready: false` after the first
  post-cutover event, for every tenant with pre-cutover audit rows.
  `scripts/operator/smoke-gate.sh` requires both drills, so it failed. With
  #1095, the pre-cutover rows are reported as a legacy prefix
  (`db_legacy_prefix_event_count`, `db_audit_legacy_prefix_row_count`), not as
  failures.
- **Binary rollback.** Before #1100, a rollback was one-way. With #1100, rolling
  forward again needs one reviewed repair per tenant that was active during
  the rollback. See [Rollback](#rollback).

Without them in the target, treat both as described in
[What breaks](#what-breaks-and-the-mitigation).

## Scope

| | |
|---|---|
| From | build `5f239be4` (deployed 2026-09-22), schema V74 |
| To | `main` plus #1071, #1073, #1077, #1078, #1084, #1085, #1086, #1087 |
| New migrations | V75–V82 and V84–V89: fourteen files. There is no V83 (the runner tolerates gaps). |
| Binaries | ingest **and** issuer. The issuer changed (#1015, #1086), so deploy `both`. |
| Switches, all default off | `TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED`, `TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED`, `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON`/`_VERSION`, `TRACE_COMMONS_INFERENCE_CONNECTION_CATALOG_JSON`. The witness's certificate profile defaults to `v1`. |

Where each default is set:

- **Account admission.** Absent, `false` or `0` is off, and anything else
  refuses boot: `admission.rs` `account_config_from_values`.
- **Legacy invite link.** Off unless set to `true` or `1`; on without the
  attestation key refuses boot: `legacy_invite_link.rs:154-182`.
- **Earned-trust shadow policy.** Both variables absent is off; one without the
  other refuses boot: `account_trust_growth.rs` `shadow_policy_from_values`
  (#1084).
- **Witness certificate profile.** `default_value = "v1"`:
  `trace-commons-witness.rs:137`.

**Merge-order note.** #1073 (V82) and #1087 (V84–V88) both append to the
`MIGRATIONS` list in `db/postgres.rs`, and they conflict there. #1077 (V89)
conflicts with each of them in the same place. Keep the list in version order:
V81, V82, V84, …, V89. Each migration commits by itself, so if a deploy
happens between merges, the database applies them in merge order. That is
harmless here: none of the three depends on the others. #1073 also conflicts
with #1087 in `tests/account_merge_pg.rs`.

## Released clients: what changed on the routes they call

The 0.12.6 client calls these server routes:

- **issuer:** `/v1/onboard`, `/v1/enroll`, `/v1/trace-upload-claim`
- **ingest:**
  - `/v1/traces`
  - `/v1/contributors/me/submission-status` and `/me/score-attestation`
  - `/v1/community/profile`, `/v1/community/leaderboard`
  - `/v1/account/login-links`, `/v1/account/native/{authorize,token}`,
    `/v1/account/logout`
  - `/v1/account/traces/{id}/withdraw`
  - `/v1/account/near/provision/*`, `/v1/account/near-ai/provision/*`
  - `/v1/admission/challenge`
- **witness CVM:** `/v1/witness`, `/v1/witness/admission`, `/v1/attestation*`

No route was removed or renamed. Everything the diff adds is a new route
(`/v1/account/contribution-status`, `/v1/account/invites/redeem`,
`/v1/account/source-sessions/status`, the legacy-link and
inference-connection routes, the `/v2` NEAR provisioning routes,
`/v1/admin/rederive-dedup`, `/v1/admin/audit-chain-repair`) or an optional
envelope field (`source_session`).

| Route / behaviour | Verdict | Evidence |
|---|---|---|
| `POST /v1/traces`, new submission | **Breaks without the V78 grants** (500). Unchanged with them. | Every submission write takes `FOR UPDATE` on the source-session row (`trace_corpus_pg.rs:1643`, called from `upsert_submission_and_witness_evidence`, `:7150`). V78 grants the runtime nothing. |
| `POST /v1/traces`, re-POST of an existing submission | **Breaks without the V76 grant** (500); 200 with it | `reject_conflicting_witness_retry` (`trace-commons-ingest.rs:13352`) reads `trace_witness_certificate_evidence` (`trace_corpus_pg.rs:2069`). |
| `POST /v1/traces`, re-POST with *different* witness headers | New `409 witness evidence conflict` | Only when the first POST was witnessed and stored by the new build. 0.12.6 caches the certificate with the approved envelope and re-sends the same headers, so it is not expected to hit this. |
| `POST /v1/traces`, idempotent receipt | Additive | The duplicate's audit event is now mirrored to the DB, not written file-only. |
| Source-session binding (#1021) with account admission **off** | No requirement | `source_session` is required only in `reserve_account` (`admission.rs:466`, `:649`), which is reached only when account admission is on **and** the tenant is `near-`/`nearai-` (`:316`). |
| `POST /v1/account/traces/{id}/withdraw` | **Breaks without the V78 grants**, and without the token-bundle grant, which it already needs today | `withdraw_trace_source_session` reads `trace_submission_sessions` (`trace_corpus_pg.rs:1748`). The token-bundle step is `pending_token_bundle_deletions` (`trace-commons-ingest.rs:17158`, `db/token_bundles.rs:274`) plus the invoker-rights trigger `trace_revoke_token_bundles` (V65/V66/V68). |
| Withdraw response `credit_retained` | Semantics changed (#990) | It can now be `false`, when unsettled credit is forfeited. 0.12.6 does not read it. |
| `submission-status`, `score-attestation`, community, login links, native auth, v1 NEAR provisioning | Unchanged or additive fields | The v1 provisioning contract is unchanged. The `/v2` routes are new, and the new refusal labels (#1085) do not reach the wire. |
| Issuer `/v1/onboard`, `/v1/trace-upload-claim` | Unchanged for `tenant-…` invites | #1015 refuses invites with a *fixed* tenant in `near-`/`nearai-` (mint, import, and registry lookup). #1086 adds a route. |
| Witness certificate acceptance (#1017/#1018) | Unchanged for v1 | The v1 signing bytes are frozen (`certificate.rs` test). The new build also accepts v2. Verified certificates are now persisted, which needs the V76 grant. The pin is now read even when the bypass switch is off. |

A pre-existing client defect affects the smoke test, not the cutover.
`account login` in 0.12.6 prints `<ingest_url>/account/login?...`. The
onboarding response's `ingest_url` carries `/v1/traces`, so that URL 404s on
the current build and on the new one. The server path is `/account/login`.
It is still open in `crates/trace-commons-contributor/src/account_auth.rs`
(`browser_url`).

## Required grants

The pilot's ingest login (`tc_ingest_runtime_login`) holds its table grants
through the hand-made group `trace_ingest_runtime`. `pg_default_acl` is empty,
so a table a migration creates is invisible to ingest until someone grants on
it. Run this as the migrator (`app`) in the same session that applies the
migrations, after V89 and before the new ingest starts:

```sql
-- V78: submissions, status changes, object-ref/derived/vector/export writes
-- and withdrawal all touch the source-session tables. V78 grants the runtime
-- nothing. Without these, every new upload and every withdrawal returns 500.
GRANT SELECT ON trace_submission_sessions TO trace_ingest_runtime;
GRANT SELECT, UPDATE (withdrawn_at) ON trace_source_sessions TO trace_ingest_runtime;

-- V76: persisted witness evidence, written for each verified certificate and
-- read on every re-POST of an existing submission. Documented in
-- attested-inference.md, "Z2 provenance capture and rollout".
GRANT trace_witness_evidence_runtime TO trace_ingest_runtime;

-- Needed since V65-V68, not a cutover regression; withdrawal fails without
-- it on the current build too. The trigger trace_revoke_token_bundles() on
-- trace_submissions (fired when a row is revoked, withdrawn, purged or
-- rescrubbed) and the withdraw handler both touch the token tables through
-- the runtime.
GRANT SELECT, UPDATE ON trace_token_bundles, trace_token_attachments TO trace_ingest_runtime;
```

Switching account admission on (later, see below) also needs `INSERT` on
both `trace_submission_sessions` and `trace_source_sessions`. Its source-session
claim (`claim_trace_source_session`) inserts into both, and runs only on the
account-admission path.

Post-check, as `tc_ingest_runtime_login`:

```sql
SELECT has_table_privilege('trace_submission_sessions','SELECT'),
       has_column_privilege('trace_source_sessions','withdrawn_at','UPDATE'),
       pg_has_role('trace_witness_evidence_runtime','MEMBER'),
       has_table_privilege('trace_token_bundles','UPDATE');   -- all t
```

## Pre-checks

Read the running process, not the config files. See `deployment.md` and the
pilot notes on `/proc/<MainPID>/environ`.

1. `git diff --name-only 5f239be4 <target> -- migrations` lists fourteen files,
   V75–V82 and V84–V89. A plain install over unapplied migrations crash-loops
   the pilot, because the runtime login cannot run DDL.
2. `dig issuer.tracecommons.ai` resolves. Ingest fails closed at boot without
   the keyset.
3. The running ingest has `TRACE_COMMONS_DB_DUAL_WRITE=true`,
   `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true`, `TRACE_COMMONS_ADMISSION_ENABLED`
   (legacy, as today), and none of the new switches.
4. As `app`, `SELECT max(version), count(*) FROM _trace_commons_migrations`
   returns `74`. Every recorded name matches `main`'s `MIGRATIONS` list, so the
   #1028 boot check ("version recorded under another name") passes. The Route B
   scripts inserted the list names.
5. No invite has a fixed tenant in the reserved namespaces. #1015 stops those
   redeeming. Run this as the invite registry role or `app`:
   `SELECT count(*) FROM onboarding_invites WHERE tenant_mode = 'fixed' AND fixed_tenant_id ~* '^(near|nearai)-';`
   The result should be 0. Also grep the file allowlist for a `"tenant_id": "near` entry.
6. Disk and binary backups are in place, as in `deployment.md`.

## Backups

Take a Cloud SQL on-demand backup, as for V63–V74. **Do not rely on `pg_dump`
as `app`.** Under forced RLS it stops with `query would be affected by
row-level security policy`. With `--enable-row-security` it silently dumps
zero tenant rows. Both were observed in the rehearsal.

## Migrations (Route B)

Apply V75–V89 as `app` before touching the binaries. Run each file together
with its `INSERT INTO _trace_commons_migrations (version, name)` in one
transaction, in `MIGRATIONS` order, under the runner's advisory lock. Rehearse
first, with rollback.

- **Re-pin the Route B script.** The existing one is pinned to V74 and must be
  generalised, and its pins taken from the target commit.
- **Put the grants above in the same session**, after V89.
- **The old build keeps serving.** The rehearsal ran `5f239be4` on the V89
  schema: enrolment, submissions, status and re-POST all passed. There is no
  outage window.
- **Locks.** Each migration took 0.02–0.04 s on the rehearsal database. Only
  V89 touches a live, populated table. It runs `ALTER TABLE trace_credit_ledger
  ADD COLUMN … CHECK`: an ACCESS EXCLUSIVE lock and one validating scan, with
  no rewrite. V77 and V81 add policies to `device_keys`, `trace_accounts`,
  `trace_account_principals`, `trace_near_provisioned_devices`,
  `onboarding_invites` and `onboarding_invite_grants`. Those are brief
  catalog locks. Everything else creates new, empty objects.
- **The issuer migrates at boot as `app`.** Installing the new issuer before
  Route B applies all fourteen migrations there and then, without the grants.
  Route B first makes the issuer's boot a no-op.

Post-check as `app`:

- `max(version) = 89`, and `count(*)` is exactly 14 more than before.
- The roles `trace_witness_evidence_runtime`, `trace_account_admission_runtime`,
  `trace_account_trust_worker` and `trace_legacy_invite_link_guard` exist.
- The grant post-check above returns all `t`.

## Deploy

1. Build the target commit with Cloud Build, pinned to the tag (see
   `deployment.md`).
2. Point both `latest.txt` files at the tag, or always pass the tag.
3. `pull-and-install.sh both <tag>`. The issuer goes first, and its boot
   migration is a no-op.
4. Verify:
   - `/health` reports the tag as `build_commit` on `127.0.0.1:3907`;
   - `systemctl is-active`;
   - no `ERROR` in `/var/log/tracecommons/ingest.log`;
   - the privacy-filter line is present.

## Smoke test with a released client

Use `contributor-v0.12.6` on a throwaway invite (an individual code with
`max_uses` 1) and a synthetic trajectory file. **Always pass
`--source trajectory --trajectory <file>`.** A bare `submit --trajectory`
still discovers and uploads the machine's real Claude Code and Codex
sessions.

1. `login --invite https://issuer…/onboard#CODE --default` succeeds and prints
   the tenant.
2. `submit --source trajectory --trajectory t.jsonl --yes` gives
   `outcome: submitted, status: accepted`.
3. `status` lists the submission.
4. Re-POST: move `receipts.jsonl` aside, submit the same file, and restore the
   file. Expect `submitted`, with the same `submission_id`.
5. `account login --no-browser`. Open the printed URL **with `/v1/traces`
   removed from the path** (see the client defect above), then confirm.
6. With `daemon run --dry-run` running, `daemon withdraw <id>` returns
   `withdrawn: true`. The server status becomes `revoked`.
7. Run the drills. `postgres-rls` and `rollback` should be `ready`. With
   #1095 in the target, `audit-chain` and `db-reconciliation` should be
   `ready` too, with the pre-cutover rows counted as a legacy prefix (row 4
   of What breaks).

## Switch-on order, after the cutover is stable

Each step is a separate change with its own verification. None is part of
the cutover itself.

1. **Mark the pooled tenants.** Start with the two Devfolio shared-code
   tenants. Use `legacy-invite-migration.md`, "Marking a pooled tenant": set
   the tenant context in the same transaction. Review
   `legacy_invite_tenants|review_before_linking` in `legacy-invite-counts.sql`
   and decide each tenant; do not mark by rule. Existing pooled devices keep
   working.
2. **Legacy invite linking.**
   - `GRANT trace_account_invite_runtime TO trace_ingest_runtime;` (V75; also
     needed by `/v1/account/invites/redeem`).
   - Confirm the ingest attestation key is configured.
   - Set `TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED=true` and restart. With the
     key missing, ingest refuses to start.
3. **Earned-trust shadow.**
   - `GRANT trace_account_trust_worker TO trace_ingest_runtime;` (V85). It is
     needed by `/v1/admin/record-account-trust-facts`. Do not grant it to
     `trace_account_admission_runtime`.
   - Set both `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_*` variables and
     schedule `evaluate-account-trust`. Admission never reads it.
4. **Account admission. Do not enable it yet.** Prerequisites:
   - **A client release that sends `source_session`, adopted by the NEAR
     users.** Only clients built from `main` after #1021 send it. Every
     released client up to 0.12.6 omits it. Their new uploads on a
     `near-`/`nearai-` tenant would get `422 source_session_invalid`
     (`admission.rs:649`).
   - `GRANT trace_account_admission_runtime TO trace_ingest_runtime;`, plus
     `INSERT` on `trace_submission_sessions` and `trace_source_sessions` for
     the source-session claim.
   - **Narrow the runtime's `trace_accounts` UPDATE.** The readiness check
     (`admission_ledger.rs:646`) refuses a runtime that can UPDATE
     `trace_accounts.account_id`. The pilot's table-wide grant fails it with
     `account_admission_permissions_or_linkage_not_ready`, as reproduced.
     Ingest writes only `closed_at`, in the merge path.
     ```sql
     REVOKE UPDATE ON trace_accounts FROM trace_ingest_runtime;
     GRANT UPDATE (created_at, closed_at) ON trace_accounts TO trace_ingest_runtime;
     ```
   - `SELECT trace_account_admission_linkage_ready()` returns `t`. That needs
     no unresolved non-pooled link conflicts, and no open non-`near` account
     without an invite device.
   - Only then set the policy variables and `ENABLED=true`. With the grants
     above, legacy `tenant-…` devices were verified to keep uploading. That
     covered individual and pooled devices, and a new pooled enrolment.
5. **Witness certificate v2, last.** Follow "verifiers first" in
   `attested-inference.md`. Only after clients that verify v2 are adopted:
   0.12.x has no v2 support.

## Rollback

- **What works.** Reinstall the `5f239be4` binaries. Released clients kept
  submitting and reading status against the V89 schema (tested). The schema
  stays at V89; the old build ignores the new tables.
- **Rolling forward again needs one repair per affected tenant** (#1100 in
  the roll-forward build). While the old build runs, it appends file audit
  events whose DB mirrors carry no chain hash, or whose events are file-only.
  - The new build then chains from the file head. The DB's latest hashed row
    is older, so every mirrored append is refused as stale. Under
    `REQUIRE_DB_MIRROR_WRITES`, every audited write fails for each tenant that
    was active during the rollback.
  - For each such tenant, run `POST /v1/admin/audit-chain-repair` as that
    tenant's admin:
    1. Dry run (the default). Expect `divergence:
       "file_ahead_through_legacy_rows"`, and segment counts that fit the
       traffic the rollback served. A tenant that was not active reports
       `clean`.
    2. `{"dry_run": false, "accept_legacy_segment": true}`. Without
       `accept_legacy_segment` the repair refuses
       `legacy_segment_not_accepted` and writes nothing. Expect
       `chain_resumed: true`.
    3. Run it once more: `clean`. Then confirm one submission succeeds, and
       run the audit-chain and db-reconciliation drills.
  - Any other refusal label (`db_head_not_in_file`,
    `file_chain_break_after_db_head`, `legacy_row_mismatch`,
    `file_head_not_in_db`) is not a rollback. Stop and treat it as chain drift.
    The procedure and labels are in `audit-trail-forensics.md`, "Rolling
    forward after a binary rollback".
  - Without #1100 in the roll-forward build, treat a rollback as final.
- **Schema.** Schema rollback is a Cloud SQL restore. It loses every write
  since the backup.

## What breaks and the mitigation

| # | What | Who | Mitigation |
|---|---|---|---|
| 1 | New uploads and all status writes 500 (V78 tables not granted) | every client | V78 grants, same session as the migrations |
| 2 | Re-POSTs 500, and witnessed submissions cannot persist evidence (V76) | every client | `GRANT trace_witness_evidence_runtime` |
| 3 | Withdrawal 500; revocation, purge and rescrub hit the token trigger. Pre-existing since V65–V68. | every client using withdraw; operators | token-bundle grant |
| 4 | Audit-chain and db-reconciliation drills `ready: false` after the first new event per tenant, so `smoke-gate.sh` fails | operators | Fixed by #1095. The first hashed row may chain from the file history, pre-cutover rows are counted as a legacy prefix, and reader parity compares them without chain fields. Without #1095 in the target, read the gap as the known transition, one per tenant. |
| 5 | Rollback, then roll-forward, locks tenants out of submissions | everyone, after a rollback | Fixed by #1100. Run the audit-chain repair per affected tenant: dry run, then `accept_legacy_segment: true`. See [Rollback](#rollback). Without #1100 in the roll-forward build, do not roll forward again. |
| 6 | Account admission refuses 0.12.x NEAR uploads with 422 | NEAR-provisioned clients | Keep it off until a `source_session` client is adopted |
| 7 | Account admission refuses to boot while the runtime can UPDATE `trace_accounts.account_id` | operators | Narrow the grant (switch-on step 4) |
| 8 | Witness v2 certificates rejected by 0.12.x | witnessed clients | Keep the witness at `v1` |
| 9 | 0.12.6 `account login` prints a URL that 404s | clients withdrawing | Client fix: strip the path from `ingest_url` |
| 10 | Drill responses: `purpose` became `purpose_hash` (#1044); rollback drill adds `legacy_submit_audit_row_count` | anyone parsing drill JSON | No in-repo script reads `purpose`. `smoke-gate.sh` and `rotate-kek.sh` read only `ready`, `blocking_gaps`, `success` and `required_checks`. Update any out-of-repo parser. |
| 11 | Out-of-repo deploy scripts are pinned to `5f239be4` / `EXPECT_MAX_MIGRATION=74` | operators | Re-pin to the target, with the maximum at 89 |

## Evidence

A local rehearsal on PostgreSQL 14, built to match the pilot:

- **Roles.**
  - A non-superuser `CREATEROLE` migrator owns every table.
  - `public` has the PG15 shape: `CREATE` is revoked from `PUBLIC`.
  - The runtime login belongs to a group that got table-wide SIUD once at
    V62. It gets nothing on V63–V73 except the V64 hand grant, plus
    `trace_public_run_runtime` at V74. It has `CREATE` on the schema, which
    the runner's boot check needs.
- **Ingest configuration**, pilot-shaped:
  - dual-write with required mirror writes and required RLS readiness;
  - the legacy admission switch on;
  - the witness pin and bypass on, with no live witness;
  - EdDSA tokens from a local issuer;
  - a file-allowlist invite path with one individual and one pooled code;
  - a versioned remote object store (file-system provider).
- **The client** is the released `contributor-v0.12.6`, built from its tag.
- **Schema paths.** V1–V62 were applied from `5f239be4`, then V63–V73 from
  `43aa8ad3`. These reproduced V64's eight "no privileges" warnings exactly as
  the pilot logged them. V74 came next, then V75–V89 from the candidate.

| Run | Result |
|---|---|
| Old build `5f239be4` at V74: enrol (individual and pooled), 3 submits, status | pass |
| Old build: withdraw | **fail**, `permission denied for table trace_token_bundles` (pre-existing) |
| Old build: withdraw with the token-bundle grant | pass |
| Old build on the migrated V89 schema: submits, re-POST, new enrolments | pass |
| Candidate, migrations only, no new grants | status and enrolment pass. **Every new submit fails**: `permission denied for table trace_submission_sessions` in `lock_source_session_for_submission`. Withdraw fails the same way. |
| Candidate with the required grants | new submits for 3 tenants, a server-side idempotent re-POST, 2 new enrolments on the second invite use and the pooled code, status, and withdraw (`withdrawn: true`, server `revoked`): all pass. DB audit rows written after the cutover are hashed, and match the file log's `event_id`, `previous_event_hash` and `event_hash`. |
| Candidate plus account admission, runtime role granted, pilot `trace_accounts` UPDATE | boot refused, `account_admission_permissions_or_linkage_not_ready` |
| The same, with `trace_accounts` UPDATE narrowed | boot ok. Legacy individual, pooled, and newly enrolled pooled devices all submit `accepted`. |
| Rollback to `5f239be4` after candidate traffic | submits and status pass |
| Roll forward again (build without #1100) | **every submit fails**; repair refuses `file_head_not_in_db`. #1100 reproduces this in `audit_chain_repair_resumes_the_chain_after_a_binary_rollback` and repairs it. |
| Fresh database, V1–V89 as the non-superuser migrator | pass. The schema is identical to the upgraded database's, apart from the emulated runtime grants. |
| `witness_certificate_cross_implementation` and `witness_admission_chain` tests on the candidate | 5 and 13 passed |

Not covered locally:

- **A live witness.** The witness needs a dstack guest agent, so no witnessed
  upload was run.
- **A `near-` tenant.** NEAR provisioning needs a wallet or NEAR AI ceremony.
  The account-admission refusal for 0.12.x NEAR clients comes from code
  reading (`admission.rs:640-651`) and the fact that no released client
  carries `source_session`.
- **PG15.** The migrator's PG15 behaviour was emulated through the schema
  shape.
