# Dedup simhash v2 re-derivation and recluster

Date: 2026-09-22 (all times UTC)
Status: **the write-mode pass completed on the pilot 2026-09-22 18:08-18:15Z
under the pass build, and the flip build (#980, merged 18:45Z as `dd0a581fd`)
was installed at 19:19Z.** The corpus and the inline path are on the v2
stamp; the mixed-stamp window described in `dedup_assign.rs` was open from
18:15Z to 19:19Z and exactly one trace was scored in it. Steps 10-12 are
complete and recorded at the end of this document.

Runbook: [`../../operator/dedup-recluster.md`](../../operator/dedup-recluster.md)
(this report is its step 12). Design:
[`../specs/2026-09-21-dedup-simhash-recluster-design.md`](../specs/2026-09-21-dedup-simhash-recluster-design.md).

## Summary

The v1 simhash (`fnv1a-2shingle.v1`, multiset 2-shingle) put 522 of the
pilot's 696 clustered gate decisions (75%) in one cluster, because
scaffolding shingles repeated thousands of times in long agent traces owned
all 64 bits. Re-deriving the corpus under the set-semantic 3-shingle v2
(`fnv1a-3shingle-set.v2`, tau 8) leaves 1427 clusters over 1758 rows with
the largest at 48 (2.7%), and the nearest-representative histogram puts
unrelated traces at Hamming 15-24. There is no clean valley: 405 rows (23%)
sit in the 3-14 band, which the spec's rule says is a signal to escalate
(author-kind weighting, then MinHash) rather than a threshold to move, and
that escalation is the next iteration, not this one.

## What was wrong

Measured 2026-09-21 on the pilot, read-only, as the narrow
`trace_gate_driver` role (recorded in the spec's "Problem" section):

| statistic | value |
|---|---|
| `trace_gate_decisions` rows with a `dedup_cluster_id` | 696 |
| clusters | 67 |
| largest cluster | **522 rows (75% of clustered rows)** |
| members within Hamming 10 of the cluster's earliest member | all 522 (median 6) |
| median nearest-member Hamming distance inside it | 0 |
| distinct `dedup_simhash` values among the 522 | 246 |

The cause was the hash, not the linkage and not the embedding arm.
`trace_simhash_v1` (`crates/trace-commons-server/src/dedup_simhash.rs`)
adds +1/-1 into each of 64 accumulators for **every occurrence** of every
2-token shingle. In a long agent session most of the canonical text is tool
output, and tool output repeats a small vocabulary of shingles (JSON keys,
paths, line-number prefixes, the harness's own headers) thousands of times.
A handful of shingles at 1,000+ occurrences outweigh tens of thousands seen
once, and every long trace from the same harness lands within a few bits
of every other one. `DEDUP_CONSTANTS_V1.tau_hamming = 10` was set from unit
tests on 12-15 token sentences, which never exercise a text long enough for
occurrence counts to matter.

Assignment was already representative-based (each new row is compared
with each cluster's earliest-decided member only, never with every member;
`dedup_assign::assign_cluster` and `sweep_clusters`), so single linkage did
not chain the corpus. The embedding arm has never fired in production:
every production call passes `embed_cosine_micros: None`.

Nothing contributor-visible and nothing in a ledger reads
`dedup_cluster_size` today. It feeds two shadow contributor-cap columns
(`contributor_factor_micros`, `contributor_cumulative_raw_micros`) that
nothing reads back. That is why this was a re-derivation pass and not a
credit-correction event.

## What changed

**The v2 hash.** `trace_simhash_v2` in `dedup_simhash.rs`: same tokenizer
(lowercase, split on non-alphanumerics), overlapping 3-token shingles,
FNV-1a per shingle, and the shingle hashes are collected into a `HashSet`
before the vote, so each distinct shingle contributes exactly once however
often it occurs. Width falls back to 2 for a two-token text and to the
unigram for one token. The name is `fnv1a-3shingle-set.v2`; a stored row's
`dedup_signal_version` is `events.v1+fnv1a-3shingle-set.v2`.

**The constants** (`dedup_assign.rs`):

| | v1 | v2 |
|---|---|---|
| `tau_hamming` | 10 | 8 |
| `tau_e_micros` | 150,000 (cosine distance 0.15) | 30,000 (0.03) |
| `version` | 1 | 2 |

`tau_hamming = 8` is the spec's starting value from the unit-weight simhash
model: a J 0.9 pair (late fork, light edit) sits at Hamming 6.7 +/- 2.4 and
a J 0.6 pair (same-project sessions sharing a large file read) at 14.7 +/-
3.4; at 8 the J 0.6 pair lands inside about one time in twenty-five and
the J 0.9 pair about three times in four. `tau_e_micros` moves in the same
bump because 0.15 is the median cosine distance between two unrelated
traces on bge-large-en-v1.5 (pilot novelty p05 0.123, p50 0.164, p95
0.291), so wiring the arm at 0.15 would have chained the corpus on its own.
The arm stays unwired.

**The pass** (#978, merged 2026-09-22 04:03Z): `POST
/v1/admin/rederive-dedup?algorithm=<name>[&dry_run=true][&limit=N]`. Per
row it reuses the stored value if already on the target stamp, skips
`digest-prefix.v1` and placeholder rows, and otherwise loads the exact
ciphertext the gate scored, decrypts inside the gate service, renders the
same `events.v1` canonical text the inline path hashes, and hashes it under
the target algorithm. Then one `sweep_clusters` over every row in
`decided_at` order under the target constants; in write mode each row whose
four dedup columns changed is written with the sweep's final sizes. A row
the pass could not derive keeps its simhash and stamp but is still swept
under its own stamp, and its two cluster columns are written if they
changed (`trace-commons-ingest.rs`, `run_rederive_dedup_pass`). The pass
build still derives v1 inline; no migration in either build.

**The loader fix** (#982, merged 2026-09-22 17:24Z). The first dry run
exposed that the pass asked only for the active `submitted_envelope`
object ref. The PII backstop's rescrub invalidates that ref and stores an
active `rescrubbed_envelope` in its place, so every backstop-released trace
failed to load. The helper now prefers the rescrubbed ref and falls back to
the submitted one. The same fix reached the export revalidation and the
DB-reconciliation drill, which had been counting every backstop-released
trace as a blocking gap.

**The flip** (#980, merged 2026-09-22 18:45Z as `dd0a581fd`): a one-commit
change of `ACTIVE_DEDUP_ALGORITHM` to `DedupAlgorithm::V2` plus its tests,
rebased onto main after #978 so the PR was the flip alone. Installed on the
pilot as build `dd0a581f` at 19:19Z.

## Dry runs

**First dry run**, build `5f51f213` (#979's merge commit, which carries
#978), about 2 minutes:

```
rows=1802 derived=585 failed=1217
```

1215 of the 1217 failures were `trace gate worker requires an active
contribution envelope object ref`. Decisions lacking an active submitted
envelope, by month: July 1 of 417, August 563 of 718, September 651 of 667.
That is the backstop-rescrub shape above, fixed by #982. On the 585 rows
that did derive, the largest would-be cluster at tau 8 was 38.

**Second dry run**, build `ea5f75fe` (#981's merge commit, which carries
#982), about 6 minutes:

```
rows=1802 derived=1756 failed=46
target_rows=1756 distinct_target_simhashes=1585
single_tenant_multi_member_clusters=95 multi_tenant_multi_member_clusters=3
```

44 of the 46 failures had no active envelope of either kind, consistent
with revoked, expired or purged submissions; 2 are unidentified (see "Not
established").

Nearest-representative Hamming histogram (v2, tau 8), one entry per target
row, distance to the nearest same-stamped cluster representative:

| bucket | 0 | 1-2 | 3-4 | 5-6 | 7-8 | 9-10 | 11-12 | 13-14 | 15-16 | 17-20 | 21-24 | 25-32 | 33-64 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| rows | 109 | 18 | 59 | 72 | 73 | 55 | 49 | 97 | 181 | 801 | 233 | 8 | 0 |

Would-be clustering at each candidate tau (`DRY_RUN_CANDIDATE_TAU_HAMMING`):

| tau | clusters | singletons | 2-9 | 10-99 | 100+ | largest | largest share |
|---|---|---|---|---|---|---|---|
| 4 | 1506 | 1426 | 74 | 6 | 0 | 36 | 2.05% |
| 6 | 1468 | 1381 | 80 | 7 | 0 | 47 | 2.68% |
| **8** | **1425** | **1329** | **90** | **6** | **0** | **48** | **2.73%** |
| 10 | 1381 | 1283 | 91 | 7 | 0 | 77 | 4.38% |
| 12 | 1335 | 1225 | 102 | 8 | 0 | 89 | 5.07% |
| 14 | 1264 | 1139 | 116 | 9 | 0 | 58 | 3.30% |

(Shares are the report's micros -- 20501, 26765, 27334, 43849, 50683,
33029 -- as percentages of 1756 target rows. The largest cluster at tau 14
being smaller than at tau 12 is a property of first-member representative
linkage: a wider tau changes which row founds which cluster.)

## Write-mode pass

Run 2026-09-22 18:08-18:15Z on build `ea5f75fe`, after a database backup
tagged `pre-dedup-v2-pass`. Completion line:

```
rows=1802 derived=1756 written=1758 failed=46 write_failed=0
```

`written` exceeds `derived` by two because two of the 46 non-derived rows
had their cluster columns changed by the sweep (they keep their v1 stamp
and value). Ingest was healthy throughout: 0 errors, no restart.

`scripts/operator/dedup-cluster-report.sh` before and after:

| statistic | before (v1) | after (v2) |
|---|---|---|
| rows with a cluster | 696 | 1758 |
| clusters | 67 | 1427 |
| singletons | 48 | 1329 |
| clusters of 2-9 | 16 | 92 |
| clusters of 10-99 | 2 | 6 |
| clusters of 100+ | 1 | 0 |
| largest cluster | 522 (75%) | 48 (2.7%) |
| rows with a simhash / distinct / zero | -- | 1758 / 1587 / 0 |
| stamp: NULL read as legacy v1 | 1484 | 45 |
| stamp: explicit `events.v1+fnv1a-2shingle.v1` | 318 | 1 |
| stamp: `events.v1+fnv1a-3shingle-set.v2` | 0 | 1756 |
| member-to-earliest Hamming: median / p95 / max | 6.0 / 10 / 10 | 0.0 / 6 / 8 |
| largest cluster: median nearest-member Hamming | 0.0 | 1.0 |

Before the pass only 696 of the 1802 rows carried a `dedup_cluster_id`;
why the other 1106 v1 rows had none was not established here. The pass
assigns every row it sweeps, so "rows with a cluster" now equals "rows
with a simhash". The before-stamp figures (1484 + 318 = 1802) and the
after-stamp figures (1756 + 45 + 1 = 1802) both account for every row.

The after "member-to-earliest" p95 of 6 and max of 8 are at or under tau
8, as the sweep guarantees; the largest cluster's median nearest-member
distance of 1.0 (was 0.0 under v1) says its members are near-identical
texts rather than the harness fingerprint.

## What the shape says, and the tau decision

The spec's expected shape was a large mass at 20+ and a small mass at 0-6
with little between. What was measured:

- **Unrelated traces separate.** 1215 rows (69%) sit at 15-24, and the
  mega-cluster is gone at every candidate tau (largest 36 to 89, never
  above 5.1%). Under v1 every long trace from the harness was within 10 of
  the first one.
- **Known positives are visible.** 109 rows at Hamming 0 and 18 at 1-2 are
  the resubmission / near-identical class.
- **There is no clean valley.** 405 rows (23%) sit in 3-14, spread almost
  flat (59, 72, 73, 55, 49, 97). The runbook and the spec both say that a
  populated 8-14 band means the signal needs escalating -- author-kind
  weighting of tool-result shingles first, MinHash second -- and that the
  threshold should not be moved to accommodate it. The per-tau table
  agrees: no tau between 4 and 14 produces a step change, only a slow
  drift in cluster count (1506 to 1264).

The maintainer's decision was to run the write-mode pass at the spec's
default tau 8 and treat escalation as the next iteration, for two reasons:
v2 at tau 8 is strictly better than v1 on every statistic above, and
`dedup_cluster_size` feeds no contributor-visible or ledger number today
(only the shadow contributor-cap columns nothing reads), so a cluster
boundary in the 3-14 band that is wrong in either direction costs nothing
anyone sees. The rows in that band are the same-project / overlapping-file-
read case the spec's Jaccard table marks "should not cluster" at J 0.2-0.6,
which is exactly the case author-kind weighting is designed to move.

## Not established / open

- **The 2 unidentified failures.** 44 of the second dry run's 46 failures
  are explained by there being no active envelope of either kind; the
  other 2 were not diagnosed. They keep their v1 stamp and value and are
  refused against every v2 row until a rerun succeeds on them.
- **The 3-14 band and the escalation path.** Which of the 405 rows are
  same-project sessions, late forks, or something else has not been
  measured; the spec's Jaccard table is a model, not a label. Author-kind
  weighting (spec section "Candidate signals", first escalation) has not
  been implemented. Its dry run would decide whether MinHash is needed.
- **Same-project separation: min Hamming 10 over 50 seeds against tau 8.**
  The #978 unit test
  `v2_long_trace_same_project_different_session_is_not_within_tau`
  (`dedup_simhash.rs`) measures its synthetic same-project pairs at mean
  19.6, min 10, max 26 over 50 seeds and asserts only that the minimum
  stays above tau 8. Two bits of margin on a synthetic generator is not a
  guarantee on the corpus, and the pilot's 3-14 band shows real pairs
  closer than the synthetic minimum. The test bounds the generator, not
  the corpus.
- **No planted control.** The runbook's step 2 (resubmit one already-scored
  session from the test tenant and confirm the pair shares a cluster after
  the write) was deliberately skipped: the dry run's 109 rows at Hamming 0
  already show the known-positive class, and planting needs a test-tenant
  submission. So there is no single pair whose cluster membership was
  checked end to end.
- **The embedding arm's threshold.** `tau_e_micros = 30_000` is set from
  the novelty distribution of unrelated pairs, not from a measurement of
  what near-identical text scores on bge-large-en-v1.5. The pass never
  embeds. It is confirmed or moved by the arm's own dry run when it is
  wired.
- **The `written` overshoot** (1758 vs 1756) is explained above from the
  code path, not from a per-row log; which two v1-stamped rows changed
  cluster columns was not looked up.

## Post-flip (runbook steps 10-12)

- Flip build installed: `dd0a581f`; `/health` reported
  `"build_commit":"dd0a581f"`, 12 of 12 healthy samples, no restart, no
  migration applied at boot (max recorded version 74).
- The flip build went live at 2026-09-22 19:19:16Z, closing the mixed-stamp
  window that opened with the write-mode pass at 18:15Z.
- **Step 10, write mode once more** (19:20Z, build `dd0a581f`):
  `rows=1803 derived=1 reused=1756 not_derivable=0 failed=46 written=1
  unchanged=1758 write_failed=0`. One trace was scored inside the window and
  `derived=1` is that row; every stored v2 value was reused.
- **Step 11, `POST /v1/admin/recompute-contributor-caps`** (19:2xZ):
  `contributor-cap recompute pass completed updated=1803 failed=0`, no
  skipped-decision warnings.
- **Step 12, final `dedup-cluster-report.sh`** (19:20:44Z, as
  `tc_gate_driver_login`): rows with a cluster 1759, clusters 1428,
  singletons 1330, 2-9: 92, 10-99: 6, 100+: 0, largest 48 (2.7%); stamps v2
  1757 / legacy (NULL) 45 / explicit v1 1; member-to-earliest Hamming
  median 0.0 / p95 6 / max 8; largest-cluster median nearest-member 1.0.
  Identical to the post-pass report apart from the one window row.
- The 46 non-derived rows are unchanged (45 legacy + 1 explicit v1 after the
  window row moved onto v2); the 2 unidentified failures remain unidentified.
- Ingest was healthy throughout every step; load stayed below 0.4.
