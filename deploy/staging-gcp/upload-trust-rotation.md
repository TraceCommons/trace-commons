# Staging upload-trust worker renewal

These deployment artifacts keep the scoped worker credential renewable after
its seven-day initial lifetime. The root helper checks daily and renews only
when 48 hours or less remain. It uses the installed issuer's Ed25519 key,
current managed public-key set and existing verifier issuer/audience/TTL;
it creates no signing key, database role, IAM binding or account privilege.
The selected TTL is at most seven days and at most the verifier's limit.
A configured limit below three days requires a separately reviewed timer
cadence; this daily helper refuses it. No bearer, private key, plaintext
claims or account references appear in output. Receipts contain status,
expiry, secret version and a SHA-256 of the random jti.

## Exact additional permission

Grant the staging runtime service account this predefined role **on only**
`tc-staging-account-trust-worker-token`:

```
roles/secretmanager.secretVersionAdder
serviceAccount:tc-staging-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com
```

The feature's existing per-secret `secretAccessor` grant remains required.
No project-wide role, secret administration/IAM role, database grant or operator
ADC credential is needed. The helper fetches the VM's own temporary access token
through authenticated metadata and calls only `versions/latest:access` and
`:addVersion` for that exact worker secret. It never rotates the database
password or changes the feature's policy, mode, caps, main ingest environment,
scoring, payouts or admission switches.

## Prerequisites and installation

The exact approved staging VM/project/service-account identity is checked on
every configured run. The feature environment must be the root-owned reviewed
nine-setting file `/etc/tracecommons/client-upload-trust.env`, produced by the
activation procedure. The deployed ingest already refuses startup unless both
trust readiness contracts pass. When tenant-access-grant enforcement is enabled, provision
a legitimate service-principal grant with role `account_trust_worker`, matching
the configured worker tenant, issuer, audience and subject. Token minting does
not create that grant. Do not disable the grant gate or substitute a contributor
or admin grant. Provision/reuse the actual reviewed worker grant through the
normal operator procedure before starting the scheduler. `python3-cryptography` must be present on the
VM; it was observed installed during read-only preflight. There are no pip or
repository dependency additions.

First run the offline suite from this directory:

```
python3 -m unittest discover -v
```

After reviewing and hash-verifying the transferred files, install as root.
Preserve a timestamped copy of any existing helper/unit before replacing it:

```
install -o root -g root -m 0755 rotate_upload_trust.py /usr/local/sbin/tc-staging-upload-trust-rotate
install -o root -g root -m 0644 tc-staging-upload-trust-rotate.service tc-staging-upload-trust-rotate.timer /etc/systemd/system/
systemd-analyze verify /etc/systemd/system/tc-staging-upload-trust-rotate.service /etc/systemd/system/tc-staging-upload-trust-rotate.timer
systemctl daemon-reload
systemctl start tc-staging-upload-trust-rotate.service
systemctl enable --now tc-staging-upload-trust-rotate.timer
systemctl list-timers tc-staging-upload-trust-rotate.timer
systemctl show tc-staging-upload-trust-rotate.service -p Result -p ExecMainStatus
journalctl -u tc-staging-upload-trust-rotate.service --since -10min --no-pager
```

The first normal invocation should report `not_due` with the current expiry;
a missing feature file is a successful no-op (also skipped by the systemd
condition). Timer installation and a successful `not_due` run do not prove a
live rotation occurred. Verify the runtime account can access/add versions on
only the worker secret. The offline suite exercises actual signatures,
applied/shadow mode selection, failed-summary refusal, 48-hour boundaries, role/tenant scope, atomic file replacement, failed restart
and readiness rollback. Before claiming a live renewal, retain its safe receipt,
fresh process token equality, worker readiness and unchanged main-config hash.

## Write and rollback behavior

Renewal adds the new worker-secret version first, atomically changes only
`TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN` in the feature environment with
root:tracecommons 0640, and restarts ingest. It verifies staging health,
unchanged build, exact running feature settings, disabled admission, and a
successful narrowly authenticated evaluation request. This request can refresh
one legitimate evaluation; it creates no trace or credit event.

On failure with a still-valid previous credential, the helper restores the
previous feature file, adds the previous valid secret value as a new latest
version, restarts and rechecks it. It attempts every rollback step and reports
whether rollback completed. It never assumes disabling the new secret version
makes the old version the `latest` alias. When the previous file token is expired
(or has less than five minutes remaining), no valid rollback exists: retain the
new candidate and fail visibly for operator recovery. The underlying feature
configuration files allow recovery even when ingest cannot start with an
expired old token. The issuer and its managed public-key endpoint must still
be available. Malformed or wrong-scope old credentials refuse rather than being
silently replaced.

The helper serializes itself with `/run/tc-upload-trust-rotation/lock`; use this
same exclusive lock around any manual activation/disable or feature-file change.
It also detects a file changed since its snapshot and refuses to overwrite it.

Inspect failed service results and renew before the reported expiry. Without
renewal, the worker fails at expiry and, as evaluations become stale, clients
lose earned uplift; an expired configured token also refuses an ingest restart.
There is no separate paging integration in these artifacts.

To revert the helper, disable the timer, wait for any in-progress oneshot to
finish, then restore the reviewed helper/unit backups and restart the timer.
Do not kill a rotation midway. Removing the timer while leaving earned trust
active requires another renewal mechanism before expiry. To disable the
feature itself, use the activation procedure's disable mode, which removes
both the dedicated environment and drop-in and verifies the null allowance.
