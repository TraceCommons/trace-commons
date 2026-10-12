# Pilot switch-on: account admission and legacy invite linking, 2026-10

This runbook turns on, in order, on the pilot (`tc-pilot-host`):

1. the build that carries #1373 (attested uploads skip the allowance) and
   migration V125;
2. the pooled-tenant markers for the two Devfolio tenants;
3. legacy invite linking (`TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED`);
4. account contribution admission (`TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED`)
   with a weekly allowance.

It is a draft for the owner to execute, step by step, with a stop condition
and a rollback on every step. Nothing in this file has been run against the
pilot. Every command, path, variable and label below was checked against the
repo at `origin/main` `6f6389129`; the source is named where it is not obvious.

Background to read first, not repeated here: [account trust](./account-trust.md),
[legacy invite migration](./legacy-invite-migration.md),
[env reference](./env-reference.md) ("Account contribution admission",
"Legacy invite linking") and the earlier
[2026-09 cutover](./pilot-cutover-2026-09.md), whose "Switch-on order" this
runbook executes and whose step 4 prerequisites it carries forward.

## What changes, and for whom

| Who | After the switch-on |
|---|---|
| Legacy `tenant-…` invite devices, including `tenant-zaki-pilot` and the pooled Devfolio devices | Unchanged. A `tenant-…` request never reaches the account ledger, linked or not (`legacy-invite-migration.md`, rule 1). |
| `near-`/`nearai-` upload carrying verified admission evidence **and** `source_session` | Admitted with no debit. Recorded `authority_kind = 'attested'` (V125). |
| `near-`/`nearai-` upload carrying verified evidence and **no** `source_session` (every 0.12.x client; a witness build that strips the field) | Takes the V59 evidence ledger, exactly as today, under the legacy `TRACE_COMMONS_ADMISSION_*` limits. No account-ledger row. (`account-trust.md`, "Attested uploads".) |
| `near-`/`nearai-` upload with **no evidence** | Needs `source_session` and draws the weekly allowance: one unit per reserved attempt. Without `source_session` it is refused `422 source_session_invalid`. Over the allowance: `429 account_limit_reached`. |
| An account holding a valid invite grant (including a legacy tenant once linked) | Skips the allowance. Still faces the hourly quota, request-size and capacity limits. |

Client behavior on a refusal, from the reviewed plan (tag `contributor-v0.13.1`):
`422 source_session_invalid` is not an admission label, so the client shows
`ingest-unreachable` and marks the entry `Failed`, which is never retried;
`429 account_limit_reached` shows `admission-limit-reached` and also ends
`Failed`. Folders stay armed in both cases. This is why the evidence-path
carve-out above matters: a 0.12.x client that sends evidence is unaffected.

Weekly period: `{"mode":"fixed","seconds":604800}`. Buckets are aligned to the
Unix epoch (`floor(epoch / seconds)` in `account_policy_period`,
`admission_ledger.rs`), so a week runs Thursday 00:00 UTC to Thursday 00:00
UTC. Every account's allowance resets at that boundary, not seven days after
its first upload.

## Decisions fixed by the owner (2026-10-11)

| Setting | Value |
|---|---|
| Account admission | on |
| `period` | `{"mode":"fixed","seconds":604800}` |
| `bounded_allowance` | `<BASE>`, not yet fixed. Recommended: 100. |
| `processing_cost_bound` | 1 |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS` | 900 |
| `growth_rule` | `"none"` for this switch-on |
| Evidence settings | copied 1:1 from the legacy `TRACE_COMMONS_ADMISSION_*` set into `TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_*` (all four or none) |
| Legacy `TRACE_COMMONS_ADMISSION_*` set | kept; required at startup since #1373 |
| Invite linking | on |
| Policy version | `pilot-weekly-v1` (any `[A-Za-z0-9._-]`, up to 64; the JSON `version` and the `_POLICY_VERSION` variable must be equal) |

**Freeze rule.** Each account's budget row stores the allowance, the cost bound
and the version. A configured policy that differs from a stored row refuses
every unattested reservation as `403 admission_refused`
(`account-trust.md`, "Attested uploads"). Change the numbers only together with
a version bump. Fixed-period spend is summed across versions that share the
same duration and bucket, so a bump inside a week does not refresh the
allowance.

`<BASE>` is a policy value that is effectively permanent once accounts have
rows. Pick it before step 5. Accounts start with nothing spent: V59 spend does
not carry over. There is no account-path equivalent of `GLOBAL_COST_LIMIT`, so
total admissions across bounded accounts have no cap.

## Conventions

Run everything below from the repo root on the laptop, at the commit being
shipped. Commands reach the host with `gcloud compute ssh`; blocks marked
"host" are sent to a remote `bash -s` over stdin. Nothing prints a secret:
database URLs are loaded into shell variables on the host and never echoed,
and signer keys are shown only as a count and an 8-character prefix. (A URL is
visible in `ps` on the host for the life of a `psql` call; the host is
single-purpose.)

Before anything: confirm the account and project (the active account must be
`zaki@iqlusion.io` and must hold compute and SQL permissions; if it is expired,
stop and re-authenticate).

```bash
gcloud auth list && gcloud config get-value project
```

Set up the helpers once per shell:

```bash
export TC_PROJECT=tracecommons-pilot-2026 TC_ZONE=us-central1-a TC_HOST=tc-pilot-host

# The prelude every host block starts with. It reads the running process, the
# only ground truth for config on this host (pilot-config-only-in-process): the
# env files, drop-ins and journal all give confidently wrong answers.
TC_LIB=$(cat <<'LIB'
set -euo pipefail
pid()   { systemctl show -p MainPID --value trace-commons-ingest; }
penv()  { local p; p=$(pid); [ -n "$p" ] && [ "$p" != 0 ] || { echo "STOP: ingest is not running" >&2; return 1; }; sudo cat "/proc/$p/environ" | tr '\0' '\n'; }
names() { penv | cut -d= -f1 | grep -E '^(TRACE_COMMONS_(ACCOUNT_ADMISSION_|LEGACY_INVITE_LINK_|INGEST_ATTESTATION_|TENANT_TOKENS|INGEST_TOKEN|ADMISSION_)|DATABASE_URL$)' | sort || true; }
ing_url() { penv | sed -n 's/^DATABASE_URL=//p'; }
app_url() { sudo sed -n 's/^DATABASE_URL=//p' /etc/tracecommons/ingest.env | tr -d "\"'"; }
# Line counts of ingest.log and ingest.err, each its own offset. The file is an
# argument to wc: a redirect would be opened by the unprivileged ssh user.
logmark() {
  local f n
  for f in ingest.log ingest.err; do
    n=$(sudo wc -l "/var/log/tracecommons/$f" 2>/dev/null | awk '{print $1}')
    printf '%s ' "${n:-0}"
  done
}
LABELS='account_admission_(permissions_or_linkage_not_ready|policy_invalid|policy_missing|policy_version_missing|lease_invalid|evidence_policy_invalid|activation_invalid|readiness_unavailable|database_unavailable|requires_durable_database|requires_legacy_evidence_policy)|admission_runtime_permissions_not_ready|admission_provider_policy_missing_or_invalid|account_identity_unlinked|legacy_invite_link_(requires_attestation_key|enabled_invalid)|panicked|ERROR'
# scan FILE TAIL_ARG ("+N" = from line N, "-40" = last 40): matching lines, ANSI stripped and
# truncated, or "none".
scan() {
  local out
  out=$(sudo tail -n "$2" "/var/log/tracecommons/$1" 2>/dev/null \
    | sed 's/\x1b\[[0-9;]*m//g' | grep -E "$LABELS" | cut -c1-240 || true)
  echo "--- $1"
  if [ -n "$out" ]; then printf '%s\n' "$out"; else echo none; fi
}
# $1 = the "log err" pair that logmark printed before the restart.
refusals_since() {
  local ml me; read -r ml me <<<"${1:-0 0}"
  scan ingest.log "+$(( ml + 1 ))"; scan ingest.err "+$(( me + 1 ))"
}
refusals_tail() { scan ingest.log "-40"; scan ingest.err "-40"; }
LIB
)

# tcrun: send the prelude, the optional TC_VARS assignments, and the heredoc
# that follows to the host.
tcrun() { { printf '%s\n' "$TC_LIB"; printf '%s\n' "${TC_VARS:-:}"; cat; } | gcloud compute ssh "$TC_HOST" --project "$TC_PROJECT" --zone "$TC_ZONE" --command 'bash -s'; }
```

Sources: unit name `trace-commons-ingest`, binary
`/opt/tracecommons/bin/trace-commons-ingest`, logs
`/var/log/tracecommons/ingest.{log,err}` (they are not in the journal, and
carry ANSI colour, hence the `sed`): `deploy/pilot-gcp/systemd/trace-commons-ingest.service`
and the pilot notes in `deployment.md`. `/etc/tracecommons/ingest.env` holds the
`app` (migrator) URL: `deploy/pilot-gcp/ingest.env.template` line 28. The
ingest login's URL is read from the process, because a later `EnvironmentFile`
overrides `ingest.env`'s `DATABASE_URL`.

## Prerequisites

- [ ] The owner has fixed `<BASE>`.
- [ ] The commit to ship (`<SHA>`) contains #1373: `git merge-base --is-ancestor bb59d3039 <SHA>`.
- [ ] `<SHA>` carries no migration other than V125 beyond the running build. If
      the growth branch (V126) has landed in `<SHA>`, do not ship it with this
      runbook: pick a `<SHA>` before it, or rewrite step 1 to cover V126 and
      leave the scheduler and growth settings off.
- [ ] `dig +short issuer.tracecommons.ai` resolves. Ingest fails closed at boot
      without the issuer keyset, and every step below restarts ingest.
- [ ] Every restart is a short ingest outage. Pick a window; do not stack steps
      3 to 5 into one restart (a failure then points at three causes).
- [ ] 0.13.x is the client generation to test with (`contributor-v0.13.1` is the
      latest tag). 0.12.x devices on `near-`/`nearai-` tenants keep working only
      while they send admission evidence; how many there are is unknown until
      measured (step 2).

## Step 0: record the current state

Read-only. Save the output; later steps compare against it.

```bash
tcrun <<'EOF'
echo "== health";    curl -s http://127.0.0.1:3907/health | head -c 400; echo
echo "== unit";      systemctl show trace-commons-ingest -p FragmentPath -p DropInPaths -p EnvironmentFiles -p NRestarts -p ActiveEnterTimestamp
echo "== env names (process)"; names
echo "== legacy admission values (non-secret)"
penv | grep -E '^TRACE_COMMONS_ADMISSION_(ENABLED|WINDOW_ATTEMPTS|ACCOUNT_COST_LIMIT|GLOBAL_COST_LIMIT|PROCESSING_COST_BOUND|LEASE_SECONDS|CHALLENGE_TTL_SECONDS|ACCEPTED_MODELS|MIN_REQUEST_BYTES)=' | sort
echo "== signers (count, 8-char prefixes)"
for k in PROVIDER_SIGNERS GATEWAY_SIGNERS; do
  v=$(penv | sed -n "s/^TRACE_COMMONS_ADMISSION_$k=//p")
  echo "$k: $(printf '%s' "$v" | tr ',' '\n' | grep -c .) keys: $(printf '%s' "$v" | tr ',' '\n' | cut -c1-8 | paste -sd' ' -)"
done
echo "== migrations (as app)"
APP=$(app_url); psql "$APP" -XAt -c "select current_user"
psql "$APP" -XAt -c "select max(version), count(*) from _trace_commons_migrations"
psql "$APP" -XAt -c "select version, name from _trace_commons_migrations where version >= 123 order by version"
EOF
```

Expected (facts measured 2026-10-11, build `6a5db334`):

- `/health` reports `build_commit` `6a5db334`.
- `names` lists `TRACE_COMMONS_ADMISSION_*` (including `PROVIDER_SIGNERS`,
  `GATEWAY_SIGNERS`, `ACCEPTED_MODELS`, `MIN_REQUEST_BYTES`,
  `PROCESSING_COST_BOUND`=1, `ACCOUNT_COST_LIMIT`=500, `LEASE_SECONDS`=900) and
  all three `TRACE_COMMONS_INGEST_ATTESTATION_{SIGNING_KEY_PEM,PUBLIC_KEY_PEM,SIGNING_KID}`.
  No `TRACE_COMMONS_TENANT_TOKENS`, no `TRACE_COMMONS_INGEST_TOKEN`
  (a contributor static token must resolve to a live account at startup, or
  ingest refuses with `account_identity_unlinked`:
  `trace-commons-ingest.rs`, account-admission startup block). No
  `TRACE_COMMONS_ACCOUNT_ADMISSION_*`, no `TRACE_COMMONS_LEGACY_INVITE_LINK_*`.
- Provider signers: 5 keys; gateway signers: 1.
- `current_user` is `app`. The migration list ends at V124
  (`pipeline_default_routing`); `6a5db334..6f6389129` adds only V125
  (`git diff --name-only 6a5db334 6f6389129 -- migrations`).
- `DropInPaths` and `EnvironmentFiles` are recorded for step 4 and 5. If a
  drop-in already sets any name that steps 4 or 5 will set, stop: a later
  `EnvironmentFile=` also beats `Environment=`, so resolve the conflict first.

Stop if: `build_commit` is not `6a5db334` (the scope below is wrong; redo the
migration diff against what is live), the max migration is not 124, or any
absent name above is present.

## Step 1: deploy the build with #1373 and V125

Source: `deployment.md`, "Redeploying the binary" (apply migrations as the
migrator **before** installing the binary; the runtime role cannot run DDL, so a
plain install over an unapplied migration crash-loops the service, as on
2026-09-21) and `deploy/pilot-gcp/pull-and-install.sh`.

### 1a. Check the diff

```bash
SHA=<SHA>                       # the commit to ship; must contain bb59d3039
git merge-base --is-ancestor bb59d3039 "$SHA" && echo "contains #1373"
git diff --name-only 6a5db334 "$SHA" -- migrations
```

Expected: `contains #1373`, and exactly one line,
`migrations/V125__account_admission_attested_kind.sql`. Anything else: stop.

V125 only widens a CHECK (`authority_kind IN ('account','attested')`); it adds no
table and no grant, and is idempotent (its own header). The running old binary
ignores it (`deployment.md`: an older binary ignores migration versions it does
not know).

### 1b. Back up

```bash
gcloud sql instances list --project "$TC_PROJECT" --format='value(name)'
SQL_INSTANCE=<name printed above>
gcloud sql backups create --instance "$SQL_INSTANCE" --project "$TC_PROJECT"
```

Do not use `pg_dump` as `app`: under forced RLS it stops, or silently dumps
zero tenant rows (`pilot-cutover-2026-09.md`, "Backups"). Record the backup id.

### 1c. Apply V125 as `app`

```bash
git show "$SHA":migrations/V125__account_admission_attested_kind.sql > /tmp/V125.sql
shasum -a 256 /tmp/V125.sql
gcloud compute scp /tmp/V125.sql "$TC_HOST":/tmp/V125.sql --project "$TC_PROJECT" --zone "$TC_ZONE"
tcrun <<'EOF'
sha256sum /tmp/V125.sql        # must equal the shasum printed on the laptop
APP=$(app_url)
test "$(psql "$APP" -XAt -c "select count(*) from _trace_commons_migrations where version = 125")" = 0 \
  || { echo "STOP: V125 already recorded"; exit 1; }
psql "$APP" -X --single-transaction -v ON_ERROR_STOP=1 \
  -f /tmp/V125.sql \
  -c "INSERT INTO _trace_commons_migrations (version, name) VALUES (125, 'account_admission_attested_kind')"
psql "$APP" -XAt -c "select version, name from _trace_commons_migrations where version = 125"
psql "$APP" -XAt -c "select pg_get_constraintdef(oid) from pg_constraint where conname = 'trace_account_admission_submissions_authority_kind_check'"
rm -f /tmp/V125.sql
EOF
rm -f /tmp/V125.sql
```

Applying by hand skips the runner's advisory lock; that is safe here because the
issuer is not restarted and the ingest login cannot run DDL, so nothing else
migrates meanwhile.

The name `account_admission_attested_kind` is the one in the `MIGRATIONS` table
(`db/postgres.rs`); the runner records version and name only and refuses a
version recorded under another name, so a typo here fails the next boot.

Expected: `125|account_admission_attested_kind`, and a constraint definition
reading `CHECK ((authority_kind = ANY (ARRAY['account'::text, 'attested'::text])))`.
Stop if the transaction errors; it rolls back whole.

### 1d. Build and publish

From a clean checkout at `$SHA` (the build uploads the working tree;
`git status` must be clean):

```bash
git status --short | head -5     # must print nothing
test "$(git rev-parse HEAD)" = "$(git rev-parse "$SHA")" && echo "HEAD is $SHA"
REV=$(python3 scripts/operator/pipeline.py revision) && \
gcloud builds submit --config cloudbuild.yaml \
  --project "$TC_PROJECT" \
  --substitutions _TAG=$(git rev-parse --short HEAD),_CODE_REVISION_HASH="$REV"
gcloud builds list --project "$TC_PROJECT" --limit 3
```

Wait for `SUCCESS` on the build for this tag before step 1e. `_TAG` becomes
`build_commit`. The build publishes both binaries and moves both `latest.txt`
pointers, so never run a bare `pull-and-install.sh` afterwards.

### 1e. Install ingest only

The issuer is unchanged by #1373 (`deployment.md`: deploy ingest alone unless
the issuer also changed). Copy the reviewed script from the laptop and run it
as a file; do not feed it to `bash -s`, because `gcloud` inside it could read
the rest of the script from stdin:

```bash
TAG=$(git rev-parse --short HEAD)
gcloud compute scp deploy/pilot-gcp/pull-and-install.sh "$TC_HOST":/tmp/pull-and-install.sh --project "$TC_PROJECT" --zone "$TC_ZONE"
gcloud compute ssh "$TC_HOST" --project "$TC_PROJECT" --zone "$TC_ZONE" \
  --command "TC_EXPECT_TAG=$TAG bash /tmp/pull-and-install.sh ingest $TAG; rm -f /tmp/pull-and-install.sh"
```

Expected output: `latest.txt tag: <TAG>`, `sha256 verified`, `installed
/opt/tracecommons/bin/trace-commons-ingest`, `active`, and a printed rollback
line `sudo install -m0755 …bak-<stamp> … && sudo systemctl restart
trace-commons-ingest`. **Copy that rollback line.** The script refuses
(`REFUSING: expected tag`) when `latest.txt` is not your tag; that means the
build has not published, not that anything is wrong.

### 1f. Post-check

```bash
tcrun <<'EOF'
curl -s http://127.0.0.1:3907/health | head -c 300; echo
systemctl is-active trace-commons-ingest || true
systemctl show trace-commons-ingest -p NRestarts -p ActiveEnterTimestamp
sudo grep "privacy filter backend resolved" /var/log/tracecommons/ingest.log | tail -1 | sed 's/\x1b\[[0-9;]*m//g' | cut -c1-200
echo "== names (must equal step 0)"; names
echo "== last 40 lines of each log, filtered"; refusals_tail
EOF
```

Expected: `build_commit` equals `$TAG`; `active`; `NRestarts` 0; the
privacy-filter line is present; names identical to step 0.

Stop if: not `active`, `NRestarts` above 0, or `/health` shows another commit.

### Rollback, step 1

Run the printed rollback line. V125 stays applied; the old binary ignores it
and writes only `authority_kind = 'account'`, which the widened CHECK still
admits. The schema rollback (a Cloud SQL restore) is not needed and loses every
write since the backup.

## Step 2: read-only pre-flight

Nothing here writes. It answers whether the next startups will pass and what the
switch-on touches.

### 2a. Roles, grants and linkage as the ingest login

```bash
tcrun <<'EOF'
ING=$(ing_url)
export PGOPTIONS='-c default_transaction_read_only=on'
psql "$ING" -XAt -c "select current_user"        # expect tc_ingest_runtime_login
psql "$ING" -XAt -c "select trace_account_admission_linkage_ready(),
       pg_has_role('trace_account_admission_runtime','MEMBER'),
       pg_has_role('trace_account_invite_runtime','MEMBER')"
echo "== V90 post-check (all t)"
psql "$ING" -XAt -c "SELECT has_table_privilege('trace_submission_sessions','SELECT'),
       has_column_privilege('trace_source_sessions','withdrawn_at','UPDATE'),
       pg_has_role('trace_witness_evidence_runtime','MEMBER'),
       has_column_privilege('trace_token_bundles','state','UPDATE'),
       has_column_privilege('trace_token_attachments','deleted','UPDATE'),
       NOT has_column_privilege('trace_accounts','account_id','UPDATE')"
echo "== legacy ledger execute rights (required by the legacy policy check)"
psql "$ING" -XAt -c "SELECT has_function_privilege('public.trace_reserve_admission(text,text,uuid,text,text,text,bigint,bigint,bigint,bigint,uuid,bigint)','EXECUTE'),
       has_function_privilege('public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE')"
EOF
```

Expected: `tc_ingest_runtime_login`; `t|t|t`; six `t`; `t|t`. Sources: V90 post-check
in `pilot-cutover-2026-09.md`; the two function signatures from
`check_admission_runtime` in `admission_ledger.rs`.

A `f` in the membership row is fixed by the group grant, as `app` (not the
login), then re-run 2a (`pilot-cutover-2026-09.md`, "Switch-on order" steps 2
and 4):

```sql
GRANT trace_account_admission_runtime TO trace_ingest_runtime;
GRANT trace_account_invite_runtime TO trace_ingest_runtime;
```

`trace_account_admission_linkage_ready()` false means the durable inventory
holds an ambiguous identity (a `near-`/`nearai-` device without live linkage; a
legacy-namespace device not invite-onboarded; an open legacy account with no
invite behind it; an unresolved non-pooled link conflict). Stop and read
`legacy-invite-migration.md`, "Readiness under coexistence".

### 2b. The exact readiness predicate startup will evaluate

`account_admission_runtime_ready` is one SQL statement; extract it from the
commit being shipped and run it as the ingest login. A superuser gets `f` by
design, so run it only as that login. From the repo root:

```bash
git show "$SHA":crates/trace-commons-server/src/admission_ledger.rs | awk \
  '/pub async fn account_admission_runtime_ready/{f=1} f&&/query_one\(r#"/{sub(/.*query_one\(r#"/,"");p=1}
   p{if($0~/"#, &\[\]\)/){sub(/"#, &\[\]\).*/,"");print;exit}print}' > /tmp/ready.sql
echo ';' >> /tmp/ready.sql
wc -l /tmp/ready.sql             # about 51 lines
gcloud compute scp /tmp/ready.sql "$TC_HOST":/tmp/ready.sql --project "$TC_PROJECT" --zone "$TC_ZONE"
tcrun <<'EOF'
ING=$(ing_url)
PGOPTIONS='-c default_transaction_read_only=on' psql "$ING" -XAt -v ON_ERROR_STOP=1 -f /tmp/ready.sql
rm -f /tmp/ready.sql
EOF
rm -f /tmp/ready.sql
```

Expected: `t`. (The extraction was checked to parse and run against a scratch
PostgreSQL, where a superuser returned `f`.) `f` means one privilege is
missing; the startup refusal would be
`account_admission_permissions_or_linkage_not_ready`. Stop.

### 2c. Counts

The counts script needs a superuser or `BYPASSRLS` role; the pilot's `app` has
`rolbypassrls = f` (pilot notes), in which case the script refuses with
`LegacyInviteCountsRoleSubjectToRls`.

```bash
gcloud compute scp scripts/operator/legacy-invite-counts.sql "$TC_HOST":/tmp/legacy-invite-counts.sql --project "$TC_PROJECT" --zone "$TC_ZONE"
tcrun <<'EOF'
APP=$(app_url)
psql "$APP" -XAt -c "select rolname from pg_roles where rolsuper or rolbypassrls order by 1"
EOF
```

If a usable role exists, run the script against it (`legacy-invite-counts.md`):

```bash
tcrun <<'EOF'
BYPASS_URL=<load without printing; the role from the list above>
PGOPTIONS='-c default_transaction_read_only=on' psql "$BYPASS_URL" -X -v ON_ERROR_STOP=1 -f /tmp/legacy-invite-counts.sql
EOF
```

Record `accounts` (`wallet`/`near_ai` open and anchored), `legacy_invite_tenants|review_before_linking`,
`ambiguous` and `readiness_blockers|total`. If no role qualifies, the counts are
**unknown**: the boolean in 2a/2b is the readiness answer, and the number of
`near-`/`nearai-` devices on 0.12.x is unmeasured. The earlier figure (5
accounts at V115) is a memory, not a measurement. Clean up:
`rm -f /tmp/legacy-invite-counts.sql` on the host.

Decide each tenant counted under `review_before_linking` before step 4; do not
mark by rule (`legacy-invite-migration.md`).

### 2d. Pooled markers (before)

```bash
tcrun <<'EOF'
APP=$(app_url)
for t in tenant-devfolio-hackathon tenant-devfolio-origin-hackathon; do
  psql "$APP" -XAt -v ON_ERROR_STOP=1 <<SQL
BEGIN;
SELECT set_config('trace_commons.trace_tenant_id', '$t', true);
SELECT '$t', count(*) FROM trace_legacy_invite_pooled_tenants;
COMMIT;
SQL
done
EOF
```

Expected before step 3: `0` for both. (Forced RLS: a read without the tenant
setting returns nothing and proves nothing.)

Stop if any of 2a/2b is not `t`. Do not proceed to step 3 on an unexplained
count.

## Step 3: mark the pooled tenants

Source: `legacy-invite-migration.md`, "Marking a pooled tenant". Run as `app`,
one transaction per tenant, with the tenant context set in the same
transaction. The tenant row must already exist (it is created on first
redemption); a foreign-key error means it does not, so stop.

```bash
tcrun <<'EOF'
APP=$(app_url)
for t in tenant-devfolio-hackathon tenant-devfolio-origin-hackathon; do
  psql "$APP" -XAt -v ON_ERROR_STOP=1 <<SQL
BEGIN;
SELECT set_config('trace_commons.trace_tenant_id', '$t', true);
INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
VALUES ('$t', 'shared event invite') ON CONFLICT (tenant_id) DO NOTHING;
SELECT '$t', count(*) FROM trace_legacy_invite_pooled_tenants;
COMMIT;
SQL
done
EOF
```

Expected: `1` for each tenant. The `reason_label` is operator text (1 to 128
characters, free of names). Marking changes nothing for the tenants' devices:
they keep onboarding and uploading (`legacy_invite_link_pg` asserts it for a
Devfolio-shaped invite).

Re-run 2d to confirm, and re-run 2a (`linkage_ready` must still be `t`).

### Rollback, step 3

Markers are inert until linking is on, and are meant to stay
(`legacy-invite-migration.md`: "Keep the pooled marker"). To remove one anyway,
as `app` with the same tenant context in the transaction:
`DELETE FROM trace_legacy_invite_pooled_tenants WHERE tenant_id = '<t>';`.
Do not remove a marker once linking is on: an unmarked shared tenant can be
claimed by the first member to link it.

## Step 4: turn on legacy invite linking

Own drop-in and own restart, so a failure points at one cause. Prerequisites,
all from step 2: `trace_account_invite_runtime` membership `t`; the three
`TRACE_COMMONS_INGEST_ATTESTATION_*` names present (linking countersigns with
that key; without it ingest refuses with
`legacy_invite_link_requires_attestation_key`); both pooled tenants marked.

```bash
tcrun <<'EOF'
MARK=$(logmark)
sudo install -d -m 755 /etc/systemd/system/trace-commons-ingest.service.d
sudo tee /etc/systemd/system/trace-commons-ingest.service.d/legacy-invite-link.conf >/dev/null <<'UNIT'
[Service]
Environment=TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED=true
UNIT
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
sleep 10
systemctl is-active trace-commons-ingest || true
systemctl show trace-commons-ingest -p NRestarts -p ActiveEnterTimestamp
curl -s http://127.0.0.1:3907/health | head -c 300; echo
penv | grep -E '^TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED='
refusals_since "$MARK"
EOF
```

Expected: `active`; `NRestarts` 0; the build commit unchanged; the process shows
`TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED=true`; both logs print `none`.

Stop (and roll back) if: the service is not `active` or restarts; the variable
is absent from `/proc/<pid>/environ` (the drop-in did not take; fix before
going on); a refusal label appears.

Effect: `POST /v1/account/invites/legacy-link[/challenge]` open. With the switch
off both answer 503 `legacy_invite_link_not_enabled`. Nothing links on its own;
a person has to ask, signed into a NEAR account, with the legacy device key.

### Rollback, step 4

```bash
tcrun <<'EOF'
sudo rm /etc/systemd/system/trace-commons-ingest.service.d/legacy-invite-link.conf
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
sleep 10
systemctl is-active trace-commons-ingest || true
names
EOF
```

Persists: any link already made (`trace_legacy_invite_links`), its invite grant,
and the account's `invited` authority. Turning linking off only returns 503 on
the routes. Revoking a link is a separate operator action
(`legacy-invite-migration.md`, "Resolving a conflict").

## Step 5: turn on account admission

Prerequisites: step 2 all `t`; step 4 stable; `<BASE>` fixed; the legacy
`TRACE_COMMONS_ADMISSION_*` set present (its absence refuses startup with
`account_admission_requires_legacy_evidence_policy`).

The drop-in is generated on the host from the running process, so the four
evidence settings are copied byte for byte and never pass through the laptop.
The script refuses a value with a character systemd would reinterpret, and
refuses to run if any evidence setting is missing or appears twice.

Write the file first (it starts nothing), review it, then restart:

```bash
TC_VARS='BASE=<BASE>; VERSION=pilot-weekly-v1' tcrun <<'EOF'
case "$BASE" in ''|*[!0-9]*) echo "STOP: BASE must be a positive integer"; exit 1;; esac
[ "$BASE" -ge 1 ] || { echo "STOP: BASE must be >= 1"; exit 1; }
printf '%s' "$VERSION" | grep -Eq '^[A-Za-z0-9._-]{1,64}$' || { echo "STOP: bad VERSION"; exit 1; }
cp_val() {
  local v; v=$(penv | sed -n "s/^TRACE_COMMONS_ADMISSION_$1=//p")
  [ "$(printf '%s\n' "$v" | grep -c .)" = 1 ] || { echo "STOP: $1 missing or duplicated" >&2; exit 1; }
  printf '%s' "$v" | grep -Eq '^[A-Za-z0-9,._/:+=-]+$' || { echo "STOP: $1 has a character systemd would reinterpret" >&2; exit 1; }
  printf '%s' "$v"
}
PROVIDERS=$(cp_val PROVIDER_SIGNERS)
GATEWAYS=$(cp_val GATEWAY_SIGNERS)
MODELS=$(cp_val ACCEPTED_MODELS)
MINBYTES=$(cp_val MIN_REQUEST_BYTES)
POLICY='{"version":"'"$VERSION"'","processing_cost_bound":1,"bounded_allowance":'"$BASE"',"period":{"mode":"fixed","seconds":604800},"growth_rule":"none"}'
sudo install -d -m 755 /etc/systemd/system/trace-commons-ingest.service.d
sudo tee /etc/systemd/system/trace-commons-ingest.service.d/account-admission.conf >/dev/null <<UNIT
[Service]
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED=true
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION=$VERSION
Environment='TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON=$POLICY'
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS=900
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_PROVIDER_SIGNERS=$PROVIDERS
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_GATEWAY_SIGNERS=$GATEWAYS
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_ACCEPTED_MODELS=$MODELS
Environment=TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_MIN_REQUEST_BYTES=$MINBYTES
UNIT
echo "== drop-in as written (signer keys truncated)"
sudo sed -E 's/(SIGNERS=)(.{8})[^ ]*/\1\2.../' /etc/systemd/system/trace-commons-ingest.service.d/account-admission.conf
EOF
```

Review the printed drop-in: the JSON carries `<BASE>`, `processing_cost_bound`
1, `period` fixed 604800 and `growth_rule` none. If anything is wrong, delete
the file and redo; nothing has restarted. Then restart and verify in one block:

```bash
tcrun <<'EOF'
MARK=$(logmark)
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
sleep 12
systemctl is-active trace-commons-ingest || true
systemctl show trace-commons-ingest -p NRestarts -p ActiveEnterTimestamp
curl -s http://127.0.0.1:3907/health | head -c 300; echo
echo "== names"; names
echo "== effective values (non-secret)"
penv | grep -E '^TRACE_COMMONS_ACCOUNT_ADMISSION_(ENABLED|POLICY_VERSION|POLICY_JSON|LEASE_SECONDS|EVIDENCE_MIN_REQUEST_BYTES|EVIDENCE_ACCEPTED_MODELS)='
echo "== evidence equals legacy"
for k in PROVIDER_SIGNERS GATEWAY_SIGNERS ACCEPTED_MODELS MIN_REQUEST_BYTES; do
  a=$(penv | sed -n "s/^TRACE_COMMONS_ADMISSION_$k=//p"); b=$(penv | sed -n "s/^TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_$k=//p")
  if [ "$a" = "$b" ]; then echo "$k: identical"; else echo "$k: DIFFERS"; fi
done
refusals_since "$MARK"
EOF
```

Expected: `active`; `NRestarts` 0; `/health` unchanged; the account names now
listed alongside the legacy set and the process shows
`ENABLED=true`, the version, the JSON (version, `processing_cost_bound` 1,
`bounded_allowance` `<BASE>`, `period` fixed 604800, `growth_rule` none),
lease 900, and `identical` for all four evidence settings; both logs `none`.

Startup refusals, by label (all from `trace-commons-ingest.rs` and
`trace_commons_ingest_internal/admission.rs`): `account_admission_permissions_or_linkage_not_ready`
(step 2a/2b), `account_admission_policy_invalid` (bad JSON, version mismatch,
or `cost_bound > allowance`), `account_admission_evidence_policy_invalid`
(partial or invalid evidence set), `account_admission_requires_legacy_evidence_policy`
(legacy set missing), `account_admission_lease_invalid`. A refusing ingest
crash-loops every 10 seconds (`RestartSec=10`), so roll back at once.

### Post-checks

1. Server side, as above: active, no restarts, names and values as expected.
2. Behavior, from a client signed into a `nearai-` account (step 6a). The
   status flips from `legacy_evidence` to `bounded`.
3. Watch `ingest.err` and `ingest.log` through the first real `nearai-` uploads
   (strip ANSI; `refusals_since` from the prelude does it). An attested upload logs no
   line; `authority_kind` on the reservation row is the audit record
   (`account-trust.md`). A refused one is visible to the client as
   `403 admission_refused`, `422 source_session_invalid` or
   `429 account_limit_reached`.

### Rollback, step 5

```bash
tcrun <<'EOF'
sudo rm /etc/systemd/system/trace-commons-ingest.service.d/account-admission.conf
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
sleep 12
systemctl is-active trace-commons-ingest || true
names
EOF
```

With the drop-in gone, `near-`/`nearai-` traffic goes back to the evidence path
and needs evidence again (`contribution-status` answers `legacy_evidence`).
Persists: `trace_account_admission_budget` rows (they freeze the policy
numbers per version), `trace_account_admission_submissions` rows, and source
session claims. Re-enabling later must use the same policy numbers under the same
version or a new version; a different number under the same version refuses
unattested reservations (freeze rule).

**Rollback is one-way for a migrated device.** The client retires the legacy
key as the last step of a migration (`daemon/legacy_migration.rs`). If account
admission is rolled back after a legacy device has migrated, that device is a
`nearai-` device that needs evidence and has no legacy identity to fall back
to. Let account admission settle before step 6c.

## Step 6: client verification

### 6a. A `nearai-` account reads its status

With a 0.13.x CLI (`contributor-v0.13.1` or later), signed into a `nearai-`
account (`trace-commons-contributor account login`, `--no-browser` on a
headless machine; source: `AccountAction` in `crates/trace-commons-contributor/src/cli.rs`):

```bash
trace-commons-contributor --json account status
```

`--json` is a global flag. Read `contribution_status`:

- before step 5: `authority` `legacy_evidence`;
- after step 5: `authority` `bounded`, `ready` `true`, `policy_version`
  `pilot-weekly-v1`; for an account that has spent its allowance, `ready`
  `false` with `refusal_label` `account_limit_reached` and a
  `retry_after_seconds` toward the next Thursday 00:00 UTC (an attested upload
  from it would still be admitted).

A legacy `tenant-…` session still answers this route `403
account_identity_unlinked` (`account-trust.md`); that is expected, not a fault.

Stop if `authority` is still `legacy_evidence` after step 5 (the switch is not
live in the serving process) or `ready` is `false` for a fresh account.

### 6b. A legacy device still uploads

Confirm `tenant-zaki-pilot` and a pooled Devfolio device each still upload and
reach `accepted`, with the real client's normal submit and History. For a
synthetic check use the released-client smoke in
[`pilot-cutover-2026-09.md`](./pilot-cutover-2026-09.md), "Smoke test with a
released client" (always `--source trajectory --trajectory <file>`).

### 6c. The macOS move on the owner's Mac, last

Only after 6a and 6b are green and a real `nearai-` upload has been admitted
and accepted. The client migration is
`legacy_invite_migrate` (`daemon/legacy_migration.rs`): it provisions a second
key, signs into NEAR AI, asks `contribution-status` and goes on only on a
`ready` answer (so a not-ready server leaves the legacy identity fully
working), then links with the legacy key and switches. The final step retires
the legacy key; see the one-way note under step 5's rollback.

The server side of the move needs linking on (step 4). The owner's tenant
`tenant-zaki-pilot` is an individual tenant, not pooled, so it can link.

## Step 7: growth (placeholder)

Not part of this switch-on. `growth_rule` stays `"none"`, and nothing below is
set. The earned-trust growth step is a separate, later change on its own
branch, expected to bring: tier policy `tiered-v1`, migration V126, the
`TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_*` settings, and a scoped worker token.

**PLACEHOLDER: fill in from the growth PR when it lands (#TBD). No commands are
given here on purpose.** Two things this runbook already fixes for it:

- The policy version is frozen by the stored budget rows. Growth needs a
  version bump to `growth_rule: external` with its own `growth_policy_version`,
  `allowance_ceiling` and `evaluation_max_age_seconds`
  (`parse_bounded_policy`, `account_trust.rs`), and a spend bucket shared
  across the bump (`account-trust.md`).
- Do not enable external growth below V115 or without the frontier-row check
  (`account-trust.md`, the `trace_accounts` / `trace_account_trust_frontiers`
  query).

## What persists, by step

| Step | Undo | What stays |
|---|---|---|
| 1 | Reinstall `.bak-<stamp>`, restart | V125 (forward-only; harmless to the old binary), the backup, and `.bak-*` binaries |
| 3 | Optional `DELETE` of the marker | Markers; keep them once linking is on |
| 4 | Remove drop-in, restart | Links, invite grants, `invited` authority, link audit rows. Switching off only returns 503 on the routes. |
| 5 | Remove drop-in, restart | Budget rows (they freeze the policy per version), account-ledger submissions, source-session claims; a migrated device cannot fall back to its legacy key |

## Open points for the owner

1. **`<BASE>`** (recommended 100) is permanent per version. Accept that bounded
   accounts have no global cap.
2. **Counts may be unmeasurable** without a `BYPASSRLS` role (step 2c); the
   boolean from 2a/2b is then the only readiness evidence, and the count of
   0.12.x `near-`/`nearai-` devices stays unknown.
3. **Conditions in `account-trust.md`** (Z4/Z5 readiness, approved R1-R7 copy,
   the fleet inventory, and a client that retries refused entries; 0.13.1
   holds folders but does not retry refused entries): this runbook assumes the
   2026-10-11 decision accepts them as met.
4. **Witness `source_session` stripping**: the pinned witness image predates
   #1021 and drops the field. Since #1373 a witnessed upload with verified
   evidence and no `source_session` takes the V59 ledger instead of being
   refused. Whether the live witness still strips it can be read from the CVM
   (`deploy/witness/README.md`: the `phala cvms get <cvm-id> --json` read-back);
   it matters for whether the owner's Mac uploads count against the allowance
   path or the evidence path, not for whether they are admitted.
