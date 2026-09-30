# Dedup simhash re-derivation and recluster

How to move the cross-trace dedup signal from one simhash algorithm to
another without ever mixing the two in one cluster, and how to measure the
clustering before and after. Design and rationale:
[`../superpowers/specs/2026-09-21-dedup-simhash-recluster-design.md`](../superpowers/specs/2026-09-21-dedup-simhash-recluster-design.md).

## Why this exists

Every `trace_gate_decisions` row carries a 64-bit `dedup_simhash` of its
canonical rendered text and a `dedup_signal_version` stamp naming how it was
derived (`<render version>+<algorithm>`, e.g. `events.v1+fnv1a-2shingle.v1`;
a stored `NULL` is read as that legacy v1 stamp). Rows join a cluster only
against rows on the same stamp, at Hamming distance at most the algorithm's
`tau_hamming`.

The multiset 2-shingle v1 hash collapses on long, scaffolding-heavy agent
sessions: on the pilot 522 of 696 clustered rows sat in one cluster. The
set-semantic 3-shingle v2 (`fnv1a-3shingle-set.v2`) replaces it. Because
stamps never mix, the corpus has to be re-derived onto the new stamp
**before** the inline path starts writing it, or every resubmission in the
gap becomes a full-credit singleton (see the module docs at the top of
`crates/trace-commons-server/src/dedup_assign.rs`).

Two builds, in order:

1. **The pass build** carries `POST /v1/admin/rederive-dedup` and still
   derives v1 inline. Deploying it changes nothing until the route is called.
2. **The flip build** moves `ACTIVE_DEDUP_ALGORITHM` to v2 so the inline path
   stamps and clusters under v2. It is installed only after the pass has
   COMPLETED in write mode on the target database.

Neither carries a migration: every column the pass reads is already granted
to `trace_gate_driver` by V45 and V57, and every write goes through the
existing four-column dedup writer on the tenant-scoped pool.

## What nothing depends on today

`dedup_cluster_size` feeds two shadow contributor-cap columns
(`contributor_factor_micros`, `contributor_cumulative_raw_micros`) that
nothing reads back; no contributor-visible number and no ledger number
depends on it. That is why this is a re-derivation pass and not a
credit-correction event, and why the window between steps 7 and 9 below is
harmless to any number anyone sees. Keep it short anyway.

## The route

```
POST /v1/admin/rederive-dedup?algorithm=<name>[&dry_run=true][&limit=N]
Authorization: Bearer <admin JWT>
```

| parameter | meaning |
|---|---|
| `algorithm` | required; `fnv1a-2shingle.v1` or `fnv1a-3shingle-set.v2`. Any other value is a 400. |
| `dry_run` | derive and sweep, write nothing, log the report. Default `false`, which WRITES. |
| `limit` | bound the enumeration (oldest first), for a smoke. |

Unknown or misspelled parameters are a 400 (`dryrun`, `dry-run`), never read
as the default. Without a DB mirror or an artifact store the route is a 503.
The ack is `{"accepted":true,"limit":..,"mode":"dry_run"|"write","algorithm":".."}`;
check `mode` before walking away, because the work is fire-and-forget.

Per row the pass:

- reuses the stored value when the row is already on the target stamp (no
  load, no decrypt -- this is what makes a rerun cheap and a crash resumable);
- skips rows stamped `digest-prefix.v1` or the placeholder stamp
  (`not_derivable`: development and test services only);
- otherwise loads the exact ciphertext the gate scored, decrypts inside the
  gate service, renders the same canonical text the inline path hashes, and
  hashes it under the target algorithm. The plaintext never leaves the gate
  service.

A load or derive failure is counted `failed` and logged hash-only; the row
keeps its old stamp and value, so it stays refused against every re-derived
row until a rerun succeeds on it.

Then one sweep over every row in `decided_at` order, under the target
algorithm's constants, and in write mode every row whose four dedup columns
changed is written with the sweep's final cluster sizes. A rerun over a
converged corpus writes nothing and its completion line reads
`unchanged = rows`.

Completion line, in `/var/log/tracecommons/ingest.log`:

```
Trace Commons dedup re-derivation pass completed
  rows=.. derived=.. reused=.. not_derivable=.. failed=.. written=.. unchanged=.. write_failed=..
  algorithm=.. dry_run_report={..}
```

## The dry-run report

`dry_run_report` is one JSON object of aggregates. Below 20 target rows it
carries counts only (histogram and per-tau tables empty, tenant counts
`null`), so `limit=5` is a smoke test of the mechanism, not a preview of the
numbers.

- `rows`, `derived`, `reused`, `not_derivable`, `failed`; `target_rows` (rows
  on the target stamp after derivation) and `distinct_target_simhashes`.
- `nearest_representative_hamming`: for each target row, its Hamming
  distance to the nearest same-stamped cluster representative at the
  algorithm's own tau, in the buckets `0, 1-2, 3-4, 5-6, 7-8, 9-10, 11-12,
  13-14, 15-16, 17-20, 21-24, 25-32, 33-64`. The expected shape is a large
  mass at 20+ and a small mass at 0-6 with little between.
- `by_candidate_tau`: for each tau in `4, 6, 8, 10, 12, 14`, the would-be
  cluster count, singletons, clusters of 2-9, 10-99, 100+, the largest
  cluster's size and its share of target rows in micros. This is the table
  `dedup-cluster-report.sh` prints, so the projection and the measurement
  read the same way.
- `single_tenant_multi_member_clusters` / `multi_tenant_multi_member_clusters`
  at the algorithm's own tau. On the pilot a tenant is one contributor, so
  the first is resubmission or forking by one person and the second is the
  sybil case or a shared session. Counts only.

Choosing the threshold: the v2 constants start at `tau_hamming = 8`. Put the
threshold in the valley of the histogram. If the 8-14 band is populated, the
signal needs escalating (author-kind weighting, then MinHash), not the
threshold moving; see the spec's "Candidate signals".

## Measuring: `scripts/operator/dedup-cluster-report.sh`

Read-only `psql` as `trace_gate_driver`, touching only the columns that role
can read, printing counts only:

```sh
scripts/operator/dedup-cluster-report.sh \
  --db-url="$TRACE_COMMONS_GATE_DRIVER_DATABASE_URL"
```

It prints: rows with a cluster, cluster count, singletons, clusters of 2-9,
10-99 and 100+, the largest cluster's size and share; distinct simhash count
and zero-value count; rows per stamp (NULL shown as the legacy label); the
min / median / p95 / max Hamming distance from each member to its cluster's
earliest-decided member; and the median nearest-member distance inside the
largest cluster. Requires PostgreSQL 14+ (`bit_count`).

## Operator steps

Host facts: ingest runs as `tc_ingest_runtime_login`, which owns no table.
No migrator step is needed for either build. Use the admin JWT the
perplexity re-score runbook uses
([`perplexity-scoring-driver.md`](perplexity-scoring-driver.md)).

1. **Before anything**: run the report script and keep the output. This is
   the "before".
2. **Plant the control**: submit one already-scored session again from the
   test tenant (`scripts/operator/smoke-gate.sh` submits synthetic
   envelopes). Note its submission id locally, not in any log. Under v1 it
   lands in the big cluster and proves nothing; under v2 it is the one pair
   whose distance is known to be 0.
3. `git diff --name-only <running build_commit> <pass build commit> -- migrations`
   prints nothing. Plain build-and-install.
4. Install the pass build. Confirm `/health` reports its commit.
5. **Smoke**:
   ```sh
   curl -sS -X POST -H "Authorization: Bearer $ADMIN_JWT" \
     "$INGEST_BASE/v1/admin/rederive-dedup?algorithm=fnv1a-3shingle-set.v2&dry_run=true&limit=5"
   ```
   The ack says `"mode": "dry_run"`; the completion line reports `rows=5`.
6. **Full dry run** without `limit`. Read the report against the expected
   shape and choose the threshold. If it is not 8, change
   `DEDUP_CONSTANTS_V2.tau_hamming` in `dedup_assign.rs`, rebuild the pass
   build, reinstall, and re-run the dry run, so the written result is the one
   that was inspected.
7. **Write mode**, no `limit`:
   ```sh
   curl -sS -X POST -H "Authorization: Bearer $ADMIN_JWT" \
     "$INGEST_BASE/v1/admin/rederive-dedup?algorithm=fnv1a-3shingle-set.v2"
   ```
   Wait for the completion line. Duration is not estimated here; per row it
   is the perplexity re-score's load, unwrap, decrypt and render with a hash
   in place of the model call, so strictly cheaper per row than a re-score.
8. Run the report script again ("after, v2 rows"). Confirm: the largest
   cluster's share is no longer 75%; the stamp table shows the corpus on
   `events.v1+fnv1a-3shingle-set.v2` except `not_derivable` rows; and, with
   a read-only query as the driver role, the control's decision row and its
   original share a `dedup_cluster_id`.
9. Install the flip build (again no migration). From this moment inline
   writes stamp v2.
10. **Write mode once more.** This run reuses every stored v2 value and
    derives only the rows scored between steps 7 and 9; confirm `derived` is
    that window's row count.
11. `POST /v1/admin/recompute-contributor-caps` so the shadow cap columns are
    recomputed from the corrected sizes.
12. Report script a final time and file the before/after in
    `docs/superpowers/reports/`.

Run the pass in a quiet window: a submission scored while it runs is
assigned inline against the pre-pass state and is not in the pass's
snapshot. Step 10 closes that gap; running it twice is harmless.

## Rollback

- Flip build rolled back to the pass build: inline writes return to v1 while
  the corpus is on v2; new rows cluster among themselves until the flip
  build is reinstalled and step 10 repeated. Nothing fuses.
- Full rollback to a pre-pass build: it reads every v2 row's stamp from the
  column and refuses them against its own v1 rows. Nothing fuses.
- Re-deriving back to v1 is the same route with `algorithm=fnv1a-2shingle.v1`
  in write mode.

## What this does not touch

The renderer (`CANONICAL_RENDER_VERSION` stays `events.v1`), perplexity,
novelty, the vector index, gate status, `credit_quality_micros`, the
contributor-cap constants, any settlement, and the correction simhash
(pinned to v1 by name because correction clustering stores no stamp, #538).
The old `POST /v1/admin/recluster-dedup` route remains; it re-sweeps stored
values without re-deriving and now shares its sweep with this pass.
