# Pilot cutover to main, 2026-09

This runbook moves the pilot from build `5f239be4` at schema V74 to current
`main`, schema V91. Every server PR the earlier draft listed as landing with
the cutover (#1071, #1073, #1077, #1078, #1084, #1085, #1086, #1087) is now
on `main`, as are #1095, #1098 (V90), #1100, #1112 and #1114. It records a
go/no-go for released contributor clients, what was tested and how, and the
order of operations.

It was written from local rehearsals, not from the pilot. Nothing here was
run against the pilot or any remote system. The latest rehearsal ran `main`
at `5888b8bcd`. The target is `main` at `d8fb248b7`. Between the two, the
server changed in two places only:

- **#1112** writes the file-side tombstone when an account withdraws. See
  item 12 in [What breaks and the mitigation](#what-breaks-and-the-mitigation)
  and step 6 of the [smoke test](#smoke-test-with-a-released-client).
- **#1114** edits `V60__onboarding_retention.sql` so a fresh PostgreSQL 16
  migrator can apply it. The pilot recorded V60 long ago. The runner skips a
  recorded migration by version and name (`recorded_migration_state` in
  `db/postgres.rs`) and never compares file contents, so the edit never runs
  there. Route B must skip it too.

Every other commit in that range is client-side: #1101–#1104 and #1109.

## Decision

**Go, with conditions.** Released clients (0.12.x, latest `contributor-v0.12.6`)
keep enrolling, submitting, reading status and withdrawing after the cutover.
The runtime grants that submit, re-POST and withdraw need are now a migration,
V90 (#1098). An earlier draft of this runbook had the operator run them by
hand; do not. See [Runtime grants: V90](#runtime-grants-v90).

All new feature switches stay off at cutover. Two of them must not be turned
on until later work lands:

- **Account admission** (`TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED`) refuses
  every new upload from a released client on a `near-`/`nearai-` tenant.
  Legacy `tenant-…` invite identities, including pooled Devfolio devices, are
  unaffected (tested).
- **Witness certificate v2** is not understood by 0.12.x clients.

The rehearsals found two operator-side problems. Neither affects clients, and
both are fixed on `main` by #1095 and #1100:

- **Drills after the cutover.** Before #1095, the audit-chain and
  db-reconciliation drills reported `ready: false` after the first
  post-cutover event, for every tenant with pre-cutover audit rows. With
  #1095, the pre-cutover rows are reported as a legacy prefix
  (`db_legacy_prefix_event_count`, `db_audit_legacy_prefix_row_count`), not as
  failures. db-reconciliation still has expected gaps right after the cutover;
  see step 7 of the [smoke test](#smoke-test-with-a-released-client).
- **Binary rollback.** Before #1100, a rollback was one-way. With #1100, rolling
  forward again needs one reviewed repair per tenant that was active during
  the rollback. See [Rollback](#rollback).

## Scope

| | |
|---|---|
| From | build `5f239be4` (deployed 2026-09-22), schema V74. Confirm this on the host first: pre-check 0. |
| To | `main` at `d8fb248b7`, rehearsed at `5888b8bcd`; the delta is described above |
| New migrations | V75–V82 and V84–V91: sixteen files. There is no V83 (the runner tolerates gaps). If the pilot turns out to be at V73, V74 comes first: seventeen files. |
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
| `POST /v1/traces`, new submission | **Breaks without V90** (500). Unchanged with it. | Every submission write takes `FOR UPDATE` on the source-session row (`trace_corpus_pg.rs:1643`, called from `upsert_submission_and_witness_evidence`, `:7150`). V78 grants the runtime nothing; V90 grants it `SELECT` and `UPDATE (withdrawn_at)`. |
| `POST /v1/traces`, re-POST of an existing submission | **Breaks without V90** (500); 200 with it | `reject_conflicting_witness_retry` (`trace-commons-ingest.rs:13352`) reads `trace_witness_certificate_evidence` (`trace_corpus_pg.rs:2069`). V90 makes the runtime a member of V76's `trace_witness_evidence_runtime`. |
| `POST /v1/traces`, re-POST with *different* witness headers | New `409 witness evidence conflict` | Only when the first POST was witnessed and stored by the new build. 0.12.6 caches the certificate with the approved envelope and re-sends the same headers, so it is not expected to hit this. |
| `POST /v1/traces`, idempotent receipt | Additive | The duplicate's audit event is now mirrored to the DB, not written file-only. |
| Source-session binding (#1021) with account admission **off** | No requirement | `source_session` is required only in `reserve_account` (`admission.rs:466`, `:649`), which is reached only when account admission is on **and** the tenant is `near-`/`nearai-` (`:316`). |
| `POST /v1/account/traces/{id}/withdraw` | **Breaks without V90**, including its token-bundle grants, which withdrawal already needs on the current build | `withdraw_trace_source_session` reads `trace_submission_sessions` (`trace_corpus_pg.rs:1748`). The token-bundle step is `pending_token_bundle_deletions` (`trace-commons-ingest.rs:17158`, `db/token_bundles.rs:274`) plus the invoker-rights trigger `trace_revoke_token_bundles` (V65/V66/V68). |
| Withdraw response `credit_retained` | Semantics changed (#990) | It can now be `false`, when unsettled credit is forfeited. 0.12.6 does not read it. |
| `submission-status`, `score-attestation`, community, login links, native auth, v1 NEAR provisioning | Unchanged or additive fields | The v1 provisioning contract is unchanged. The `/v2` routes are new, and the new refusal labels (#1085) do not reach the wire. |
| Issuer `/v1/onboard`, `/v1/trace-upload-claim` | Unchanged for `tenant-…` invites | #1015 refuses invites with a *fixed* tenant in `near-`/`nearai-` (mint, import, and registry lookup). #1086 adds a route. |
| Witness certificate acceptance (#1017/#1018) | Unchanged for v1 | The v1 signing bytes are frozen (`certificate.rs` test). The new build also accepts v2. Verified certificates are now persisted, which needs the V76 role that V90 grants. The pin is now read even when the bypass switch is off. |

A client defect affects the smoke test, not the cutover. `account login` in
0.12.6 prints `<ingest_url>/account/login?...`. The onboarding response's
`ingest_url` carries `/v1/traces`, so the printed URL is
`…/v1/traces/account/login?...`, which returns 404 on the current build and on
the new one. The server path is `/account/login`. #1096 fixed this in the
client (`browser_url` in `crates/trace-commons-contributor/src/account_auth.rs`
now builds the URL on the ingest origin), but it is merged, not released:
the latest release is still `contributor-v0.12.6`. The rehearsal confirmed
both halves. The 0.12.6 URL returned 404 as printed. A client built from
`main` printed `/account/login` on the ingest origin, and that URL worked
unmodified.

## Runtime grants: V90

The pilot's ingest login (`tc_ingest_runtime_login`) holds its table grants
through the hand-made group `trace_ingest_runtime`. `pg_default_acl` is empty,
so a table a migration creates is invisible to ingest until something grants
on it. V90 (`migrations/V90__ingest_runtime_grants.sql`, #1098) is that
grant. It gives `trace_ingest_runtime`:

- `SELECT` on `trace_submission_sessions`, and `SELECT, UPDATE (withdrawn_at)`
  on `trace_source_sessions` (V78);
- membership in `trace_witness_evidence_runtime` (V76);
- `SELECT` on `trace_token_bundles` and `trace_token_attachments`,
  `UPDATE (state, processing_state, processing_summary)` on the bundles, and
  `UPDATE (deleted, prepared)` on the attachments (V65–V68);
- on `trace_accounts`, it revokes the table-wide `UPDATE` and grants back
  `UPDATE (created_at, closed_at)` only. It then fails the migration if the
  runtime can still update `account_id`.

It also grants `INSERT` on both source-session tables to
`trace_account_admission_runtime`, not to the general runtime. Only the
account-admission path claims a source session.

**Do not run the hand grants from the earlier draft of this runbook.** That
block ran `GRANT SELECT, UPDATE ON trace_token_bundles, trace_token_attachments`
table-wide, which widens what V90 deliberately narrowed to column scope. V90
alone is enough. In the rehearsal at `5888b8bcd`, 0.12.6 submit, re-POST and
withdraw all passed with V90 and no hand grants.

If the old block was already run somewhere, undo the table-wide token grants
as the migrator. A table-level `REVOKE UPDATE` also removes the column
grants, so V90's are re-issued after it:

```sql
REVOKE UPDATE ON trace_token_bundles, trace_token_attachments FROM trace_ingest_runtime;
GRANT UPDATE (state, processing_state, processing_summary) ON trace_token_bundles TO trace_ingest_runtime;
GRANT UPDATE (deleted, prepared) ON trace_token_attachments TO trace_ingest_runtime;
```

Post-check, as `tc_ingest_runtime_login`:

```sql
SELECT has_table_privilege('trace_submission_sessions','SELECT'),
       has_column_privilege('trace_source_sessions','withdrawn_at','UPDATE'),
       pg_has_role('trace_witness_evidence_runtime','MEMBER'),
       has_column_privilege('trace_token_bundles','state','UPDATE'),
       has_column_privilege('trace_token_attachments','deleted','UPDATE'),
       NOT has_column_privilege('trace_accounts','account_id','UPDATE');   -- all t
```

Do not check `has_table_privilege('trace_token_bundles','UPDATE')`. V90 grants
column `UPDATE`, not table `UPDATE`, so that check returns `f` on a correct
V90 database. That is what the earlier draft's post-check did, and it gave
`t|t|t|f`.

## Pre-checks

Read the running process, not the config files. See `deployment.md` and the
pilot notes on `/proc/<MainPID>/environ`.

0. **Confirm what is live, on the host.** This runbook assumes `5f239be4` at
   V74. Check it before anything else:
   - `curl -s http://127.0.0.1:3907/health` on the host reports
     `build_commit`. Expect `5f239be4`.
   - As `app`: `SELECT max(version), count(*) FROM _trace_commons_migrations`.
     Expect `74`.
   - At `73`, apply V74 first; see
     [If the pilot is at V73](#if-the-pilot-is-at-v73). Any other build or
     version: stop, and redo the scope and the pins against what is live.
1. `git diff --name-only 5f239be4 d8fb248b7 -- migrations` lists seventeen
   files: V60 plus the sixteen new ones, V75–V82 and V84–V91.
   - **V60 is an edit, not a new migration** (#1114). The pilot has it
     recorded, so neither the runner nor Route B applies it. Confirm
     `SELECT name FROM _trace_commons_migrations WHERE version = 60` returns
     `V60__onboarding_retention`.
   - If the diff lists any file other than these seventeen, `main` has moved:
     stop, and redo the scope and the pins.
   - A plain install over unapplied migrations crash-loops the pilot, because
     the runtime login cannot run DDL.
2. `dig issuer.tracecommons.ai` resolves. Ingest fails closed at boot without
   the keyset.
3. The running ingest has `TRACE_COMMONS_DB_DUAL_WRITE=true`,
   `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true`, `TRACE_COMMONS_ADMISSION_ENABLED`
   (legacy, as today), and none of the new switches.
4. Every name recorded in `_trace_commons_migrations` matches `main`'s
   `MIGRATIONS` list, so the #1028 boot check ("version recorded under another
   name") passes. The Route B scripts inserted the list names.
5. No live invite has a fixed tenant in the reserved namespaces. #1015 stops
   those redeeming. The column is on `onboarding_invite_grants` (V42), not
   `onboarding_invites`, which has no `tenant_mode`. That table has forced
   RLS, and only `trace_invite_registry` has a policy that shows every row.
   Run this as a login in `trace_invite_registry` (the one
   `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL` points at; see
   `pilot-allowlist.md`):
   ```sql
   SELECT count(*) FROM onboarding_invite_grants
    WHERE tenant_mode = 'fixed' AND fixed_tenant_id ~* '^(near|nearai)-'
      AND revoked_at IS NULL;
   ```
   The result should be 0. **A 0 as `app` proves nothing.** RLS hides every
   row from `app`. With one such invite present, the query returned 0 as the
   migrator and 1 as a registry login or a superuser. Also grep the file
   allowlist for a `"tenant_id": "near` entry.
6. **Nothing but `app` granted the runtime `UPDATE` on `trace_accounts`.**
   V90 revokes the pilot's table-wide `UPDATE` as `app`. A `REVOKE` removes
   only the grants its own role made; PostgreSQL warns and goes on. V90 then
   checks the result, and fails with `V90: trace_ingest_runtime can still
   update trace_accounts.account_id` if another grantor's `UPDATE` survives.
   As `app`:
   ```sql
   SELECT a.grantor::regrole::text FROM pg_class c, aclexplode(c.relacl) a
    WHERE c.oid = 'trace_accounts'::regclass
      AND a.grantee = 'trace_ingest_runtime'::regrole AND a.privilege_type = 'UPDATE'
   UNION
   SELECT a.grantor::regrole::text FROM pg_attribute att, aclexplode(att.attacl) a
    WHERE att.attrelid = 'trace_accounts'::regclass AND att.attname = 'account_id'
      AND a.grantee = 'trace_ingest_runtime'::regrole AND a.privilege_type = 'UPDATE';
   ```
   This must return only `app`, or nothing. A grant a superuser made is
   recorded under the owner, `app`, so it passes. For any other grantor
   `<role>`, remove that role's grant as `app`, the table owner, before
   Route B:
   ```sql
   REVOKE GRANT OPTION FOR UPDATE ON trace_accounts FROM <role> CASCADE;
   ```
   `CASCADE` removes the grants `<role>` made with that option. If `<role>`
   holds `UPDATE` for its own use, grant that back without the grant option.
   Then re-run the query. Both the failure and this remediation were
   reproduced locally: V90 failed on the second grantor's grant, and applied
   cleanly after the `REVOKE … CASCADE`.
7. Disk and binary backups are in place, as in `deployment.md`.

## Backups

Take a Cloud SQL on-demand backup, as for V63–V74. **Do not rely on `pg_dump`
as `app`.** Under forced RLS it stops with `query would be affected by
row-level security policy`. With `--enable-row-security` it silently dumps
zero tenant rows. Both were observed in the rehearsal.

## If the pilot is at V73

Only if pre-check 0 found V73. V74 takes `PUBLIC`'s `EXECUTE` away from the
public-run functions, so the ingest runtime loses the public-run pages until
it is granted the new role. Apply V74 as `app` and, in the same transaction
or the same `psql` script, grant it. `deployment.md` has the background,
under V74.

```sql
-- after V74 and its _trace_commons_migrations row
GRANT trace_public_run_runtime TO trace_ingest_runtime;
```

Then continue with Route B from V75. Post-check as `tc_ingest_runtime_login`:
`SELECT pg_has_role('trace_public_run_runtime','MEMBER')` returns `t`.

## Migrations (Route B)

Apply V75–V91 as `app` before touching the binaries. Run each file together
with its `INSERT INTO _trace_commons_migrations (version, name)` in one
transaction, in `MIGRATIONS` order, under the runner's advisory lock. Rehearse
first, with rollback.

- **Go through V91, not V89.** If Route B stops early, the new issuer's boot
  applies the rest there and then: unrehearsed DDL at install time, including
  V90's grant changes. Route B's point is that the issuer's boot finds nothing
  to do.
- **Re-pin the Route B script.** The existing one is pinned to V74. It must be
  generalised, and its pins taken from `d8fb248b7`. Its bundle is V75–V91
  only. V60's file hash changed in #1114, but V60 is already recorded, so the
  script must skip it like any recorded version and must never re-apply it
  because the hash changed.
- **No hand grants.** V90 is the grant step. Run the V90 post-check above
  after V91.
- **The old build keeps serving.** An earlier rehearsal ran `5f239be4` on the
  V89 schema: enrolment, submissions, status and re-POST all passed. In the
  `5888b8bcd` rehearsal, `5f239be4` served submits, a re-POST and status on
  the V91 schema. There is no outage window.
- **Locks.** Each migration took 0.02–0.04 s on the rehearsal database. V89
  touches a live, populated table. It runs `ALTER TABLE trace_credit_ledger
  ADD COLUMN … CHECK`: an ACCESS EXCLUSIVE lock and one validating scan, with
  no rewrite. V77 and V81 add policies to `device_keys`, `trace_accounts`,
  `trace_account_principals`, `trace_near_provisioned_devices`,
  `onboarding_invites` and `onboarding_invite_grants`. V90 changes grants on
  `trace_accounts`, the token tables and the source-session tables. Those are
  brief catalog locks. Everything else, V91 included, creates new, empty
  objects.
- **The issuer migrates at boot as `app`.** Installing the new issuer before
  Route B makes it apply all sixteen migrations there and then. Route B
  through V91 first makes the issuer's boot a no-op.

Post-check as `app`:

- `max(version) = 91`, and `count(*)` is exactly 16 more than before (17 if
  V74 was applied too).
- The roles `trace_witness_evidence_runtime`, `trace_account_admission_runtime`,
  `trace_account_trust_worker` and `trace_legacy_invite_link_guard` exist.
- The [V90 post-check](#runtime-grants-v90) returns all `t`, as
  `tc_ingest_runtime_login`.

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

**If you build binaries yourself** (a rehearsal, or a build off Cloud Build),
never share one `CARGO_TARGET_DIR` across checkouts of different commits.
Cargo can reuse another checkout's artifacts and give you a binary from the
wrong commit, with no error. The rehearsal hit this. Give each checkout its
own target directory, and check every binary's `/health` `build_commit`
against the commit you meant to build before you use it.

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
5. `account login --no-browser`. 0.12.6 prints `…/v1/traces/account/login?…`,
   which returns 404 (see the client defect above). Open it **with
   `/v1/traces` removed from the path**, then confirm. #1096 fixes this, but
   only in clients built from `main`; no release carries it yet.
6. With `daemon run --dry-run` running, `daemon withdraw <id>` returns
   `withdrawn: true`. The server status becomes `revoked`.
7. Run the drills. **Gate on `audit-chain`, `rollback` and `postgres-rls`**:
   each should be `ready`. `audit-chain` counts the pre-cutover rows as a
   legacy prefix (row 4 of What breaks).

   **Do not expect `db-reconciliation` to be `ready` right after the cutover.**
   Its `blocking_gaps` can carry three expected entries. Anything else in it
   is a finding.
   - `audit_reader_sample_parity=failed` and `audit_reader_sample_failures=1`:
     transitional. The reader sample is the tenant's 16 most recent audit
     events (`TRACE_DB_AUDIT_RECONCILIATION_SAMPLE_LIMIT`). While a
     pre-cutover row is among them, the file and DB samples differ. The gap
     clears once 16 newer events exist. In the rehearsal, a tenant with one
     pre-cutover row cleared it after 17 status reads.
   - `accepted_current_derived_without_active_vector_entry=<n>`: accepted
     traces the vector worker has not indexed yet. It shrinks as the worker
     catches up. The pre-cutover baseline had it too.
   - After the step 6 withdrawal, only on a target without #1112:
     `status_mismatches=1`, `derived_status_mismatches=1`, and reader-parity
     failures (`contributor_credit_reader_parity`,
     `reviewer_metadata_reader_parity`, `analytics_reader_parity`,
     `db_reader_parity_failures`). Before #1112, withdrawal updates the
     database but not the file-side record, so the file still says
     `accepted`. See row 12 of What breaks. With #1112, these gaps after a
     withdrawal are a finding.

   Record the full `blocking_gaps` for each tenant, so a later run can be
   compared. `scripts/operator/smoke-gate.sh` requires db-reconciliation to be
   `ready`, so it will not pass until these gaps clear.

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
   - `GRANT trace_account_admission_runtime TO trace_ingest_runtime;`. V90
     already gave that role `INSERT` on `trace_submission_sessions` and
     `trace_source_sessions`, which the source-session claim
     (`claim_trace_source_session`) needs.
   - **Narrow the runtime's `trace_accounts` UPDATE: done by V90.** The
     readiness check (`admission_ledger.rs:646`) refuses a runtime that can
     UPDATE `trace_accounts.account_id`. The pilot's table-wide grant failed
     it with `account_admission_permissions_or_linkage_not_ready`, as
     reproduced. V90 revokes that grant and grants back
     `UPDATE (created_at, closed_at)`; ingest writes only `closed_at`, in the
     merge path. Nothing to run here. The V90 post-check's last column
     confirms it.
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
| 1 | New uploads and all status writes 500 (V78 tables not granted) | every client | V90, applied by Route B |
| 2 | Re-POSTs 500, and witnessed submissions cannot persist evidence (V76) | every client | V90 (membership in `trace_witness_evidence_runtime`) |
| 3 | Withdrawal 500; revocation, purge and rescrub hit the token trigger. Pre-existing since V65–V68. | every client using withdraw; operators | V90's column grants on the token tables. Not the table-wide grant from the earlier draft. |
| 4 | Audit-chain and db-reconciliation drills `ready: false` after the first new event per tenant, so `smoke-gate.sh` fails | operators | Fixed by #1095. The first hashed row may chain from the file history, pre-cutover rows are counted as a legacy prefix, and reader parity compares them without chain fields. db-reconciliation keeps the transitional sample gap until 16 newer events exist; see smoke step 7. |
| 5 | Rollback, then roll-forward, locks tenants out of submissions | everyone, after a rollback | Fixed by #1100. Run the audit-chain repair per affected tenant: dry run, then `accept_legacy_segment: true`. See [Rollback](#rollback). |
| 6 | Account admission refuses 0.12.x NEAR uploads with 422 | NEAR-provisioned clients | Keep it off until a `source_session` client is adopted |
| 7 | Account admission refuses to boot while the runtime can UPDATE `trace_accounts.account_id` | operators | Done by V90. Pre-check 6 makes sure V90 can do it. |
| 8 | Witness v2 certificates rejected by 0.12.x | witnessed clients | Keep the witness at `v1` |
| 9 | 0.12.6 `account login` prints a URL that 404s | clients withdrawing | Strip `/v1/traces` from the printed URL. #1096 fixes the client, but it is merged, not released. |
| 10 | Drill responses: `purpose` became `purpose_hash` (#1044); rollback drill adds `legacy_submit_audit_row_count` | anyone parsing drill JSON | No in-repo script reads `purpose`. `smoke-gate.sh` and `rotate-kek.sh` read only `ready`, `blocking_gaps`, `success` and `required_checks`. Update any out-of-repo parser. |
| 11 | Out-of-repo deploy scripts are pinned to `5f239be4` / `EXPECT_MAX_MIGRATION=74` | operators | Re-pin to the target, with the maximum at 91 |
| 12 | After a withdrawal, the file-side submission record still says `accepted`, so db-reconciliation reports `status_mismatches` and reader-parity failures for that tenant | operators | Present on `5f239be4`, and on `main` up to #1112, which writes the file tombstone and marks the file records revoked. With #1112 in the target, new withdrawals leave no such gap. A withdrawal made before it keeps the gap: read it as that withdrawal, and check that the DB says `revoked`. |

## Evidence

### Rehearsal at `5888b8bcd`

The latest rehearsal ran `main` at `5888b8bcd` on PostgreSQL 14, built to
match the pilot as below, with pre-check 0's other branch: it started at V73.

- **Schema.** V1–V62 were applied from `5f239be4` and V63–V73 from
  `43aa8ad3`, with the pilot's V62 hand grants and V64 hand grant in between.
  The new issuer's boot then applied V74–V91 as the non-superuser migrator.
  That is not Route B, and the `trace_public_run_runtime` grant was not
  applied; the public-run pages were not exercised. V90 applied cleanly, and
  no hand grants were run.
- **Before the cutover**, on `43aa8ad3` at V73: 0.12.6 enrolled on an
  individual and a pooled invite, submitted three traces and read status.
- **After the cutover**, on `5888b8bcd` at V91:

| Run | Result |
|---|---|
| Status of pre-cutover submissions | pass |
| Server-side idempotent re-POST of a pre-cutover submission | pass, same `submission_id` |
| New submits on an individual and a pooled tenant | pass, `accepted` |
| New enrolment on the individual invite's second use, then a submit | pass |
| 0.12.6 `account login`: URL as printed | 404, as expected |
| The same URL with `/v1/traces` removed, then `daemon withdraw` of a pre-cutover submission | pass: `withdrawn: true`, server `revoked` |
| DB audit rows written after the cutover | hashed, and each matches the file log's `event_id`, `previous_event_hash` and `event_hash`. The DB head equals the file head. |
| `audit-chain`, `rollback`, `postgres-rls` drills | `ready` for both tenants |
| `db-reconciliation` drill | not `ready`, with only the smoke step 7 gaps |
| The same tenant after 17 more audit events | the sample-parity gap cleared; only the vector gap remained |
| A client built from `main` (#1096): `account login` | printed `/account/login` on the ingest origin; that URL worked unmodified |
| Rollback to `5f239be4` on V91, then roll forward with the #1100 repair | as in [Rollback](#rollback) |

### Earlier rehearsal, candidate at V89

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
  the pilot logged them. V74 came next, then V75–V89 from the candidate, by
  Route B.

The "required grants" in this table are the hand grants that V90 has since
replaced.

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
- **Route B through V91.** The `5888b8bcd` rehearsal let the issuer's boot
  apply V74–V91. Route B's mechanics were rehearsed to V89 only.
