# NEAR Benchmark Corpus Handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a raw-envelope export dataset kind that delivers full redacted trace envelopes, plus an operator script that packages an export run into a JSONL corpus and a provenance manifest.

**Architecture:** A new `TraceExportDatasetKind::RawEnvelopeCorpus` served by `POST /v1/workers/raw-envelope-export`, built by copying the existing replay-export handler and changing only the emitted item type so it retains the envelope instead of extracting a metadata subset. Every guardrail, grant, job, manifest, audit, and eligibility check is reused unchanged. Replay export is not modified, and a test pins that it still emits no envelope. A Python operator script then converts the response into `corpus.jsonl` + `handoff-manifest.json`.

**Tech Stack:** Rust (axum, serde) for the server; Python 3 for the operator script, matching the existing `scripts/operator/*.py` convention.

**Spec:** `docs/superpowers/specs/2026-08-13-near-benchmark-export-design.md`

## Global Constraints

- PostgreSQL only. A single `cargo check -p trace-commons-server` is sufficient; do not add libsql feature flags or dual-backend testing.
- Verify with `RUSTFLAGS='-D warnings'` — plain `cargo check` does not apply what CI applies:
  - `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
  - `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
- Clippy is CI-enforced. Run: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen the allow-list.
- Run `cargo fmt --all` before committing. After every commit run `git show --stat HEAD` and confirm only intended files changed — the repo is not rustfmt-clean, so an editor hook can turn a one-line edit into a whole-file diff.
- No emojis in commits, PRs, code, or comments. Short imperative commit subjects, no `feat:` / `fix:` prefixes.
- Hash-only audit and logging. Never put raw URLs, tokens, contributor identity, trace bodies, or operator-secret material into audit rows or log strings.
- Do not add dependencies. The Python script uses only the standard library.
- `trace-commons-ingest.rs` is ~61k LOC; its tests live in `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` via `#[path]`. Add tests there, never inline them back.
- Do not split existing files.

---

### Task 1: Add the `RawEnvelopeCorpus` dataset kind

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:1852-1877` (enum + `label` + `storage_name`)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:39491-39502` (`trace_export_dataset_kind_from_storage_name`)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `TraceExportDatasetKind::RawEnvelopeCorpus`, with `storage_name()` returning `"raw_envelope_corpus"` and `label()` returning `"raw envelope corpus"`. Later tasks match on this variant.

- [ ] **Step 1: Write the failing test**

Add to `tests.rs`:

```rust
#[test]
fn raw_envelope_corpus_dataset_kind_round_trips_through_storage_name() {
    let kind = TraceExportDatasetKind::RawEnvelopeCorpus;
    assert_eq!(kind.storage_name(), "raw_envelope_corpus");
    assert_eq!(
        trace_export_dataset_kind_from_storage_name("raw_envelope_corpus").unwrap(),
        TraceExportDatasetKind::RawEnvelopeCorpus
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest raw_envelope_corpus_dataset_kind_round_trips -- --nocapture`

Expected: FAIL to compile — no variant named `RawEnvelopeCorpus`.

- [ ] **Step 3: Add the variant**

In the enum at line 1852, add `RawEnvelopeCorpus,` as the last variant. Then add the two match arms:

```rust
// in label()
Self::RawEnvelopeCorpus => "raw envelope corpus",
// in storage_name()
Self::RawEnvelopeCorpus => "raw_envelope_corpus",
```

In `trace_export_dataset_kind_from_storage_name`, add before the `_ =>` arm:

```rust
"raw_envelope_corpus" => Ok(TraceExportDatasetKind::RawEnvelopeCorpus),
```

The compiler will flag any other non-exhaustive match on this enum. Fix each by treating the new kind the same as `ReplayDataset`, **except** in `trace_export_dataset_kind_creates_positive_credit` (line 39504), where it must behave as `ReplayDataset` does — read that function and match its existing treatment rather than guessing.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest raw_envelope_corpus_dataset_kind_round_trips`

Expected: PASS.

- [ ] **Step 5: Verify the whole crate still builds warning-free**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`

Expected: clean. A non-exhaustive-match error here means a match arm was missed in Step 3.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Add a raw envelope corpus export dataset kind"
git show --stat HEAD
```

---

### Task 2: Add the raw-envelope item and export types

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (beside `TraceReplayDatasetItem` at 66127 and `TraceReplayDatasetExport` at 65722)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `TraceExportDatasetKind::RawEnvelopeCorpus` from Task 1.
- Produces:
  - `TraceRawEnvelopeDatasetItem { submission_id: Uuid, trace_id: Uuid, privacy_risk: ResidualPiiRisk, redaction_counts: serde_json::Value, envelope: TraceContributionEnvelope }` with `fn from_record(record: &TraceCommonsSubmissionRecord, envelope: &TraceContributionEnvelope) -> Self`.
  - `TraceRawEnvelopeDatasetExport { tenant_id: String, tenant_storage_ref: String, export_id: Uuid, audit_event_id: Uuid, created_at: DateTime<Utc>, item_count: usize, manifest: TraceReplayExportManifest, items: Vec<TraceRawEnvelopeDatasetItem> }`.

Task 3 constructs both.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn raw_envelope_item_retains_the_envelope_events() {
    let record = sample_submission_record();
    let envelope = sample_contribution_envelope();
    let item = TraceRawEnvelopeDatasetItem::from_record(&record, &envelope);

    assert_eq!(item.submission_id, record.submission_id);
    assert_eq!(item.privacy_risk, record.privacy_risk);
    assert_eq!(item.envelope.events.len(), envelope.events.len());

    let json = serde_json::to_value(&item).unwrap();
    assert!(
        json.get("envelope").and_then(|e| e.get("events")).is_some(),
        "serialized raw envelope item must carry envelope.events"
    );
}
```

`sample_submission_record` and `sample_contribution_envelope` are existing test helpers. Find their real names first with:
`grep -n "fn sample_submission_record\|fn sample_contribution_envelope\|TraceContributionEnvelope {" crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs | head`
Use whatever the file already provides rather than adding new builders.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest raw_envelope_item_retains`

Expected: FAIL to compile — type does not exist.

- [ ] **Step 3: Add the types**

Place directly after `TraceReplayDatasetItem`'s `impl` block (ends line ~66180):

```rust
#[derive(Debug, Clone, Serialize)]
struct TraceRawEnvelopeDatasetItem {
    submission_id: Uuid,
    trace_id: Uuid,
    privacy_risk: ResidualPiiRisk,
    redaction_counts: serde_json::Value,
    envelope: TraceContributionEnvelope,
}

impl TraceRawEnvelopeDatasetItem {
    fn from_record(
        record: &TraceCommonsSubmissionRecord,
        envelope: &TraceContributionEnvelope,
    ) -> Self {
        Self {
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            privacy_risk: record.privacy_risk,
            redaction_counts: record.redaction_counts.clone(),
            envelope: envelope.clone(),
        }
    }
}
```

And beside `TraceReplayDatasetExport`:

```rust
#[derive(Debug, Serialize)]
struct TraceRawEnvelopeDatasetExport {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    audit_event_id: Uuid,
    created_at: DateTime<Utc>,
    item_count: usize,
    manifest: TraceReplayExportManifest,
    items: Vec<TraceRawEnvelopeDatasetItem>,
}
```

If `record.redaction_counts` is not a `serde_json::Value`, match the field's real type instead of converting — check `TraceCommonsSubmissionRecord`'s definition and mirror it.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest raw_envelope_item_retains`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Add raw envelope dataset item and export types"
git show --stat HEAD
```

---

### Task 3: Add the raw-envelope export handler and route

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (new handlers beside `run_worker_replay_export` at 37762; route registration at ~7100)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: Task 1's variant, Task 2's types.
- Produces: `POST /v1/workers/raw-envelope-export` returning `TraceRawEnvelopeDatasetExport`.

**Method:** copy `run_worker_replay_export` (37762), `run_dataset_replay_export_with_grant` (37785), `prepare_replay_export_execution` (37831), and `run_dataset_replay_export_job` (37858) to `*_raw_envelope_*` equivalents. Change exactly three things:
1. `TraceExportDatasetKind::ReplayDataset` → `TraceExportDatasetKind::RawEnvelopeCorpus`.
2. The default purpose string → `"trace_commons_raw_envelope_corpus"`.
3. The item push — replace `TraceReplayDatasetItem::from_record(&record, derived..., &body_read.envelope, body_read.object_ref_id)` with `TraceRawEnvelopeDatasetItem::from_record(&record, &body_read.envelope)`.

Keep `TraceAllowedUse::Evaluation`, `enforce_dataset_export_guardrails`, `is_export_eligible()`, `ensure_retention_metadata_within_server_policy`, the `source_submission_ids_hash` call, and the audit event exactly as they are. The `derived_by_submission` map is no longer needed; drop it and the `derived` binding if the compiler reports them unused.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn raw_envelope_export_rejects_request_missing_privacy_risk() {
    // Mirror the existing replay-export guardrail test: guardrails enabled,
    // purpose + status + consent_scope supplied, privacy_risk omitted.
    // Expect StatusCode::BAD_REQUEST and a message containing
    // "requires privacy_risk=low".
}

#[tokio::test]
async fn raw_envelope_export_emits_full_envelopes_for_accepted_low_records() {
    // One accepted/low record with a non-empty events vector.
    // Expect item_count == 1 and items[0].envelope.events non-empty.
}

#[tokio::test]
async fn raw_envelope_export_excludes_medium_privacy_risk_records() {
    // One accepted/low and one accepted/medium record.
    // Request privacy_risk=low. Expect item_count == 1.
}
```

Before writing these, find the existing replay-export route tests and copy their setup verbatim:
`grep -n "replay_export" crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs | head -30`
Reuse that harness — do not invent a new server-test fixture.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest raw_envelope_export`

Expected: FAIL — route not registered / handler missing.

- [ ] **Step 3: Add the handlers and register the route**

Add the four copied functions. Then register beside the replay route at ~7100:

```rust
.route(
    "/v1/workers/raw-envelope-export",
    post(worker_raw_envelope_export_handler),
)
```

Follow the auth shape of `run_worker_replay_export` exactly: `authenticate_with_tenant_access_grant` then `require_export_worker_operator`. Do not invent a new credential or reuse a different scoped token.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest raw_envelope_export`

Expected: PASS, all three.

- [ ] **Step 5: Full verification**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```

Expected: all clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Serve a raw envelope corpus export from a worker route"
git show --stat HEAD
```

---

### Task 4: Pin the replay-export boundary

**Files:**
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: existing replay export. Produces: nothing.

This is a regression guard. The two paths now differ only in their item type, and nothing else prevents a later change from adding the envelope to the replay item and silently widening what `evaluation` delivers on the older route.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn replay_dataset_item_does_not_serialize_the_envelope() {
    let record = sample_submission_record();
    let envelope = sample_contribution_envelope();
    let item = TraceReplayDatasetItem::from_record(&record, None, &envelope, None);
    let json = serde_json::to_value(&item).unwrap();

    assert!(
        json.get("envelope").is_none(),
        "replay export must not carry trace bodies; use the raw envelope corpus kind"
    );
    assert!(json.get("events").is_none());
}
```

Adjust the helper names to whatever Task 2 Step 1 established.

- [ ] **Step 2: Run it and confirm it passes immediately**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest replay_dataset_item_does_not_serialize`

Expected: PASS on first run. This test guards existing behavior, so a failure here means Task 3 accidentally modified replay export — revert that change rather than editing this test.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Pin that replay export does not emit trace bodies"
git show --stat HEAD
```

---

### Task 5: Build the handoff converter script

**Files:**
- Create: `scripts/operator/near-benchmark-handoff.py`
- Create: `scripts/operator/test_near_benchmark_handoff.py`
- Create: `scripts/operator/fixtures/near-benchmark-handoff/raw-envelope-export.json`

**Interfaces:**
- Consumes: a `TraceRawEnvelopeDatasetExport` JSON document from Task 3.
- Produces: `corpus.jsonl` and `handoff-manifest.json`.

Follow `scripts/operator/analyze-gate-outcome.py` for style: `#!/usr/bin/env python3`, a module docstring documenting the expected input schema, `from __future__ import annotations`, `argparse`, stdlib only. Tests follow `scripts/operator/test_analyze_gate_outcome.py`: runnable under pytest *or* as a standalone script exiting non-zero on failure.

**Fixture rule:** the fixture must be derived from the producer's type definitions in Task 2, not from this script's own output. A fixture and its consumer authored together agree with each other whether or not either is correct. Hand-write it from the Rust struct fields, with two items — one small, one whose envelope contains an event payload of at least 200KB to exercise line-oriented output.

- [ ] **Step 1: Write the fixture**

Two items matching `TraceRawEnvelopeDatasetExport`, with top-level `tenant_id`, `tenant_storage_ref`, `export_id`, `audit_event_id`, `created_at`, `item_count: 2`, `manifest` (containing `source_submission_ids_hash`, `purpose`, `consent_scopes`), and `items[]` each with `submission_id`, `trace_id`, `privacy_risk: "low"`, `redaction_counts`, and `envelope` containing at minimum `schema_version`, `trace_id`, `submission_id`, `events`.

Generate the large payload rather than pasting 200KB:

```python
python3 - <<'EOF'
import json, pathlib
p = pathlib.Path("scripts/operator/fixtures/near-benchmark-handoff/raw-envelope-export.json")
doc = json.loads(p.read_text())
doc["items"][1]["envelope"]["events"][0]["content"] = "x" * 220_000
p.write_text(json.dumps(doc, indent=2) + "\n")
EOF
```

- [ ] **Step 2: Write the failing tests**

```python
def test_writes_one_jsonl_line_per_item(tmpdir):
    corpus, manifest = run_converter(FIXTURE, tmpdir)
    lines = corpus.read_text().splitlines()
    assert len(lines) == 2
    assert len(lines) == manifest["item_count"]

def test_every_jsonl_line_is_a_complete_envelope(tmpdir):
    corpus, _ = run_converter(FIXTURE, tmpdir)
    for line in corpus.read_text().splitlines():
        envelope = json.loads(line)
        assert "events" in envelope
        assert "submission_id" in envelope

def test_manifest_carries_provenance_unmodified(tmpdir):
    _, manifest = run_converter(FIXTURE, tmpdir)
    source = json.loads(FIXTURE.read_text())
    assert manifest["export_id"] == source["export_id"]
    assert (manifest["source_submission_ids_hash"]
            == source["manifest"]["source_submission_ids_hash"])

def test_manifest_sha256_matches_corpus_bytes(tmpdir):
    corpus, manifest = run_converter(FIXTURE, tmpdir)
    digest = hashlib.sha256(corpus.read_bytes()).hexdigest()
    assert manifest["corpus_sha256"] == digest

def test_large_envelope_survives_round_trip(tmpdir):
    corpus, _ = run_converter(FIXTURE, tmpdir)
    big = [json.loads(l) for l in corpus.read_text().splitlines()][1]
    assert len(big["events"][0]["content"]) >= 200_000

def test_refuses_non_low_privacy_risk_item(tmpdir):
    # Copy the fixture, set items[0].privacy_risk = "medium", expect
    # a non-zero exit and a message naming the offending submission_id.
```

`run_converter` invokes the script via `subprocess` with `--input`, `--output-dir`, and returns the two artifact paths plus the parsed manifest.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `python3 scripts/operator/test_near_benchmark_handoff.py`

Expected: FAIL — script does not exist.

- [ ] **Step 4: Implement the converter**

```python
def convert(source: dict, output_dir: Path) -> dict:
    corpus_path = output_dir / "corpus.jsonl"
    entries = []
    with corpus_path.open("w", encoding="utf-8") as handle:
        for item in source["items"]:
            risk = item["privacy_risk"]
            if risk != "low":
                raise ConversionError(
                    f"submission {item['submission_id']} has privacy_risk={risk}; "
                    "only low-risk records may be handed off"
                )
            handle.write(json.dumps(item["envelope"], separators=(",", ":")) + "\n")
            entries.append({
                "submission_id": item["submission_id"],
                "privacy_risk": risk,
                "redaction_counts": item.get("redaction_counts", {}),
            })

    digest = hashlib.sha256(corpus_path.read_bytes()).hexdigest()
    manifest = {
        "export_id": source["export_id"],
        "audit_event_id": source["audit_event_id"],
        "source_submission_ids_hash": source["manifest"]["source_submission_ids_hash"],
        "item_count": len(entries),
        "consent_basis": source["manifest"].get("consent_scopes", []),
        "corpus_sha256": digest,
        "items": entries,
    }
    (output_dir / "handoff-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest
```

The `privacy_risk != "low"` refusal is defence in depth: the server guardrail already excludes those records, and this check ensures a hand-edited or mis-parameterised export cannot be packaged for handoff.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `python3 scripts/operator/test_near_benchmark_handoff.py`

Expected: all pass, exit 0.

- [ ] **Step 6: Commit**

```bash
git add scripts/operator/near-benchmark-handoff.py \
        scripts/operator/test_near_benchmark_handoff.py \
        scripts/operator/fixtures/near-benchmark-handoff/
git commit -m "Add the NEAR benchmark handoff packaging script"
git show --stat HEAD
```

---

### Task 6: Write the operator runbook

**Files:**
- Create: `docs/operator/near-benchmark-handoff.md`
- Modify: `docs/operator/README.md` (runbook index)

**Interfaces:**
- Consumes: everything above. Produces: nothing consumed by code.

- [ ] **Step 1: Write the runbook**

It must contain, in order:

1. **Pre-run verification.** The base64-over-SSH query pattern, since the pilot host is reached with `gcloud compute ssh tc-pilot-host --zone us-central1-a --project tracecommons-pilot-2026 --tunnel-through-iap`. Every query leads with `SELECT trace_current_tenant_id();` and the GUC is `trace_commons.trace_tenant_id` — note the doubled `trace_`. A zero-row result is an unproven read, not an empty corpus. Record the eligible count and the object-ref coverage count; they must be equal.
2. **The export call.** `POST /v1/workers/raw-envelope-export` with `purpose=near_benchmark_handoff`, `status=accepted`, `privacy_risk=low`, `consent_scope=debugging_evaluation`. All four mandatory; omitting any returns 400.
3. **The stop condition.** If `item_count` is below the pre-run eligible count, stop and investigate. Do not package a short export.
4. **Packaging.** `python3 scripts/operator/near-benchmark-handoff.py --input <export.json> --output-dir <dir>`.
5. **Delivery.** Per the spec's Deliver section: dedicated bucket, `near-benchmark-handoff/{export_id}/`, IAM grant over signed URL, report `corpus_sha256` out of band, delete after the fetch is confirmed in audit logs, versioning off.
6. **Failure recovery.** Use `/v1/admin/export/jobs/{id}/retry`, not a fresh grant, so one handoff maps to one grant lineage.

- [ ] **Step 2: Add it to the runbook index**

Add a line to `docs/operator/README.md` matching the existing entry format.

- [ ] **Step 3: Commit**

```bash
git add docs/operator/
git commit -m "Add the NEAR benchmark handoff runbook"
git show --stat HEAD
```

---

## Out of scope for this plan

- Running the live export against the pilot. That creates grant, job, and audit rows on production and requires explicit operator authorization.
- Any delivery to NEAR.
- Resolving the spec's open contributor-attribution question. That is a decision, not an implementation step, and it gates delivery rather than this build.
