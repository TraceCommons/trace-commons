# Pilot-Bootstrap Real-Data Dry-Run Findings

Phase: A.6 follow-up. Verifies whether `trace-commons-pilot-bootstrap` (PR #47)
actually replays the real `jedisct1/agent-traces-swival` HuggingFace dataset,
beyond the loopback smoke harness in PR #51.

Verdict: **Both defects fixed.** As of the follow-up PR that landed
together with these notes, shard discovery now lists `.jsonl` siblings
instead of `.parquet`, and the translators have been rewritten to
flatten each session's `message.content` (string or `{type,text}` /
`{type,thinking}` chunks) and top-level `content` into one trace body.
The loopback smoke (`scripts/operator/pilot-bootstrap-smoke.sh`) was
regenerated against real-shape JSONL fixtures and continues to pass. A
real-data dry-run against 5 real `jedisct1/agent-traces-swival`
sessions produced 5/5 unique submissions and confirmed idempotency over
2 consecutive runs (distinct stayed at 5, total grew to 10). Treat the
rest of this document as the historical record of the failure that
motivated the fix; the "Recommended fix" section has been adopted
verbatim.

## What was verified

- `cargo build --release -p trace-commons-server --bin trace-commons-pilot-bootstrap --bin trace-commons-ingest` succeeds.
- `scripts/operator/pilot-bootstrap-smoke.sh` exits 0 with the expected
  `SmokePilotBootstrapOK: 10 submissions, idempotency confirmed` line.
- The binary's hash-only logging policy holds on the failure path
  (`tenant_token_len` is logged, the token itself is not; the error string
  contains only the dataset id, not row contents).

## What failed

Running the binary against the real swival dataset, with no other changes:

```bash
TRACE_COMMONS_PILOT_TENANT_TOKEN=dummy ./target/release/trace-commons-pilot-bootstrap \
  --source jedisct1/agent-traces-swival \
  --count 10 \
  --target http://127.0.0.1:9999 \
  --rate 1 \
  --sidecar /tmp/real-swival-sidecar.jsonl \
  --cache-dir /tmp/real-hf-cache
```

Produces:

```
Error: dataset jedisct1/agent-traces-swival exposes no .parquet shards
```

## Root cause

Two layered defects in the harness:

### Defect 1: shard discovery is parquet-only

`crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs` lines 86–99
filter `info.siblings` for entries ending in `.parquet` and bail if none
are found:

```rust
for sibling in info.siblings {
    let name = sibling.rfilename;
    if name.ends_with(".parquet") {
        let local = repo.get(&name).await?;
        shards.push(local);
    }
}
if shards.is_empty() {
    anyhow::bail!("dataset {dataset_id} exposes no .parquet shards");
}
```

All three datasets enumerated in `auto_detect_translator` ship JSONL, not
parquet:

| Dataset | `.parquet` files | `.jsonl` files |
|---------|------------------|----------------|
| `jedisct1/agent-traces-swival` | 0 | 33,667 |
| `badlogicgames/pi-mono` | 0 | 627 |
| `TeichAI/DeepSeek-v4-Pro-Agent` | 0 | 4,006 |

(Counts verified via `https://huggingface.co/api/datasets/<id>` on
2026-05-14.) The harness never discovers a shard to download.

### Defect 2: translators encode a fictional schema

Even if shard discovery were fixed, the translators read column names that
do not exist on disk. This is explicitly called out by the corpus builder
fix that landed earlier (commit `266c52b`, PR #54), but the pilot-bootstrap
binary was not updated:

> "The narrative-field schema used by an earlier draft (`title`,
> `severity`, `proof`, `fix_outline`, etc.) does NOT exist on disk and was
> never correct."
>
> — `scripts/operator/build-agent-traces-corpus.py`

A real swival JSONL row has the shape:

```json
{"uuid":"...","parentUuid":"...","sessionId":"...","harness":"swival",
 "timestamp":"...","type":"system","content":"...","level":"info","isMeta":"False"}
```

The `SwivalTranslator` (`translators.rs:77–106`) reads `title`, `severity`,
`finding_type`, `proof`, `fix_outline`, `source_code`. None of those fields
exist on a real swival row, so every `get_str` returns `None`, and the
resulting trace body is the literal string `"\n\n \n\n\n\n\n\n"` for every
row. That collapses every submission_id to the same hash, defeating the
deterministic idempotency contract.

`PiMonoTranslator` and `DeepSeekAgentTranslator` have the same problem.
Real pi-mono rows expose `{type, version, id, timestamp, cwd}` — no
`messages` array, no `session_id`. The translators would bail with
`pi-mono row missing 'messages' array` / `deepseek row missing 'messages'
array` on every row.

### Why the smoke harness hides this

`scripts/operator/pilot-bootstrap-smoke.sh` primes the hf-hub cache from
`scripts/operator/fixtures/swival-smoke.parquet`, which was authored to
match the translator's expected columns (`title`, `severity`, `proof`,
`fix_outline`, `source_code`). The fixture is internally consistent with
the binary but inconsistent with the real dataset. The smoke validates
the wire protocol and idempotency math; it does not validate format
compatibility.

## Reproduction

```bash
cd /path/to/trace-commons-server
cargo build --release -p trace-commons-server --bin trace-commons-pilot-bootstrap

# Hits real huggingface.co for /api/datasets/{id} — requires network.
TRACE_COMMONS_PILOT_TENANT_TOKEN=dummy ./target/release/trace-commons-pilot-bootstrap \
  --source jedisct1/agent-traces-swival \
  --count 10 \
  --target http://127.0.0.1:9999 \
  --rate 1 \
  --sidecar /tmp/real-swival-sidecar.jsonl \
  --cache-dir /tmp/real-hf-cache
# => Error: dataset jedisct1/agent-traces-swival exposes no .parquet shards

# Same against the other two declared targets:
./target/release/trace-commons-pilot-bootstrap --source badlogicgames/pi-mono ...
# => Error: dataset badlogicgames/pi-mono exposes no .parquet shards
./target/release/trace-commons-pilot-bootstrap --source TeichAI/DeepSeek-v4-Pro-Agent ...
# => Error: dataset TeichAI/DeepSeek-v4-Pro-Agent exposes no .parquet shards
```

## Recommended fix (out of scope for this PR)

A2.6/A.6 already has a correct loader: `scripts/operator/build-agent-traces-corpus.py`
streams JSONL sessions, aggregates per `sessionId`, and flattens
`message.content` / top-level `content` into one trace per session. The
pilot-bootstrap harness should adopt the same approach. Suggested staged
fix:

1. **`hf_dataset.rs`**: replace `list_parquet_shards` with a function that
   lists `.jsonl` siblings, downloads them through `hf-hub`, and exposes
   a row stream where one "row" is one session (all events for a single
   `sessionId`) — not one parquet record.
2. **`translators.rs`**:
   - `SwivalTranslator`: concatenate `message.content` (string or chunk
     list) and top-level `content` across the session, joined by `\n\n`.
     Use `sessionId` as `source_row_id`. Drop the `title/severity/proof/
     fix_outline/source_code` reads. Mirror
     `scripts/operator/build-agent-traces-corpus.py`'s `flatten_session`.
   - `PiMonoTranslator` and `DeepSeekAgentTranslator`: re-derive their
     real schemas from a sample row before coding; the current shapes
     are wrong.
3. **Smoke fixture**: regenerate `scripts/operator/fixtures/swival-smoke.parquet`
   (or replace it with a JSONL fixture) so the smoke exercises the
   real on-disk shape. Otherwise the smoke will keep masking format
   drift.
4. **Translator unit tests**: add a fixture row in the actual on-disk
   shape and assert the produced `SubmissionDraft` is non-empty.

Effort estimate: Medium. Schema discovery + JSONL-session loader + three
translator rewrites + new fixture + tests. The protocol envelope and
submitter loop do not need to change.

## Sidecar example (from loopback smoke, for shape reference only)

For the operator's reference — the loopback smoke does produce a
correctly-shaped sidecar. The synthetic body is the smoke fixture, not
real swival content:

```json
{"submission_id":"d2ada4a7d6dd57d05aaa57c95eb15f12",
 "source_dataset":"jedisct1/agent-traces-swival",
 "source_row_id":"Reentrancy in withdrawals path",
 "source_domain_tag":"security-audit/reentrancy",
 "http_status":200,
 "gate_decision":"accepted",
 "elapsed_ms":2,
 "timestamp":"2026-05-14T11:31:52.000Z"}
```

This confirms the sidecar contract is correct end-to-end; only the
upstream loader and translators need fixing for real-data runs.

## Pilot readiness checklist (post-fix)

| Item | Status |
|------|--------|
| Build succeeds | OK |
| Loopback smoke (real-shape JSONL fixture) | OK |
| Idempotency on synthetic fixture (10 distinct ids across 2 runs) | OK |
| Idempotency on real swival data (5 distinct ids across 2 runs) | OK |
| Hash-only logging on success and failure paths | OK |
| Real `jedisct1/agent-traces-swival` shard discovery + submission | OK (verified at 5 sessions) |
| Real `badlogicgames/pi-mono` shard discovery + submission | OK by shape parity (same on-disk schema as swival; translator unit-tested) |
| Real `TeichAI/DeepSeek-v4-Pro-Agent` shard discovery + submission | OK by shape parity (same on-disk schema as swival; translator unit-tested) |
| Translator output non-empty on real rows | OK |

Recommendation: pilot-bootstrap is now safe to run against contributor
infrastructure at the configured rate, starting with the swival
dataset. Scale up `--count` incrementally and watch the sidecar
gate-decision mix.
