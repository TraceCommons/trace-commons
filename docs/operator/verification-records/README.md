# Client end-to-end verification pass records

One file per released version, named `app-v<version>.md`, recording that a
human installed the real contributor artifact on macOS, Linux and Windows and
drove it through to an accepted submission.

- The procedure: [`../client-end-to-end-verification.md`](../client-end-to-end-verification.md)
- The template: [`./TEMPLATE.md`](./TEMPLATE.md)
- The gate: [`../../../scripts/operator/check-verification-record.sh`](../../../scripts/operator/check-verification-record.sh),
  run by the `version` job of `.github/workflows/release-apps.yml` on every
  `app-v*` tag push. Its self-test is
  [`../../../scripts/operator/test-check-verification-record.sh`](../../../scripts/operator/test-check-verification-record.sh).

A tag whose version has no complete record here does not release.

## What belongs in a record

Hashes, labels, counts, and written observations. The same hash-only rule the
rest of this directory follows ([`../hash-only-logging.md`](../hash-only-logging.md)):
no invite codes, no bearer tokens, no filesystem paths from the operator's
machine, no trace content, no contributor identity. The artifact is identified
by its SHA-256 and the invite by its hash, because both of the raw forms are
live credentials or live provenance.

## What a record is not

It is not evidence that the software is correct; the unit and integration
suites are for that. It is evidence that the artifact a contributor downloads
can be installed and used, which is the specific thing every other check in
this repository is blind to.

## Backfill

There is no record for `app-v0.1.0` through `app-v0.3.0`. Those releases
predate this procedure, and 0.3.0 in particular is known not to have worked on
macOS. Do not backfill them; a fabricated pass record is worse than a missing
one.
