# Pilot Bootstrap Replay Harness Implementation Plan (A.6)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new `trace-commons-pilot-bootstrap` binary that pulls HF agent-traces datasets, translates rows to `trace-commons-protocol` submission envelopes, and POSTs through the existing `/v1/submissions` ingest API at a configurable rate. Generates 30k+ realistic submissions for A2.5 floor calibration + Phase A.5 metric design + end-to-end pipeline validation.

**Architecture:** Single binary in `crates/trace-commons-server/src/bin/trace-commons-pilot-bootstrap.rs`. Reads HF datasets via JSONL session loading (parquet support was removed in PR #67 after the real swival/pi-mono/DeepSeek datasets turned out to ship JSONL only). Translates rows via per-dataset translators, sends submissions via `reqwest`. Labeling sidecar in local JSONL. Single-process, single-tenant; deterministic submission IDs for idempotency.

**Tech Stack:** Rust, `hf-hub`, `arrow` (verify in tree; if not, surface for approval), `reqwest` (already in tree), `serde_json`, `clap`.

**Spec:** `docs/superpowers/specs/2026-05-14-pilot-bootstrap-harness-design.md`

---

## File Map

**New files**

| Path | Responsibility |
|------|----------------|
| `crates/trace-commons-server/src/bin/trace-commons-pilot-bootstrap.rs` | The binary entry point — CLI parsing, top-level orchestration |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/mod.rs` | Module root re-exporting the submodules |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs` | HF dataset reader — parquet shard discovery, streaming row iteration |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/translators.rs` | `Translator` trait + concrete translators (`SwivalTranslator`, `PiMonoTranslator`, `DeepSeekAgentTranslator`) |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/submitter.rs` | Rate-limited HTTP submitter — bucket, retry, idempotency check |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/sidecar.rs` | Labeling sidecar — append-only JSONL writer |
| `crates/trace-commons-server/tests/pilot_bootstrap_translators.rs` | Per-translator unit tests with fixture rows |
| `docs/operator/pilot-bootstrap.md` | Operator runbook for running the harness |

**Modified files**

| Path | What changes |
|------|--------------|
| `crates/trace-commons-server/Cargo.toml` | Add `[[bin]]` entry for `trace-commons-pilot-bootstrap`. Verify `hf-hub`, `arrow`, `reqwest`, `serde_json`, `clap` are all already in tree; surface missing deps for approval before adding. |
| `docs/trace-commons-roadmap.md` | A.6 entry under Phase A status block |

**Out of scope (do not touch)**

- The gate-service binary (`trace-commons-ingest`) — harness submits *to* it; doesn't modify it
- `trace-commons-protocol` crate — harness *uses* its envelope types; doesn't extend them
- Auth path — uses existing tenant-token flow
- The vector index, embedder, or gate code — harness exercises them via the API

---

## Pre-flight

- [ ] **Confirm green baseline.**

```bash
cargo check -p trace-commons-server --bins
cargo test -p trace-commons-server
```

Expected: clean (modulo pre-existing warnings).

- [ ] **Read the spec.**

Especially the "What success looks like" section — it pins what the binary needs to actually do at the end.

- [ ] **Dependency check.**

```bash
grep -E "^hf-hub|^arrow|^reqwest|^serde_json|^clap" crates/trace-commons-server/Cargo.toml crates/trace-commons-gate-enclave/Cargo.toml
```

Confirm `reqwest`, `serde_json`, `clap` are already direct deps. `hf-hub` is in `trace-commons-gate-enclave` for the candle path — verify it's accessible from `trace-commons-server` (workspace dep or add a direct entry). `arrow` is likely *not* in tree — **surface for approval before adding**; alternative is `parquet` crate (also new) or shelling out to `huggingface-cli` to download + use `serde_json` parsing for jsonl-format datasets.

If `arrow` isn't approved, fall back: use `huggingface_hub` Python via `Command::new("python3")` for the dataset fetch + JSONL conversion. Slower, more ops surface, but no new Rust dep.

**Recommendation:** propose `arrow = "55"` (latest stable, MIT/Apache-2.0, used by every major Rust data project including DataFusion). Or `parquet = "55"` if the operator prefers a smaller-scope dep.

---

## Slice 1 — Binary scaffolding + CLI

### Task 1: Set up the binary + CLI shape

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace-commons-pilot-bootstrap.rs`
- Create: `crates/trace-commons-server/src/bin/pilot_bootstrap/mod.rs`
- Modify: `crates/trace-commons-server/Cargo.toml` (add `[[bin]]` entry)

CLI args:

```rust
#[derive(Parser)]
struct Cli {
    /// Source HF dataset id. Default: jedisct1/agent-traces-swival
    #[arg(long, default_value = "jedisct1/agent-traces-swival")]
    source: String,

    /// Per-dataset translator. Auto-detected from source if unset.
    #[arg(long)]
    translator: Option<String>,

    /// Total number of submissions. Default 1000.
    #[arg(long, default_value_t = 1000)]
    count: usize,

    /// Target ingest URL.
    #[arg(long, default_value = "http://localhost:3907")]
    target: String,

    /// Tenant bearer token (or env: TRACE_COMMONS_PILOT_TENANT_TOKEN).
    #[arg(long, env = "TRACE_COMMONS_PILOT_TENANT_TOKEN")]
    tenant_token: String,

    /// Rate limit (requests per second). Default 1.0.
    #[arg(long, default_value_t = 1.0)]
    rate: f64,

    /// Output sidecar JSONL path.
    #[arg(long, default_value = "./pilot-bootstrap-sidecar.jsonl")]
    sidecar: PathBuf,

    /// Deterministic seed for row sampling.
    #[arg(long, default_value_t = 42)]
    seed: u64,
}
```

- [ ] **Step 1: Write the binary stub + CLI parser.**

The stub prints the parsed args and exits — proves the binary structure compiles. No actual submission logic yet.

- [ ] **Step 2: Add the `[[bin]]` entry to `Cargo.toml`.**

```toml
[[bin]]
name = "trace-commons-pilot-bootstrap"
path = "src/bin/trace-commons-pilot-bootstrap.rs"
```

- [ ] **Step 3: Verify both default + `--features local-gpu-models` builds compile.**

```bash
cargo check -p trace-commons-server --bins
cargo check -p trace-commons-server --bins --features local-gpu-models
```

- [ ] **Step 4: Commit.**

```bash
git commit -m "Add trace-commons-pilot-bootstrap binary scaffolding"
```

---

## Slice 2 — HF dataset reader

### Task 2: `hf_dataset.rs` — discover + stream parquet shards

**Files:**
- Create: `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`

Behavior:

1. Use `hf-hub` to list files in the dataset repo.
2. Filter to parquet shards (the swival format).
3. Download shards on-demand (skip already-cached).
4. Stream rows via `arrow` or `parquet` (whichever was approved).
5. Yield `Row { fields: BTreeMap<String, serde_json::Value> }` per row.

Tests:
- `lists_parquet_shards_for_known_dataset` (integration test; runs only with network)
- `streams_rows_from_local_parquet_fixture` (unit test; ship a small parquet fixture file)

- [ ] **Step 1: Write the reader.**
- [ ] **Step 2: Tests.**
- [ ] **Step 3: Commit.**

```bash
git commit -m "Add HF parquet dataset reader for pilot bootstrap"
```

---

## Slice 3 — Translators

### Task 3: `Translator` trait + `SwivalTranslator`

**Files:**
- Create: `crates/trace-commons-server/src/bin/pilot_bootstrap/translators.rs`
- Create: `crates/trace-commons-server/tests/pilot_bootstrap_translators.rs`

Trait:

```rust
pub trait Translator {
    fn name(&self) -> &str;
    fn translate(&self, row: &Row) -> Result<SubmissionDraft>;
}

pub struct SubmissionDraft {
    pub submission_id: String,        // deterministic content hash
    pub trace_body: String,
    pub source_dataset: String,
    pub source_row_id: String,
    pub source_domain_tag: String,
}
```

`SwivalTranslator` impl:

```rust
fn translate(&self, row: &Row) -> Result<SubmissionDraft> {
    let title = row.get_str("title").unwrap_or_default();
    let severity = row.get_str("severity").unwrap_or_default();
    let finding_type = row.get_str("finding_type").unwrap_or_default();
    let proof = row.get_array_strs("proof").join("\n");
    let fix_outline = row.get_str("fix_outline").unwrap_or_default();
    let source_code = row.get_str("source_code").unwrap_or_default();

    let body = format!(
        "{title}\n\n{severity} {finding_type}\n\n{proof}\n\n{fix_outline}\n\n{}",
        &source_code[..source_code.len().min(2000)]
    );
    let id = sha256_hex(&body)[..32].to_string();
    Ok(SubmissionDraft {
        submission_id: id,
        trace_body: body,
        source_dataset: "jedisct1/agent-traces-swival".into(),
        source_row_id: row.get_str("title").unwrap_or_default().to_string(),
        source_domain_tag: format!("security-audit/{}", finding_type),
    })
}
```

Tests:
- `swival_translator_produces_deterministic_id_for_same_input`
- `swival_translator_handles_missing_fields_gracefully`
- `swival_translator_truncates_long_source_code`

`PiMonoTranslator` and `DeepSeekAgentTranslator` are stubs at this slice (return `Err("not implemented")`); filled in Slice 5.

- [ ] **Step 1: Trait + Swival translator + 3 tests.**
- [ ] **Step 2: Commit.**

```bash
git commit -m "Add Swival translator for pilot bootstrap (pi-mono + DeepSeek stubbed)"
```

---

## Slice 4 — Submitter + sidecar + main loop

### Task 4: Rate-limited submitter + sidecar writer + orchestration

**Files:**
- Create: `crates/trace-commons-server/src/bin/pilot_bootstrap/submitter.rs`
- Create: `crates/trace-commons-server/src/bin/pilot_bootstrap/sidecar.rs`
- Modify: `crates/trace-commons-server/src/bin/trace-commons-pilot-bootstrap.rs` (wire the main loop)

`Submitter`:
- `reqwest::Client` with `Authorization: Bearer <token>`
- Rate-limited via simple token-bucket or `tokio::time::sleep(1.0 / rate)`
- Idempotency: HEAD the submission by ID first; if 200, skip
- Error handling: retry on 5xx with exponential backoff (max 3 retries); log + skip on 4xx

`Sidecar`:
- Append-only JSONL writer
- One line per attempted submission: `{submission_id, source_dataset, source_row_id, source_domain_tag, http_status, gate_decision, elapsed_ms, timestamp}`

Main loop in the binary:
1. Parse CLI
2. Open dataset (Slice 2)
3. Pick translator (Slice 3)
4. Open submitter + sidecar
5. For each row (up to `--count`): translate → submit → write sidecar entry
6. Print summary on exit (total, accepted, rejected, errors)

- [ ] **Step 1: Submitter (rate-limit + retry + idempotency).**
- [ ] **Step 2: Sidecar (append-only JSONL).**
- [ ] **Step 3: Main loop wiring.**
- [ ] **Step 4: Integration test:** spin up a mock HTTP server (e.g. `wiremock` if approved; or a tiny `axum` mock in-test), point the binary at it, run 10 submissions, assert sidecar contents.

If `wiremock` isn't in tree, surface for approval; alternative is using `axum`'s test server (already in tree via `trace-commons-server` deps) for the mock.

- [ ] **Step 5: Commit.**

```bash
git commit -m "Wire submitter + sidecar + main loop for pilot bootstrap"
```

---

## Slice 5 — Multi-dataset support

### Task 5: pi-mono + DeepSeek-v4-Pro-Agent translators

**Files:**
- Modify: `crates/trace-commons-server/src/bin/pilot_bootstrap/translators.rs`
- Modify: `crates/trace-commons-server/tests/pilot_bootstrap_translators.rs`

Fill in the two stubbed translators per the spec's notes:

- `PiMonoTranslator`: pi-mono format has tree-structured `id`/`parentId` messages. v1 picks the longest top-level session and concatenates its messages.
- `DeepSeekAgentTranslator`: DeepSeek format is similar — concatenate `message.content[].text` for messages where `role == "assistant"`.

Tests for each translator with fixture rows.

- [ ] **Step 1: PiMonoTranslator + tests.**
- [ ] **Step 2: DeepSeekAgentTranslator + tests.**
- [ ] **Step 3: Auto-detect translator from dataset id** (so `--source jedisct1/agent-traces-swival` automatically uses SwivalTranslator without `--translator`).
- [ ] **Step 4: Commit.**

```bash
git commit -m "Add pi-mono + DeepSeek agent translators with auto-detection"
```

---

## Slice 6 — Operator runbook + roadmap

### Task 6: Document the harness

**Files:**
- Create: `docs/operator/pilot-bootstrap.md`
- Modify: `docs/trace-commons-roadmap.md`

Runbook content:

- What the harness is and is not (point to spec)
- Prerequisites: running `trace-commons-ingest`, bootstrap-tenant token configured, HF cache space
- Quick-start: 100-submission smoke + 30k-submission full run
- Sidecar interpretation: how to join sidecar entries with `trace_gate_decisions` rows
- Teardown: when to stop the harness; whether to keep the sidecar; vector-index implications

Roadmap entry:

```
- A.6: pilot-bootstrap HF-trace replay harness — done (binary; awaits operator run for first 30k submissions per A.6's "What success looks like" criteria)
```

- [ ] **Step 1: Runbook.**
- [ ] **Step 2: Roadmap.**
- [ ] **Step 3: Commit.**

```bash
git commit -m "Document pilot bootstrap harness in operator runbook + roadmap"
```

---

## Done criteria

- [ ] `cargo check -p trace-commons-server --bins` clean (default + `--features local-gpu-models`)
- [ ] `cargo test -p trace-commons-server --test pilot_bootstrap_translators` green
- [ ] `cargo test -p trace-commons-server` overall green (existing + new)
- [ ] Six commits on `feat/a6-pilot-bootstrap`, subjects matching the plan:
  1. `Add trace-commons-pilot-bootstrap binary scaffolding`
  2. `Add HF parquet dataset reader for pilot bootstrap`
  3. `Add Swival translator for pilot bootstrap (pi-mono + DeepSeek stubbed)`
  4. `Wire submitter + sidecar + main loop for pilot bootstrap`
  5. `Add pi-mono + DeepSeek agent translators with auto-detection`
  6. `Document pilot bootstrap harness in operator runbook + roadmap`
- [ ] Each commit carries the Co-Authored-By trailer
- [ ] No `--no-verify`, no `--amend`
- [ ] No emojis
- [ ] PR opened against `main`

---

## What this plan does NOT do

- **Does not run the harness.** That's A.6 operator activity post-merge.
- **Does not generate the first 30k submissions.** Same.
- **Does not write the post-run analysis report.** Same.
- **Does not modify the gate-service or `trace-commons-ingest` binary.** The harness uses the existing API surface.
- **Does not replace Ironclaw.** Specifically out of scope per the spec.

## Spec open questions parked here

1. **Submission rate (1/sec default).** Configurable via `--rate`; operator tunes after first 1000 submissions complete.
2. **Sidecar durability.** Flush per-submission; idempotency handles restarts.
3. **v1 envelope contract.** Confirm with `trace-commons-protocol` maintainer before Slice 4. If multi-part required, translator changes happen in Slice 3 + 5.
4. **Embedder diversity.** Slice 5 adds two more translators specifically to mitigate the single-source vector-index degeneracy concern.
5. **Real principal identities.** Single bootstrap-tenant for v1; multi-tenant in a future retrofit.
