# Staging award calibration

This records the owner's October 27, 2026 direction to reduce staging rewards
approximately fivefold after first-run onboarding showed $6.75–$18 for nine
traces. It is a prospective configuration change. Merging this document does
not change the running service.

## Target

Keep the published conversion at $0.25 per point and reduce the new scoring
award from five points to one. Changing the amount recorded on a new award
preserves previously recorded credit, including credit not yet settled.
Changing the dollar conversion instead could change what existing unaccrued
credit will eventually pay, so it is not this cutover's mechanism.

| Setting | Expected before | Target after |
|---|---:|---:|
| `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | 5 | 1 |
| `TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MIN` | 750000 | 150000 |
| `TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MAX` | 2000000 | 400000 |

At the unchanged conversion, a new qualifying trace earns $0.25 rather than
$1.25. Nine newly rewarded qualifying traces earn $2.25 rather than $11.25. The corresponding
first-run band for nine detected traces falls to $1.35–$3.60.

This is a fivefold new-award reduction, not a claim that the preview has become
an empirically fitted forecast. The preview still multiplies an operator's
per-accepted-trace band by the number discovered. Rejections, failed gates and previously rewarded submissions can reduce new
credit earned, including to zero. Payout holds can delay or block receipt of
that credit without reducing the recorded award. Fit future estimates against acceptance and finalized
settlement results; label the observation period and sample size.

## Before applying

1. Authenticate the operator to GCP project `tracecommons-pilot-2026`, zone
   `us-central1-a`, instance `tc-staging-host`. Confirm the host identity and
   active `trace-commons-ingest` unit before touching configuration.
2. Read only the three keys above from both `/etc/tracecommons/ingest.env` and
   the active ingest process environment. Refuse duplicates, missing keys or
   a mismatch with the expected values; investigate rather than assuming the
   historical five-point observation is still current.
3. Check the public first-run endpoint and settlement table. The expected
   conversion is `usd_micros_per_credit=250000` with
   `atomic_units_per_credit=1000000`. Stop if that rate has changed. Record
   the table's version and pricing version.
4. Record the service start time and read-only aggregate counts/totals for
   existing award events, settlement batches, accruals and payout lines.
   Store this operator evidence locally without publishing account identifiers
   or secrets. Preserve the existing settlement cutover timestamp and guards.

## Apply and verify

Back up the environment file with its original restricted permissions and
ownership. Atomically replace only the three settings above, checking that
all other bytes are unchanged. Do not regenerate the entire environment from
renderer defaults: that can reset independently enabled staging services.
Restart only `trace-commons-ingest`, then record its new start time. In-flight
work on the old instance may finish before shutdown; identify new-award
behavior against the completed restart, not the moment the file was edited.

Verify the service is active and `/health` succeeds. Read the same three keys
from the new active process and confirm their target values. The endpoint
`https://ingest.staging.tracecommons.ai/v1/public/first-run-estimate` must
return min 150000 and max 400000. The settlement table's rate, version and
pricing version must remain unchanged.

Observe the next naturally occurring qualifying award and its finalized
settlement: one point must be recorded, corresponding to $0.25 at the current
rate. Do not rescore or replay previously rewarded submissions to test this;
the one-paid-award-per-submission guard remains in force. Until this fresh
award is observed, report configuration verified separately from payout
verification. Existing recorded award amounts are unchanged by this procedure. Unchanged old totals cannot
be required while normal submissions and settlements continue; compare
pre-cutover event identities and values instead.

Keep issuer approval, production-gate requirements, consent, the original
`TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM`, settlement eligibility, payout
mode, account ceilings and holds as configured. No database migration,
ledger update, new rate epoch, retroactive reduction, refund or clawback is
part of this change. Production is outside its scope.

If the service fails to restart or verification fails, restore the restricted
backup and restart the ingest. Do not revert ledger events: any award already
written at one point remains one point. A rollback resumes the previous
configuration for subsequent awards.

## Keeping deployment settings aligned

On a subsequent normal staging render, pass `TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`,
`TC_FIRST_RUN_USD_ESTIMATE_MICROS_MIN` and `TC_FIRST_RUN_USD_ESTIMATE_MICROS_MAX`
with the new values, along with the deployment's existing switches and
`TC_NOVELTY_UTILITY_SETTLE_FROM`. These are operator inputs; the committed
fail-closed templates retain their unset/zero defaults. See
[the staging deployment guide](../../deploy/staging-gcp/README.md#scoring-driver-and-scoring-award)
and [gate calibration](calibration.md).
