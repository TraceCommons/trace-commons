# Persist the residual-risk basis

Design for #474, proposal 4 only. The quarantine queue reached 221 traces on
the pilot; after removing the 112 that were `pii_backstop_attempts_exhausted`
-- a processing failure, never assessed -- 108 remained. Of those, only 18
carry `tool_sensitive_field`. **The driver for the remaining ~90 is not
established**, and it cannot be established from stored data, because the
input to the risk decision is never written down.

The leading hypothesis is `coverage_incomplete`: a classifier that errored or
could not finish leaves content unexamined, `residual_risk` fails closed to
High, and the resulting row is indistinguishable at review time from a trace
where the filter looked and found a secret. Those two things demand opposite
responses. One is a privacy finding. The other is an outage.

This design records which conditions actually held, so the hypothesis can be
tested. It is instrumentation. It changes no threshold and quarantines no
different set of traces.

## Scope, and what is deliberately excluded

#474 makes four proposals. This design implements **only proposal 4**.

Proposals 1-3 -- separating heuristic from deterministic secret findings,
stopping `local_path` volume from elevating risk, and reconsidering permanent
High from object-key findings -- are calibration changes. The issue's own
sequencing forbids making them now:

> Do not calibrate against today's data. Classifier failures were elevated all
> day and plausibly inflated the High rate through `coverage_incomplete`.
> [...] Calibrating against an outage would bake the outage into the
> thresholds.

That instruction is the reason this slice exists. Persisting the basis is the
prerequisite that makes the later re-measurement possible.

This slice therefore produces no answer by itself. It makes the ~90
unexplained quarantines measurable. Someone must re-run the measurement after
the token-budgeting fix has run for a day.

## Why the information is lost today

`residual_risk` short-circuits. It returns on the first condition that
matches:

```rust
if report.key_finding_detected {
    return ResidualPiiRisk::High;
}
if report.coverage_incomplete {
    return ResidualPiiRisk::High;
}
```

A trace with both a key finding and a coverage gap returns High on the key
finding, and the coverage gap is never evaluated, let alone recorded. Only
`ResidualPiiRisk::High` survives to storage.

This is correct for classification -- the risk value is the same either way,
and evaluating further would be wasted work -- and it is exactly wrong for
measurement. A count derived from a first-wins label would report
`coverage_incomplete` as a lower bound, undercounting it by precisely the
population where a real finding co-occurs. Since the question being asked is
"how much of this queue is an outage artifact rather than a privacy finding",
a systematic undercount of the outage side is the one error that cannot be
tolerated.

## 1. A non-short-circuiting basis, in the protocol crate

`residual_risk` is not modified. It remains the sole authority on the risk
value.

Alongside it, an observational function evaluates every condition
independently and returns all that hold:

```rust
pub enum ResidualRiskCondition {
    KeyFinding,
    CoverageIncomplete,
    ResidualSurvivor,
    ResidualScanUnavailable,
    FoundAndRemoved,
    ConsentContentFlag,
}

pub fn residual_risk_basis(
    consent: &ConsentMetadata,
    report: &RedactionReport,
    residual_findings: Option<&RedactionReport>,
) -> Vec<ResidualRiskCondition>
```

`residual_findings` is `None` on the client pass, which runs no residual scan,
and `Some` on the server path, where `PostScrubAssessment` carries it. `None`
means "no residual scan was run", which is not the same as "the residual scan
was clean" and must never be recorded as `ResidualSurvivor`'s absence being
evidence of anything.

### Two entry points, because one signature cannot say three things

That signature alone is not sufficient, and an earlier revision of this
document was wrong to imply it was. It distinguishes only two states -- a scan
ran and produced findings, or no scan ran -- while the design requires three.
A scan that was *attempted and failed* is a third fact, and it is the one
`ResidualScanUnavailable` exists to record. There is only one way to spell
`None`, so the failed case is inexpressible in that signature.

The resolution is a second entry point rather than a wider first one:

```rust
pub fn residual_risk_basis_for_failed_scan(
    consent: &ConsentMetadata,
    report: &RedactionReport,
) -> Vec<ResidualRiskCondition>;
```

It delegates to `residual_risk_basis(consent, report, None)` and appends
`ResidualScanUnavailable`. Both `Err(_)` arms -- in
`rescrub_trace_envelope_with` and `rescrub_envelope_prose_pii_with` -- call it.
Routing both through one named function is the point: the condition is never
constructed ad hoc at a call site, so a third failure arm added later cannot
quietly omit it.

### `ResidualSurvivor` is scoped to `blocked_secret_detected`

`ResidualSurvivor` is set from `residual.blocked_secret_detected` alone, not
from any non-empty residual report.

This follows from the consistency property. `blocked_secret_detected` is the
only residual flag that *forces* High in `resolve_post_scrub_risk`; residual
counts on their own merely block a downgrade, leaving the row at Medium.
Scoping the condition wider would therefore put a forces-High label on a
Medium row and break the very property section 1 requires.

Residual `key_finding_detected` and `coverage_incomplete` are not lost by this
narrowing -- they map to `KeyFinding` and `CoverageIncomplete`, which are
evaluated against both the pass's own report and the residual report.

`ResidualScanUnavailable` is a distinct condition, not a flavour of
`CoverageIncomplete`. Both `rescrub_trace_envelope_with` and
`rescrub_envelope_prose_pii_with` force High in an `Err(_)` arm when
`residual_envelope_scan` could not run at all, and that arm never constructs a
`PostScrubAssessment`, so no flag on `RedactionReport` records it. It is
invisible to any basis derived only from the report -- and it is an outage
signature, exactly the kind #474 is trying to separate from real findings.

The first four force High. The last two are the Medium floor, recorded
because a calibration pass needs the denominator as much as the numerator:
"how many Mediums were driven only by a consent flag" is the same shape of
question as the one motivating this work.

`ResidualSurvivor` covers the residual-scan findings that reach the decision
through `resolve_post_scrub_risk`'s `residual_findings`, which `residual_risk`
alone never sees. Both call sites are covered -- see section 3.

### The drift hazard

The obvious way for this design to fail is for the basis to become a second,
independently-maintained implementation of the risk rule, and to disagree with
it. A basis that says `CoverageIncomplete` on a row stored Medium is worse
than no basis, because it will be believed.

The guard is a consistency test, not a comment: across the combinations of
inputs, if the basis holds any High condition then `residual_risk` must return
High, and if the basis is empty then `residual_risk` must return Low. The two
functions are then pinned to each other, and a future edit to one that does
not touch the other fails the suite.

## 2. Storage: migration V51

```sql
ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS residual_risk_basis JSONB;
```

Following V49's precedent, which this design copies deliberately:

- **Nullable, no backfill.** NULL reads as "not recorded", never as a claim
  about why a trace is where it is. Backfilling by inferring from `status`
  would fabricate exactly the data this work exists to obtain.
- **Label-only by construction.** Values come from a fixed `&'static str`
  allowlist derived from `ResidualRiskCondition`, never from caller-supplied
  text. This mirrors the `safe_status_reason_label` choke point and keeps the
  repo's hash-only/label-only rule intact.
- **No new column grants.** Neither `trace_gate_driver` nor
  `trace_pii_backstop_driver` reads this column, and both hold column-scoped
  grants (V45). Adding one would widen a reader role for nothing.
- **No RLS change.** A column on an already-RLS-forced table inherits the
  tenant predicate.

JSONB rather than `TEXT[]` matches the table's existing convention --
`consent_scopes`, `allowed_uses` and `redaction_counts` are all JSONB -- and
reuses the `json_array_strings` reader in `trace_corpus_pg.rs`.

## 3. Write paths: both, and server-computed

The server resolves `residual_pii_risk` in **three** places, not two. All
three must write the basis:

| Site | Function | Pass |
|---|---|---|
| `trace-commons-ingest.rs:12650` | `submit_trace_handler` | `rescrub_trace_envelope` |
| `trace-commons-ingest.rs:19199` | `operator_rescrub_quarantined_submission` | `rescrub_trace_envelope` |
| `trace-commons-ingest.rs:40366` | `process_one_pii_backstop` | `rescrub_envelope_prose_pii_with` |

The third is the PII-backstop driver -- the one that releases held traces and
produces the population #474 is about. It goes through the async NEAR AI prose
path, **not** `rescrub_trace_envelope`. An implementation that instruments only
`rescrub_trace_envelope` would miss precisely the traces the issue is asking
about while appearing to work.

The second is an admin route rather than the driver. It is included because it
overwrites `record.privacy_risk` too, and a stale basis beside a fresh risk is
the failure this design exists to prevent.

Writing at ingest alone is not a smaller version of this change; it is a
broken one. The re-scrub would overwrite the risk and leave a stale basis
beside it, and a basis that disagrees with the risk on its own row is worse
than an absent one. The two fields are written together or not at all.

The backstop path is also the population #474 is actually about. The coverage
hypothesis concerns classifier errors, which is precisely what that path hits.

### Trust

The basis is computed from the server's own redaction pass. It is never read
from the envelope.

**Enforce this in the type system, not by convention.** The basis must be a
*return value* of each pass, never a field on `PrivacyMetadata`:

```rust
pub fn rescrub_trace_envelope(envelope: &mut TraceContributionEnvelope)
    -> Result<Vec<ResidualRiskCondition>, PrivacyFilterConfigError>;

pub fn rescrub_trace_envelope_with(
    redactor: &DeterministicTraceRedactor,
    envelope: &mut TraceContributionEnvelope,
) -> Vec<ResidualRiskCondition>;

pub async fn rescrub_envelope_prose_pii_with(
    adapter: &dyn PrivacyFilterAdapter,
    envelope: &mut TraceContributionEnvelope,
) -> Result<Vec<ResidualRiskCondition>, TraceContributionError>;
```

Putting the basis on the envelope would make it client-supplied by
construction, since the envelope is deserialised from contributor input.
Returning it makes that structurally impossible: there is no field for a
client to populate.

The client computes its own `residual_pii_risk` during its local pass, and
that value is attribution, in the same sense the envelope's tenant fields are.
Accepting a client-asserted basis would let a contributor describe the
conditions under which their own trace was judged -- and the gate-trust
finding from the 2026-07-29 private disclosure (section 3) is still open.

## 4. Review surface

`residual_risk_basis` is exposed on the admin and review read paths, beside
`last_status_reason`.

This is part of the slice rather than a follow-up because it is half the
issue's complaint. `coverage_incomplete` forcing High is correct fail-closed
behaviour; being unable to *tell* at review time is the defect. A reviewer
opening the queue should read "coverage_incomplete" rather than infer it.

The review-burden argument in #474 is the reason this matters beyond tidiness:
when the overwhelming majority of a queue is benign, bulk approval becomes the
rational response, which is how a real finding gets waved through.

## 5. Testing

TDD -- tests written first, then implementation.

1. **Both conditions recorded.** A report with `key_finding_detected` and
   `coverage_incomplete` yields a basis containing both. This is the test that
   fails against a first-wins implementation.
2. **Basis agrees with the risk.** The consistency property from section 1,
   across input combinations.
3. **Pre-migration rows read NULL**, and NULL is surfaced as "not recorded"
   rather than as an empty basis.
4. **Re-scrub writes both fields.** After a backstop release, the basis
   matches the newly-written `privacy_risk`.
5. **No unallowlisted label can be stored.** The choke point holds.

Note that CI never runs the PostgreSQL suite, so tests 3 and 4 gate nothing in
CI and must be run locally. The shared `trace_commons_test` database already
has migrations 30-34 applied; V51 is clear of that range.

**The pg suite reports `ok` when it skips.** Every test in
`trace_corpus_pg_store.rs` opens with

```rust
let Some(backend) = postgres_backend().await else {
    return;
};
```

so an unconfigured environment yields a green result that proves nothing. The
variable is `TRACE_COMMONS_PG_TEST_DATABASE_URL` (or `DATABASE_URL`) -- a near
miss silently skips. This was hit during verification of this very design: a
run against `TRACE_COMMONS_TEST_DATABASE_URL` reported
`1 passed` in 0.00s while the column did not exist in the target database.

A pg result is therefore only evidence when corroborated. Two cheap checks:
the run takes measurable time rather than 0.00s, and

```sql
SELECT column_name, data_type FROM information_schema.columns
WHERE table_name='trace_submissions' AND column_name='residual_risk_basis';
```

returns `residual_risk_basis | jsonb` against the database the test used.

## Known sharp edge: the upsert is COALESCE-free

The `DO UPDATE` writes `residual_risk_basis` unconditionally rather than
`COALESCE`-ing it with the stored value. This is deliberate, and it is the
right default: it guarantees the basis can never be stale relative to the
`privacy_risk` written beside it, which is the failure section 3 exists to
prevent.

The cost is that an upsert path which refreshes a submission *without* running
a scrub pass would null the column rather than preserve the prior basis. No
current path does this -- every one loads the file record, which carries the
basis forward -- but the callers were not exhaustively enumerated, and nothing
in the type system prevents a future one.

A null basis reads as "not recorded", which is honest rather than wrong, so
this degrades to lost data and never to a false claim. Worth knowing before
adding an upsert path.

## What this does not do

- It does not change which traces are quarantined.
- It does not answer why the ~90 are quarantined. It makes the question
  answerable.
- It does not touch the entropy, `local_path`, or key-finding weightings.
- It does not address #475. The backstop drain rate is a separate problem and
  this change does not affect throughput.
