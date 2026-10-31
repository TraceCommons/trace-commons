# Staging runbook

For the operator. It covers what exists today for the staging environment
and marks what is still to come. The design is in
`docs/superpowers/specs/2026-10-21-staging-environment-design.md` and the
plan in `docs/superpowers/plans/2026-10-21-staging-environment.md`. Testers
use [`./staging-tester-guide.md`](./staging-tester-guide.md).

Status as of 2026-10-22: the macOS build and update feed exist. The staging
server does not, and the first pin set is not served.

## Make a staging build

Dispatch the `staging-macos` workflow from `main`. It is manual, and it has
no approval gate (S14). Choose a new staging release version for each build;
for example, advance `0.21.0` to `0.22.0` for the next release.

```bash
gh workflow run staging-macos.yml --repo TraceCommons/trace-commons --ref main -f version=0.21.0
```

To build another commit, dispatch from `main` and pass the commit in the
`ref` input:

```bash
gh workflow run staging-macos.yml --repo TraceCommons/trace-commons --ref main -f version=0.21.0 -f ref=<branch, tag or commit>
```

A dispatch from any other branch is refused in preflight, because the
publisher trusts only runs from `main`.

The workflow runs three jobs:

1. `preflight` checks that all ten credentials exist, runs the release check
   (`scripts/ci/check-release-staging-feature.sh`) on the commit, and fixes
   the version and tag.
2. `macos` builds with `TC_CHANNEL=staging`, signs with the Developer ID,
   notarizes, staples, and writes the signed appcast.
3. `publish` uploads to the staging updates bucket and creates the GitHub
   pre-release.

The version is the requested staging release version with a `-staging.<run>`
suffix, independent of the crate versions. Preflight refuses a version at or
below any previously published staging release. Failed, unpublished builds
can retry the same release version. The numeric build number still increases
across dispatches and attempts so installed staging apps can update.
The tag is `staging-<date>-<sha7>-<run>`. The release is a pre-release and
is never marked latest.

## Check it published

1. The run is green. Watch it:

   ```bash
   gh run list --repo TraceCommons/trace-commons --workflow staging-macos.yml --limit 3
   ```

2. The pre-release exists and carries the `.dmg` and the `.sha256`:

   ```bash
   gh release view <tag> --repo TraceCommons/trace-commons
   ```

   It must say "Pre-release", and `gh release list` must not show it as
   Latest.

3. The feed names the new version:

   ```bash
   curl -fsS https://storage.googleapis.com/tc-staging-updates-20261022/staging/appcast.xml
   ```

4. Check the DMG against its checksum, as a tester would (see the tester
   guide).

## The update feed

- Feed: `https://storage.googleapis.com/tc-staging-updates-20261022/staging/appcast.xml`.
  It comes from `macos/channels/staging.env`.
- The bucket is public and holds only the feed and the DMGs. It is not the
  staging server's bucket. Never grant public read on the server's bucket
  (S15).
- The DMG is uploaded first and the appcast second, so the feed never names
  a missing file. The appcast is cached for 300 seconds, so a new build
  reaches testers within minutes.
- The appcast is signed with the staging Sparkle key
  (`TC_STAGING_SPARKLE_PRIVATE_ED_KEY`, a repository secret). The workflow
  checks the signature against the public key before it publishes.

## Re-sign the pin set

See [`./staging-pin-signing.md`](./staging-pin-signing.md). Re-sign when NEAR
AI staging redeploys a model and its measurements or allowlisted events
change, and before the served set expires (at most 14 days after signing).
A change to the allowlist is a reviewed pull request first. Signing runs on an
operator machine. The staging host never holds the key.

After a NEAR AI staging redeploy, also check that testers can sign in
again. A redeploy rotates the JWT secret and signs people out.

## Shut staging to everyone

Sign-up is open to any staging NEAR AI account (S12), so there is no list to
keep. There is no per-person switch. To shut staging to everyone, stop the
staging ingest service.

## Server bring-up

The order, the secret names and how to make each, and the commands are in
[`deploy/staging-gcp/README.md`](../../deploy/staging-gcp/README.md). In
short: build with `deploy/staging-gcp/cloudbuild.yaml`; fetch the binaries
with `deploy/pilot-gcp/pull-and-install.sh` in fetch-only mode; create the
missing `tc-staging-*` secrets; render with
`deploy/staging-gcp/render-env.sh`; install and first boot with
`deploy/pilot-gcp/deploy.sh` and its staging parameters, which refuses an
ingest whose `/health` does not report `near_ai_staging: true`; create the
login-resolver role; verify `/health`, the provisioning capabilities and the
AASA file. Rollback is the pilot's: each install keeps a timestamped
`.bak-*` beside the binary (`docs/operator/pilot-gcp-deployment.md`). The
payout rail stays `disabled` until the ingest reads its settings (payout
design section 2).

## Database backups

Staging's PostgreSQL runs on the VM, so its backups are ours (design 3.2).
`tc-staging-db-backup.timer` runs `deploy/staging-gcp/backup-db.sh`
nightly: a `pg_dump -Fc` as the `postgres` superuser, with every table's row
count taken in the same snapshot, uploaded to
`gs://tc-staging-artifacts-20261021/backups/postgres/` with a sha256 sidecar
written last. `tc-staging-db-restore-check.timer` runs
`deploy/staging-gcp/restore-check.sh` weekly: it restores the newest
complete backup into a throwaway database and fails unless every table's
row count equals the dump-time count. Install, retention (a 14-day lifecycle
rule on the prefix) and a real restore: bring-up step 9 in
[`deploy/staging-gcp/README.md`](../../deploy/staging-gcp/README.md).

Checking on them:

```bash
systemctl list-timers 'tc-staging-db-*'
systemctl --failed
sudo journalctl -u tc-staging-db-backup.service -u tc-staging-db-restore-check.service \
  --since -8d --no-pager | grep -E 'backup-db:|restore-check:'
```

Both must reach PostgreSQL as a superuser: every Trace Commons table has
FORCE ROW LEVEL SECURITY, which binds its owner `app` too, so a dump as
`app` is refused (or, with `--enable-row-security`, holds only the rows
`app` can see). Do not change either script to connect as `app`.

## Serving the first pin set

Private AI proof marks stay off until a signed pin set is served. The RTMR3
replay (owner decision S16) is in place, so the first set is made the same
way as every later one, with
[`./staging-pin-signing.md`](./staging-pin-signing.md):

1. `observe` every model the staging app uses, with `--review review.json`.
2. Fill `deploy/staging-gcp/near-ai-rtmr3-allowlist.json` from
   `review.json`, checking each value, and merge it in a reviewed pull
   request. As committed it is a placeholder that refuses everything: a set
   signed with it serves, and earns no key.
3. `sign --first` with `--allowlist` pointing at the merged file, then
   `verify` the result.
4. Copy `current.json` to the staging host's pin directory, and put the
   printed `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=` line (production's
   form, from `observe`'s `ingest=` line) in the staging ingest's drop-in.

A tester's daemon status then shows `near_ai_pins: pinned` and, after the
first attested answer, `near_ai_attestation: attested`; any other
`near_ai_attestation` value names the check that refused (the refusals table
in the signing runbook).


## Earned client upload limits

Retired: the contributor clients no longer have a daily upload limit, so
the applied mode below no longer changes anything a client sees. It is
still accepted so an existing staging render keeps starting; do not enable
it anew. It keeps staging signup, both admission ledgers, witness
requirements, and payouts as they are. Follow the [staging earned upload trust procedure](
client-upload-allowance.md#staging-earned-upload-trust-opt-in) for database
schema/role/readiness checks, scoped worker-token storage, qualification,
rollout, rotation and rollback. Do not infer deployment from checked-in
configuration or local verification.

The approved distinct qualified-trace thresholds are 0/5/20/50. No age/week wait or
attestation bonus applies; gate pass, first-in-cluster novelty, count-once
and outcome netting remain required over the existing 84-day window.

Start with `TC_CLIENT_UPLOAD_TRUST_MODE=shadow` and the actual staging
`TC_UPLOAD_TRUST_GATE_POLICY_VERSION` and
`TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION` labels. Qualify recorder coverage and
boundary accounts before selecting `applied` with the same labels. The
renderer reads only the enabled feature's
`tc-staging-account-trust-worker-token` and
`tc-staging-db-upload-trust-password`, schedules every five minutes, and
keeps both admission switches explicitly false. Applied uses the
independent `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON`/`_VERSION`
pair. To disable, unset mode and both labels and rerender/restart.


The trust runtime requires schema through V138. It uses a separate
`tc_upload_trust_login` non-owner LOGIN with NOSUPERUSER, NOBYPASSRLS and
NOCREATEROLE, and the dedicated `trace_client_upload_trust_runtime` group and its reviewed
worker/status-read grants. Do not add `app`, guard or evaluator memberships or direct trust
fact/evaluation writes. The renderer sets
`TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL` for this pool in either
mode; the main `DATABASE_URL` and app runtime identity stay unchanged.
Startup requires matching database endpoint/name and the dedicated pool's
readiness controls. V137 removes the accepted-credit prerequisite for this
staging recorder path; preserve all other eligibility checks and never
fabricate historical credits to create trust history.
