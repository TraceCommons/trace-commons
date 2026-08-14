# NEAR benchmark handoff runbook

Use this one-off procedure to export the `tenant-zaki-pilot` accepted,
low-risk, evaluation-consented trace envelopes, package them as JSONL, and
deliver a checksummed pair of GCS objects to NEAR. Do not use the replay or
benchmark-conversion exports: neither emits the raw redacted envelopes required
for this handoff.

## 1. Pre-run verification

The pilot PostgreSQL role is subject to forced RLS. A missing tenant setting
can therefore return zero rows without an error. Every operator query must set
`trace_commons.trace_tenant_id` (note the doubled `trace_`) and immediately run
`SELECT trace_current_tenant_id();` before reading data. A zero-row result is an
unproven read, not evidence of an empty corpus.

Use the base64-over-SSH pattern so multiline SQL survives the IAP SSH command
boundary without shell interpolation:

```sh
PRECHECK_SQL="$(sed 's/^    //' <<'SQL'
    BEGIN;
    SELECT set_config(
      'trace_commons.trace_tenant_id',
      'tenant-zaki-pilot',
      true
    );
    SELECT trace_current_tenant_id();

    WITH eligible AS (
      SELECT submission_id
      FROM trace_submissions
      WHERE status = 'accepted'
        AND privacy_risk = 'low'
        AND consent_scopes ? 'debugging_evaluation'
        AND allowed_uses ? 'evaluation'
    )
    SELECT
      count(*) AS eligible_count,
      count(*) FILTER (
        WHERE EXISTS (
          SELECT 1
          FROM trace_object_refs object_ref
          WHERE object_ref.submission_id = eligible.submission_id
            AND object_ref.artifact_kind = 'submitted_envelope'
            AND object_ref.invalidated_at IS NULL
            AND object_ref.deleted_at IS NULL
        )
      ) AS object_ref_coverage_count
    FROM eligible;
    COMMIT;
SQL
)"
PRECHECK_SQL_B64="$(printf '%s' "$PRECHECK_SQL" | base64 | tr -d '\n')"

gcloud compute ssh tc-pilot-host \
  --zone us-central1-a \
  --project tracecommons-pilot-2026 \
  --tunnel-through-iap \
  --command "printf '%s' '${PRECHECK_SQL_B64}' | base64 --decode | sudo -u tracecommons bash -c 'set -a; source /etc/tracecommons/ingest.env; set +a; psql \"\$DATABASE_URL\" -X -v ON_ERROR_STOP=1 -P pager=off'"
```

Confirm the self-check prints exactly `tenant-zaki-pilot`. Record both returned
counts in the handoff ticket. `eligible_count` and
`object_ref_coverage_count` must be equal; stop if they differ. The prior
measurement was 331/331 on 2026-08-13, but it is historical context only—use
the freshly measured eligible count as `EXPECTED_ITEM_COUNT` below.

## 2. Run the export

The export-worker bearer must be in `TRACE_COMMONS_EXPORT_WORKER_BEARER`. Keep
the response in a mode-0700 working directory because it contains the corpus:

```sh
umask 077
HANDOFF_WORK_DIR="$(mktemp -d)"
EXPORT_JSON="$HANDOFF_WORK_DIR/raw-envelope-export.json"

curl --silent --show-error --fail-with-body \
  --request POST \
  --header "authorization: Bearer ${TRACE_COMMONS_EXPORT_WORKER_BEARER}" \
  --header "content-type: application/json" \
  --data '{
    "purpose": "near_benchmark_handoff",
    "status": "accepted",
    "privacy_risk": "low",
    "consent_scope": "debugging_evaluation"
  }' \
  --output "$EXPORT_JSON" \
  https://ingest.tracecommons.ai/v1/workers/raw-envelope-export
```

All four filters are mandatory. Omitting any of them must return HTTP 400; do
not weaken or work around that guardrail.

## 3. Enforce the stop condition

Set `EXPECTED_ITEM_COUNT` to the freshly recorded pre-run eligible count, then
compare it with the response before packaging:

```sh
EXPECTED_ITEM_COUNT=<fresh-eligible-count>
ACTUAL_ITEM_COUNT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["item_count"])' "$EXPORT_JSON")"
test "$ACTUAL_ITEM_COUNT" -eq "$EXPECTED_ITEM_COUNT" || {
  echo "STOP: export item_count=$ACTUAL_ITEM_COUNT expected=$EXPECTED_ITEM_COUNT" >&2
  exit 1
}
```

If the export is short, stop and investigate corpus, consent, policy, and
object-ref drift. Do not package or deliver a partial result.

## 4. Package the handoff

```sh
PACKAGE_DIR="$HANDOFF_WORK_DIR/package"
python3 scripts/operator/near-benchmark-handoff.py \
  --input "$EXPORT_JSON" \
  --output-dir "$PACKAGE_DIR"
```

This writes `corpus.jsonl` and `handoff-manifest.json`. The converter refuses
any item whose `privacy_risk` is not `low`. Record the printed
`corpus_sha256`; it must be communicated to the recipient out of band.

## 5. Deliver through a dedicated GCS bucket

Use a bucket dedicated to third-party handoffs. It must not be the artifact
bucket used by `trace_artifact_store`, and object versioning must be off.
Upload to an export-ID-scoped prefix:

```sh
EXPORT_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["export_id"])' "$EXPORT_JSON")"
HANDOFF_BUCKET=<dedicated-handoff-bucket>
OBJECT_PREFIX="near-benchmark-handoff/${EXPORT_ID}"

gcloud storage buckets describe "gs://${HANDOFF_BUCKET}" \
  --format='value(versioning_enabled)'
gcloud storage cp \
  "$PACKAGE_DIR/corpus.jsonl" \
  "$PACKAGE_DIR/handoff-manifest.json" \
  "gs://${HANDOFF_BUCKET}/${OBJECT_PREFIX}/"
```

The versioning check must report false or empty. Grant the recipient's named
Google identity time-limited `roles/storage.objectViewer` access restricted to
`near-benchmark-handoff/${EXPORT_ID}/`. Prefer that attributable, revocable IAM
grant over a signed URL. If a signed URL is unavoidable, treat it as a secret
and use the shortest practical TTL.

Send the manifest's `corpus_sha256` through a separate channel. Confirm the
recipient fetch in Cloud Audit Logs, then remove the IAM grant and delete both
objects. Because versioning is off, deletion does not leave a recoverable
object generation.

State the withdrawal limitation in writing with the handoff: contributor
withdrawal removes a trace from Trace Commons, but cannot recall a copy already
fetched by the recipient. Grant expiry limits re-fetch; it does not impose a
recipient-side retention limit.

## 6. Recover a failed export job

Do not call `/v1/workers/raw-envelope-export` again after a job failure; that
would issue a fresh grant and split one handoff across multiple grant lineages.
After investigating and correcting the cause, queue the existing job with the
admin retry endpoint:

```sh
FAILED_EXPORT_JOB_ID=<failed-export-job-id>
curl --silent --show-error --fail-with-body \
  --request POST \
  --header "authorization: Bearer ${TRACE_COMMONS_ADMIN_BEARER}" \
  --header "content-type: application/json" \
  --data '{"reason":"retry NEAR benchmark handoff after investigated failure"}' \
  "https://ingest.tracecommons.ai/v1/admin/export/jobs/${FAILED_EXPORT_JOB_ID}/retry"
```

Then let the export worker claim and run the queued
`raw_envelope_corpus` job. Re-run the count comparison before packaging its
response.
