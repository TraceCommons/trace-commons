# Self-hosted OpenAI Privacy Filter as a privacy-filter backend

The PII backstop is wedged. `project_pii_backstop_wedged_chunk_size` records
the symptom: the 20 KB classify chunk now returns 502 on essentially every
request, transient failures are charged to the submission, and 248 traces sit
held on `awaiting_pii_backstop`. The upstream is NEAR AI Cloud's
`/v1/privacy/classify`, and the model it serves is `openai/privacy-filter`
(`privacy_filter_near_ai.rs:19`, `DEFAULT_MODEL`).

That model is Apache 2.0 open weights. Nothing about the dependency is
proprietary to NEAR AI. This document specifies running it ourselves.

## Why self-hosting is the right shape, not a workaround

`openai/privacy-filter` was published 2026-04-22: 1.5B total parameters with
50M active, a bidirectional token classifier rather than a generative model,
128k context, Apache 2.0. It labels a sequence in a single forward pass and
decodes spans with a constrained Viterbi procedure. Its taxonomy is exactly
the eight categories the existing adapter already parses -- `private_person`,
`private_address`, `private_email`, `private_phone`, `private_url`,
`private_date`, `account_number`, `secret`.

Three consequences follow, and they are the whole argument:

- **The 128k single-pass context retires chunking.** `CLASSIFY_CHUNK_BYTES`
  (`privacy_filter_near_ai.rs:27`) exists because the hosted endpoint caps
  tokens per request. Per `project_near_ai_classify_batching_and_rate_limit`,
  that cap is signalled as a generic 502, which is why the wedge was hard to
  diagnose. A local process has no such cap, so the window-and-stitch path --
  the thing that broke -- is not merely fixed, it is deleted.
- **The text stops leaving our infrastructure.** This is a strict upgrade over
  the status quo. The workload whose entire job is finding PII currently ships
  unredacted contributor prose to a third party. Self-hosting ends that.
- **No 502s to absorb.** `MAX_CLASSIFY_ATTEMPTS = 4` with backoff exists to
  paper over a flaky WAN dependency. Loopback does not need it.

### Hosted alternatives were considered and rejected

No commodity inference provider serves this model. DeepInfra, Together,
Fireworks, Groq, Novita and OpenRouter all sell OpenAI-compatible
`/chat/completions` for generative models; a bidirectional token classifier
does not fit that API. That is precisely why NEAR AI had to invent a
proprietary `/privacy/classify` route for it.

The one real alternative is a Hugging Face dedicated Inference Endpoint, which
supports `token-classification` natively. It is rejected because it is a
dedicated always-on instance either way -- so it is paying someone else to run
the same process -- it still requires a new Rust adapter, since HF returns its
own `entity_group`/`start`/`end`/`score` shape rather than NEAR's
`data[].spans[]`, and above all because it means sending unredacted
contributor text to a host that makes no confidentiality claim. NEAR AI was
chosen partly because it is TEE-hosted private inference
(`project_near_ai_perplexity_validated`). Replacing confidential compute with
general SaaS, for the pre-redaction PII path specifically, is a downgrade on
the axis that matters.

Hugging Face's own guidance for server-side use of this model is
`gradio.Server` inside your own deployment. The vendor's answer is to run it
yourself.

## Approach: a sibling adapter, not a generalised one

Three options were weighed.

Repointing `TRACE_NEAR_AI_PRIVACY_BASE_URL` at localhost needs zero Rust and
is rejected outright: the API key would stay mandatory, chunking would stay at
20 KB, and every log line, `/health` field and boot canary message would
report `near_ai` while talking to a local process. In a repo whose logging
discipline is hash-only-but-accurate, that makes the backlog drain
unauditable.

Generalising `NearAiPrivacyFilterAdapter` into a parameterised classify client
saves perhaps eighty lines, at the cost of editing a module both the server
and the contributor crate depend on, and of blurring the
`privacy_filter:<backend>_failure` labels. It puts a live working path at risk
for no gain.

**The chosen approach is a sibling module.** `privacy_filter_self_hosted.rs`
gets its own `build_from_env` and backend label; the NEAR AI path is left
byte-identical, so the fallback is genuinely untouched rather than probably
still fine. The span-to-`[REDACTED:label]` decoding -- the security-critical
part -- is extracted into a shared helper so it stays single-sourced.

## Topology

`tc-pilot-host` is `e2-standard-2`: 2 vCPU, 8 GB, already running ingest and
the bge-large ONNX embedder. Per `project_pilot_cpu_starved_by_local_embedder`
the box is CPU-starved today, before adding anything.

Resize to `e2-standard-4`. Not larger: with 50M active parameters, per-token
compute is small, and the binding constraint is resident memory -- roughly 3 GB
at bf16 -- against 8 GB shared with two existing consumers. Four vCPU also
relieves the pre-existing embedder starvation, which is independent
justification for the resize. Measure the drain rate before going further.

**A GCE machine-type change requires stopping the instance.** This is a
scheduled pilot downtime window, not a rolling change, and it must be planned
as one. GCE pricing for the resize has not been verified and is deliberately
not quoted here.

The filter runs as its own systemd unit, `trace-commons-privacy-filter`, as a
dedicated unprivileged user, bound to `127.0.0.1`. Model weights are staged to
local disk at deploy time; there is no Hugging Face fetch at boot, so startup
is deterministic and fails closed rather than hanging on a network dependency
the fail-closed convention says we should not have.

## The serving shim

A small FastAPI/uvicorn service wrapping the official `opf` package, exposing
`POST /v1/privacy/classify` with request `{model, input}` and response
`{data:[{spans:[{category,start,end,score}]}]}`.

Reproducing NEAR's wire shape exactly is deliberate. It means the Rust span
decoder is one implementation shared by both backends rather than two that can
drift apart, and it means the shadow comparison below is a direct diff rather
than a translation.

Python and torch entering the deployment is a real cost against the
Rust-first stack preference. It is accepted here because this is the reference
implementation of a security control, and correctness of span semantics
outweighs stack purity. It is contained behind an HTTP boundary, so it can be
replaced by a Rust `ort` implementation later without the server changing.
Versions are pinned and weights staged; this is a serving shim, not a pipeline.

## Rust changes

New `crates/trace-commons-protocol/src/privacy_filter_self_hosted.rs`:

- `build_from_env()` reading `TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL`
  (required), `_MODEL` (default `openai/privacy-filter`), `_TIMEOUT_MS`, and
  `_MAX_INPUT_BYTES` (default `MAX_TRACE_ENVELOPE_BYTES`).
- **No API key.** The transport is loopback; a mandatory dummy secret would be
  a smell, and the existing `MissingEnv` refusal for
  `TRACE_NEAR_AI_PRIVACY_API_KEY` must not be copied over.
- Backend label `self_hosted`, so `privacy_filter:<backend>_failure`, the
  `/health` `privacy_filter_backend` field, and `run_privacy_filter_canary`
  all report which backend actually answered.
- **No windowing.** One request per field. A hard `max_input_bytes` ceiling
  remains as a resource bound, but `CLASSIFY_CHUNK_BYTES` and the stitching
  path are not used by this backend.
- A new cargo feature `self-hosted-privacy-filter = ["dep:reqwest"]`. No new
  third-party crate is introduced; `reqwest` is already an optional dependency
  of the protocol crate.
- A `self-hosted` arm on `TRACE_PRIVACY_FILTER_BACKEND`. Unknown values already
  refuse startup (`env-reference.md:421`), so the change is contained.

`near-ai` remains selectable. The brief is to move off it until its bugs are
fixed, not to delete it.

## Offsets: the one thing that must not be wrong

Per `project_near_ai_privacy_filter_quirks`, NEAR AI returns **codepoint**
offsets, and the current adapter compensates for that. If the shim emits UTF-8
byte offsets and the adapter assumes codepoints, redaction lands on the wrong
bytes: the PII survives, unrelated text is destroyed, and the call reports
success. The control fails silently in the direction of leaking exactly what
it exists to remove.

This is therefore the first failing test written, before any implementation,
and it uses multi-byte input -- emoji and CJK -- not ASCII. The shim's offset
convention is fixed by that test and asserted at the contract boundary, not
assumed from either side's documentation.

## Cutover and backlog drain

Deploy with the backend still `near-ai` and verify the shim independently.
Then flip to `self-hosted` with `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1` still
set, so a broken shim refuses the boot rather than silently degrading to
deterministic-regex-only redaction.

The 248 held traces are not drained in one step.

1. **Shadow comparison first.** Run both backends over a sample and diff the
   returned spans, hash-only. Same weights does not mean same output -- NEAR
   may post-process -- and the drain is a one-way write over real contributor
   text. This gates everything downstream.
2. **Bounded canary batch.** A small `TRACE_COMMONS_PII_BACKSTOP_BATCH_SIZE`,
   with the hash-only evidence inspected before proceeding.
3. **Attempt-counter reset.** Traces that exhausted
   `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS` against a backend failing 100% of
   the time stay stuck even once the backend is healthy, because the
   enumeration filter stops returning them. An admin route clears those
   counters for submissions on `awaiting_pii_backstop`, returning counts only.
   It follows the existing `/v1/admin/*` shape and requires the EdDSA-signed
   admin JWT; per `project_pilot_admin_token_mechanism` the pilot refuses
   static tokens.
4. **Full drain**, monitored.

Note that `project_quarantine_is_mostly_processing_failure` found 112 of 177
quarantined traces were attempts-exhausted rather than assessed. The same
distinction applies here: this backlog is mostly a processing failure, and
draining it is not a privacy adjudication.

## Testing

Test-first throughout.

- Multi-byte offset fidelity, as above. Written first, failing first.
- Adapter HTTP behaviour, mirroring the shape of the existing
  `crates/trace-commons-protocol/tests/privacy_filter_near_ai_http.rs`.
- Schema-parity contract test pinning the shim's request and response shapes.
- Backend-selection tests: `self-hosted` resolves, no API key is required,
  unknown values still refuse startup, and the reported backend label is
  `self_hosted`.

Verification before any green claim, per the repo's CI trap
(`feedback_warnings_as_errors_trap`):

    RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
    RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
    cargo clippy -p trace-commons-server --all-targets -- \
      -A clippy::type_complexity -A clippy::collapsible_if \
      -A clippy::manual_option_as_slice -A clippy::useless_vec \
      -A clippy::redundant_pattern_matching
    cargo fmt --all

The `near-ai-scorer` feature must be built explicitly: it is what the pilot
compiles, and a change that only builds under default features will pass
locally and fail CI. Per `project_envelope_digest_pin_coupling`, protocol
changes move a golden hash in the contributor crate, so the workspace is
tested, not just the one crate.

## Risks

**Offset convention.** Highest severity, silent failure mode, mitigated by
writing that test first.

**Downtime window.** The resize stops the instance. Must be scheduled.

**Model output drift versus NEAR's hosted variant.** Mitigated by the shadow
comparison gating the drain.

**Python and torch in the deployment.** Mitigated by pinned versions, staged
weights, an unprivileged user, and loopback binding. Accepted as the cost of
using the reference implementation of a security control.

**Resource contention on a box that is already starved.** Mitigated by the
resize; confirmed by measuring drain rate rather than assuming.

## Out of scope

Perplexity scoring stays on NEAR AI's Qwen3.6-27B. That path is not the one
that is broken.

Client-side redaction is not proposed here, though it is the obvious next
question: the model runs in a browser via transformers.js and on a laptop via
`opf`, and the contributor crate already carries the privacy-filter feature.
Moving redaction to the contributor's machine, so PII never reaches the server
at all, is a larger change and a separate design.
