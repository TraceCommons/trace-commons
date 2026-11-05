# Run the redaction witness at submission, not at preview

Status: APPROVED design. The owner decided every question in section 9
and approved the section 3 strings on 2026-11-05. No code changes.
Date: 2026-11-05.

Code references are to `origin/main` at c7997d6e0. Paths under
`crates/trace-commons-contributor/src/` are written without that prefix.

## The decision

When a redaction witness is configured, **the preview is built on the Mac
by the deterministic redactor, with no network, and the raw session goes to
the witness only when the contributor submits.** At submission the client
checks that the witness's envelope is the approved preview with only more
removed. If it is, the witness's bytes are uploaded. If it is not, nothing
is uploaded and the trace comes back to the contributor, showing what the
witness returned, for a fresh decision.

**The consent rule.** Approving a trace approves sending *this, or this
with more removed, and nothing else*. To find out what more to remove, the
full trace goes to the privacy witness when the contributor submits.

"More removed" is a mechanical check (section 4), not a judgement. It
holds by construction for every redactor the witness runs, provided the
witness's deterministic pass is at least the client's. Today it is not:
the path prefixes do not travel (section 4.3, Q1) and the live image
predates the tool-output detectors (section 7.3). Both need a witness
redeploy, and **the owner has held any redeploy until devices can refresh
their measurement pins** (section 7). So the order is: pin refresh in
clients, then redeploy, then this.

The second driver is time. The live witness took 114.7 s on 241 KB of
tool output against a 120 s client deadline, so a witnessed preview of a
real trace times out. At submission the call runs in the background with
a deadline matched to the witness's own, and retries (section 6).

## Why

The owner: "It's not clear to me that the witness should be called before
preview. The witness should probably happen on submission."

What happens today under a configured witness:

1. **The ordinary preview refuses.** `build_preview_core` returns
   `witness_claim_unavailable` for any enrolled client with a witness
   (`daemon/preview.rs:783-802`). The comment says why: the preview
   envelope is what `use_approved_envelope` later uploads, so a local
   preview would be an uncertified submission. And it records the cost:
   "with a witness configured, the desktop shells' approve-then-upload
   flow does not work."
2. **To see a trace at all, the contributor sends it to the witness
   first.** The only witnessed preview is the explicit
   `witness_preview_request` (`daemon/ipc.rs:8155-8346`), which refuses
   without `raw_session_confirmed: true` (`ipc.rs:8166-8173`), mints a
   claim, sends the raw session, and pins the certified bytes
   (`record_previewed_envelope`, `ipc.rs:8327-8331`). Every shell fronts
   it with a confirm step and the disclosure at `witness_copy.rs:792-797`:
   "This sends this trace, including its unredacted conversation ... to
   your configured remote witness before you approve a contribution."
   Callers: `macos/Sources/TraceCommonsApp/DaemonClient.swift:190-197`,
   `windows/src/TraceCommons.Interop/NativeWitnessReview.cs:10-16`,
   `crates/trace-commons-contributor-gtk/src/ui/preview.rs:959-961`,
   `tauri-desktop/src-tauri/src/commands/native_flows.rs:202-216`.
3. **So the raw session leaves the Mac before the contributor has
   decided anything**, and looking at a trace depends on the witness being
   up and fast.
4. **The live witness is too slow for an interactive deadline.** Measured
   2026-11-04 against the live witness: synthetic requests succeed, but a
   request carrying 241 KB of tool output took 114.7 s. The client gives
   a witness call 120 s (`HttpWitnessTransport::new(..., 120 s)`,
   `submit.rs:1004` for token review, `submit.rs:1378` for everything
   else; the deadline is a whole-request `reqwest` timeout,
   `witness/transport.rs:891-901`). Real sessions are larger: one queued
   Claude Code session was 868 KB. So a witnessed preview of a real trace
   times out and the app reports `preview-failed`, which is what the
   owner saw. The witness's own limit is 300 s
   (`DEFAULT_REQUEST_TIMEOUT_SECS`, `bin/trace-commons-witness.rs:110-124`,
   overridable by `TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS`, `:191-198`).
   This is the strongest practical reason to move the witness: at
   submission the call runs in the background, can wait longer, and can
   be retried, and nobody is staring at a spinner.

The automatic path already works the way the owner describes. An entry
with no pinned preview under a configured witness is witnessed inside
`submit_loaded` at upload time: claim minted first, raw session sent,
certified bytes uploaded (`submit.rs:1780-1880`). Nobody-looked entries
(`approved_unattended`) are held when the certificate's verdict is not
`low` (`daemon/uploader.rs:654-659`, `submit.rs:895-897`), and the held
certificate is saved so the person who opens it sees the witness's bytes
without a second witness run (`hold_for_review`, `uploader.rs:199-230`,
spec 2026-09-23 R5). This spec makes the interactive path match the
automatic one, and adds the check that makes a person's approval mean
something.

## 1. Terms

- **Approved preview.** The envelope the deterministic redactor built on
  the Mac and the contributor approved. Stored as today's plain local pin
  (`daemon/approved_envelope.rs` `save`/`load`, `queue.rs:405-426`).
- **Witnessed envelope.** The bytes the witness returns, with its
  certificate (`WitnessContributionResponse`,
  `crates/trace-commons-server/src/witness_service/mod.rs:752-760`).
- **Subset check.** The client-side comparison in section 4.
- **Witness pin.** Today's `witness-sha256:` pin
  (`approved_envelope.rs:152`, `queue.rs:833-837`): the contributor
  approved the witness's own bytes.

## 2. Flow

### 2.1 Interactive (a person approves)

1. **Preview.** Built locally, deterministic only, no network:
   `build_deterministic_preview_redactor` (`envelope.rs:145-149`) with the
   enrolled config. Lift the refusal at `preview.rs:800-802` for this
   case. The client's own `pii_filter` (NEAR AI) is *not* run for a
   witnessed client's preview: the witness runs the classifier, and a
   local classifier pass would make the subset check fail whenever the two
   classifiers disagree (section 4.4).
2. **Approve.** Pins the approved preview exactly as a non-witness client
   pins one today (`previewed_envelope_digest`, plain digest), together
   with `approved_inputs`. Settings invalidation is unchanged: `witness`
   and `pii_filter` are both in the input fingerprint
   (`preview.rs:2640-2660`), so changing the witness, its pins or the
   filter re-asks.
3. **Submit (upload pass or CLI).** The uploader loads the approved
   preview. Today `submit_loaded` refuses a local pin under a configured
   witness with `witness_certificate_missing` (`submit.rs:1717-1727`).
   That branch changes: rebuild the raw contribution from the transcript
   with the entry's `approved_verdict` and `approved_correction` (today the
   `Some(approved)` arm at `submit.rs:1694` builds no raw; only the `None`
   arm does), verify the witness's measurement pin (unchanged,
   `submit.rs:1790-1815`), mint the claim (`ensure_claim`,
   `submit.rs:1196-1244`), send the raw session, verify the certificate
   (`parse_witnessed_envelope`), run the residual-secret sweep (unchanged),
   then run the subset check against the approved preview.
   The approved preview is never itself uploaded under a witness; only
   witnessed bytes are. So dropping the `witness_certificate_missing`
   refusal loses no fail-closed property: the `approved_inputs`
   fingerprint, which includes `witness` (`preview.rs:2654`), still proves
   the preview was built under this witness configuration, and the check
   runs only after that passes.
4. **Pass.** Upload the witnessed bytes unchanged, with the certificate,
   exactly as a witness pin uploads today. The grant check is the
   existing `ensure_certified_grant` (`submit.rs:2813-2838`).
5. **Fail.** Upload nothing. Save the witnessed artifact (the R5 save,
   `hold_for_review`) and return the entry to Pending with a new label,
   `witness-changed-after-review`. Opening it shows the witness's bytes
   through `summarize_witnessed_preview` (`preview.rs:1061-1062`) with no
   second witness run. Approving it then pins the witness's bytes (a
   witness pin), and the next upload sends them as-is, with no check.

Showing the witnessed bytes on the second look means the raw session has
already gone to the witness under the first approval. That is acceptable:
the first approval *was* the decision to send, its copy said the full
trace goes to the witness on submit, and the automatic path already does
exactly this for held sessions. Nothing reaches the commons without a
person approving the bytes that are sent.

### 2.2 Approve without preview, and automatic folders

Unchanged. An approve-all or a row approved without opening it has no
approved preview, so `submit_loaded` takes the existing witness arm
(`submit.rs:1780-1880`) and sends what the witness returns. The subset
check has nothing to compare against and does not run. Automatic folders
keep the risk hold and the R5 save. The consent rule for these is the one
already stated by the connect-and-forget spec: the person agreed to send
without looking.

### 2.3 Admission evidence and token bundles

Out of scope; they keep the explicit review. Both build a different
document from the preview: admission sends an isolated final-call
contribution (`final_call_witness_input`, `envelope.rs:716-743`) and
refuses a verdict or correction (`submit.rs:1110-1124`); token review
builds a bundle whose summary is part of what the contributor approves
(`preview.rs:966-978`, `1045-1051`). Ingest requires a verified
certificate for both (`trace_commons_ingest_internal/admission.rs:614`,
`:946`; `token_bundles.rs:51-52`). A local preview of either would not be
the document sent. `witness_preview_request` stays, reached only from
those two flows.

### 2.4 CLI

`submit` already witnesses at submission: it has no approved preview, so
it takes the `submit.rs:1780-1880` arm. No change. `daemon preview
<entry_id>` gains the local preview under a witness, as the shells do.

## 3. What the contributor sees

Owner rules: very short; no product name; never "daemon" or "passkey".
There is deliberately no one-time confirm: the footer is on every preview
and carries the disclosure, so nothing new interrupts (copy-guide rule
9). The changed state is a new row state, not a banner, and was approved
with the rest under Q5.

| Where | Text | Status |
|---|---|---|
| Preview footer, witnessed client | "When you submit, your full trace goes to a secure privacy check. It can only remove more." | APPROVED 2026-11-05 |
| Row state while checking | "Running privacy check…" | APPROVED 2026-11-05 |
| Row state, check unavailable (retrying) | "Privacy check unavailable. Trying again." | APPROVED 2026-11-05 |
| Row state, `witness-changed-after-review` | "Changed by the privacy check. Review again to send." | APPROVED 2026-11-05 |
| Second preview header | "This is what the privacy check returned." | APPROVED 2026-11-05 |
| Row state, retries exhausted (section 6) | "Privacy check timed out. Try again later." | APPROVED 2026-11-05 (decision 12) |
| Refusal, over the size ceiling (section 6) | existing `failed_too_large` sentence (`witness_copy.rs:805`) | existing, approved earlier |

The disclosure at `witness_copy.rs:794` ("...before you approve a
contribution") stays for the admission and token flows only.

## 4. The subset check

A pure function in the contributor crate (permissive), over two parsed
envelopes plus the claim: `subset_check(approved, witnessed, token) ->
Ok | Err(label)`. Label-only errors, no content in logs.

### 4.1 Fields

**Must match exactly:** structure and every non-string leaf, plus every
field in `TYPED_METADATA_FIELDS` (`trace_contribution.rs:5336`), which the
classifier never sees. That covers `schema_version`, `submission_id`
(already required, `submit.rs:1160`), the whole `contributor` object,
`source_session`, and per event: count, order, `event_id`,
`parent_event_id`, `event_type`, `timestamp`, `failure_modes`,
`tool_category`, `latency_ms`, `token_counts`, `cost_usd`, `success`,
`side_effect`.
`outcome.human_correction` must match byte for byte (a correction is never
scrubbed, S5, `trace_contribution.rs:5023-5040`).

Every other string leaf anywhere in the envelope — `ironclaw`,
`replay`, `tool_name`, `tool_call_id` and payloads included — follows the
text rule
in 4.2, because the witness runs the classifier over metadata too
(`redact_metadata_value`, called at `trace_contribution.rs:4883`): a
display handle or model name it flags gets `[REDACTED:...]` on the
witness side only.

**Ignored, rebuilt per run:** `trace_id` (fresh `Uuid::new_v4()` per
build, `envelope.rs:835`), `created_at`, and all of `privacy`
(`redaction_counts`, `redaction_distinct_counts`,
`privacy_filter_summary`, `pii_labels_present`, `residual_pii_risk`,
`redaction_hash`, `warnings`, `redaction_pipeline_version`;
`trace_contribution.rs:5048-5057`). One monotone sanity check is kept:
for every label, the witness's count is at least the approved count.

**Consent and trace card:** come from the claim on the witness side, not
the config. Checked by `ensure_certified_grant` (granted scopes and uses
match the fresh claim) plus "granted is a subset of what the approved
preview showed", which `validate_review_grant` already enforces against
the config (`submit.rs:2785-2811`). `trace_card.tool_categories` must
match (derived from events).

### 4.2 Text

Every string — `redacted_content`, structured-payload strings, metadata
strings, and the **keys** of dynamic maps (payloads, `feature_flags`,
`tool_manifest_hashes`, `expected_assertions`; keys are redacted too,
`trace_contribution.rs:4385-4409`) — passes if:

> the witness's string equals the approved string with zero or more
> non-overlapping substrings each replaced by one placeholder token.

The placeholder grammar is closed. Every producer was read:

- `<PRIVATE_{LABEL}_{N}>` — deterministic placeholders
  (`PlaceholderMap::placeholder_for`, `trace_contribution.rs:4793-4804`).
- `[REDACTED:{label}]` — the prose classifier (`apply_spans`,
  `privacy_filter_spans.rs:95-110`).
- `[REDACTED]` — secret ranges (`trace_contribution.rs:4644-4651`).
- `<REDACTED_PRIVATE_KEY>` — PEM blocks (`:4634-4643`, `:8187`).

**Numbered placeholders are matched as symbols, not literally.**
Numbering is first-seen per label across the whole trace (`next_by_label`),
so a single extra deterministic hit on the witness side renumbers every
later placeholder of that label. The check builds one mapping per label
from approved `<PRIVATE_X_i>` to witness `<PRIVATE_X_j>` and requires it
to be a function and injective across the whole envelope (distinct stays
distinct, same stays same). A placeholder may also be swallowed whole by
a larger replacement.

**Object keys.** Keys equal on both sides are paired first. Each
remaining witness key must be derivable (rule above) from exactly one
remaining approved key; any ambiguity fails closed.

**Structural removals allowed, and only these:** on `HttpExchange`
events, `redacted_content` → absent, and removal of `body`/`headers`
under `request`, `response` or the payload root — what
`strip_inference_bodies` does (`witness_service/inference.rs:1017-1049`).
Nothing else may add, drop or reorder events, array items or keys.

Matching is a left-to-right scan with the approved string as the
template; tokens are recognised by the grammar, so the cost is linear in
the envelope apart from the key pairing. Bounded by the existing envelope
limit.

### 4.3 Does the witness only ever add placeholder redactions?

**For text the witness changes: yes, by construction.**

- The witness performs the redaction itself, through the same protocol
  code as the client: `redact_trace` over a `DeterministicTraceRedactor`,
  with or without a prose filter (`witness_service/mod.rs:843-880`;
  binary wiring `bin/trace-commons-witness.rs:452-460`).
- Deterministic first, classifier second, on the same text, in one shared
  helper (`redact_text_with_state_through_prose_filter`,
  `trace_contribution.rs:4273-4293`). The classifier sees the
  deterministic output, so its replacements are spans of that output.
- Both production classifiers — the NEAR AI filter
  (`privacy_filter_near_ai.rs:609`) and the self-hosted one
  (`privacy_filter_self_hosted.rs:197`) — return through
  `apply_windowed_spans` → `apply_spans`, which splices `[REDACTED:label]`
  at validated codepoint offsets after collapsing overlaps and keeps every
  other byte (`privacy_filter_spans.rs:37-110`, `293-310`). Windowing
  only splits the classifier's input; offsets are shifted back and the
  splice is done once over the whole field.
- **No truncation.** An over-limit field is refused, not cut
  (`privacy_filter_near_ai.rs:535-543`); a failed classifier is a
  refusal, never a deterministic-only downgrade
  (`trace_contribution.rs:4112-4117`). Scan-budget exhaustion leaves a
  value unchanged on both sides (`:4346-4348`, `:4364-4367`) under the same constants.
- **No normalisation of text.** `canonicalize_event_payloads`
  (`:7452`) orders payloads; the check compares parsed values, so key
  order does not matter.
- **Body stripping** removes fields and is allowed explicitly (4.2).
- **Tool-output compaction** (`redact_tool_specific_payload`,
  `trace_contribution.rs:7486-7540`: replace with `[REDACTED:label]`,
  sanitise a URL, summarise a collection) is a pure function of tool name,
  field name and value. It runs on the same input on both sides and is
  already present in the preview. Only a protocol version difference could
  make it diverge, and that surfaces as a structural mismatch: fail closed,
  second look.
- The "chunk-capped tail" seen in scoring is a scorer property, not a
  redaction one; nothing in the redaction path caps a tail.

**One configuration gap: the deterministic passes are not the same
pass.** The client seeds `known_path_prefixes` with the home directory
and the transcript's cwd (`envelope.rs:151-160`). The witness takes its
prefixes from an operator flag, empty by default
(`bin/trace-commons-witness.rs:200-209`, `427-433`), and the request
carries none (`WitnessContributionRequest`, `witness_service/mod.rs:708-721`).
The generic path regex covers `/Users`, `/home`, `/private/var` and
`/tmp` (`trace_contribution.rs:8277-8284`), so on macOS and Linux homes
are caught on both sides. It does not cover a Windows home
(`C:\Users\...`) or a cwd outside those roots. There the client masks the
path and the witness leaves it, so the witness's envelope has text the
preview did not — the check fails, correctly, on most Windows traces.

Fix (Q1): the witness request carries the client's prefixes as hints the
witness adds to its own, like `granted` (attribution only; they can only
add redactions, and they travel inside a request that already holds the
raw session). This is a witness image change, so a new measurement.

Version skew is the other source of divergence: a witness image whose
deterministic pass is older than the client's can redact less. That is
the live state today (section 7.3). The check
catches it and the fail path converges (the second approval pins the
witness's bytes, which are uploaded as-is). It costs a second look; it
leaks nothing.

### 4.4 Why the local preview runs no classifier

If the client ran its own classifier (NEAR AI) for the preview and the
witness ran a different one, every span the client's classifier caught
and the witness's missed would fail the check. The witness is the single
complete redaction point (spec 2026-09-23 R2); the client's pass is
defence in depth and must be a subset of the witness's. Deterministic
only, with the same prefixes, is.

## 5. Interactions

- **Residual-risk verdict.** A person-approved entry that passes the
  check uploads whatever the certificate's verdict; the person saw a
  superset of what is sent (Q3). The `approved_unattended` hold is
  unchanged.
- **Scopes.** Minted at submission, immediately before the witness call,
  so the gap that `witness-grant-changed` guards today (pin made at
  preview, upload later) mostly disappears. A 401/403 at upload still
  re-mints and refuses on changed grants (`submit.rs:3040-3060`). The
  `witness_claim_expired` label in the 2026-09-02 spec was never built;
  the live labels are `witness-grant-changed` and `witness-claim-invalid`.
- **Settings changes.** Covered by the fingerprint (`preview.rs:2654-2660`).
  Unchanged.
- **Pin age.** Local pins already expire after three days
  (`approved_envelope.rs:86`, `release_stale_pins`). Unchanged.
- **Attestation and measurement.** Still verified before anything raw is
  sent (`submit.rs:1790-1815`). Unchanged; just later.
- **Offline, timeouts, busy.** Preview works offline. Submission
  deadlines, retries and size limits are section 6. The
  `preview-errors-and-deadlines` branch (another agent) exists locally with
  no commits past `main` at time of writing; whatever labels it adds for
  the witness call should be reused on the submission path, and become
  moot for the ordinary preview.
- **Size.** Placeholders can be longer than what they replace, so the
  witness's envelope can exceed the approved size. The witness transport
  already enforces the envelope limit (`submit.rs:1383`); an over-limit
  result is refused for size as today. `would_send_bytes` shows the
  approved preview's size with a "may change slightly" reading (Q6).
- **Server.** No change. Ingest verifies a certificate against the body as
  received and fails open for ordinary submissions
  (`trace-commons-ingest.rs:15408-15447`); nothing server-side reads a
  preview digest (`previewed_envelope_digest` appears only in contributor
  code and a Windows interop test).
- **Migration.** None needed. Existing `witness-sha256:` pins keep
  uploading the witness's bytes through `holds_witness_certificate`. A
  witnessed client has no plain local pins today, because the preview
  refused. A plain pin under a witness, which was refused, is now checked.
- **Shells.** macOS first (lead): drop the "Prepare witness review"
  action from the ordinary preview (`PreviewSheet.swift:344-375`; keep it
  for admission/token), show the preview footer, the changed state and
  the second-preview header. GTK and Windows follow macOS. Tauri is
  frozen: it needs only to render the new label and stop calling
  `witness_preview_request` for ordinary previews.

## 6. Deadlines, retries and size at submission

The witness's throughput, not its availability, is the binding
constraint today (section "Why", item 4): about 2 KB/s of tool output on
the one measurement we have. At that rate the 868 KB session would need
roughly 400 s, over the witness's own 300 s limit, so moving the call to
submission is necessary but not sufficient for large sessions.

- **Deadline.** The submission path's client deadline becomes the
  witness's request limit plus a margin for transfer: 330 s against
  today's 300 s default, set as one named constant beside the 120 s ones
  it replaces (`submit.rs:1378`). **Revised 2026-11-05 (owner decision,
  #1973):** the witness's limit is now 540 s and the client's
  `WITNESS_REVIEW_TIMEOUT` 570 s. The owner asked for about 600 s and
  630 s, but the dstack gateway closes a connection after 10 minutes with
  no data, and the witness sends nothing until it answers, so both sit a
  minute under that. The daemon's preview cap moved from 7 to 11 minutes
  with them. The interactive token and admission
  reviews keep 120 s; a person is waiting on those. A client deadline
  shorter than the witness's limit abandons work the witness would have
  finished, and is the current bug.
- **Background class.** The upload pass already declares its witness
  calls background work (`witness_as_background`, `submit.rs:1458`),
  which a witness with reserved interactive slots turns away first
  (`reserved_interactive_slots`, `bin/trace-commons-witness.rs:190`).
  Keep that: submission is background work.
- **Retry.** A timeout, a connection failure and `witness_saturated` are
  transient. The entry stays Approved and the upload pass retries with
  its existing backoff (`REASON_WITNESS_SATURATED` carries the witness's
  own delay, `submit.rs:1856-1862`). Each retry sends the raw session
  again to the same pinned witness; the approval covers that, because it
  is the same send to the same verified recipient. After a bounded number
  of consecutive timeouts on one entry (3), the entry leaves the
  retry loop with the row state "Privacy check timed out. Try again
  later." and a manual retry; it is not dropped and not refused.
- **How the row states map to this.** "Running privacy check…" is shown
  from the moment the upload pass starts the witness call until it
  returns or hits the 570 s deadline (330 s before 2026-11-05). A timeout, connection failure or
  `witness_saturated` turns the row to "Privacy check unavailable. Trying
  again." until the next attempt starts, when it reads "Running privacy
  check…" again. The third consecutive timeout on the entry ends the loop
  with "Privacy check timed out. Try again later." No percentage: the
  witness reports none, and an invented one would be wrong.
- **Size.** The client refuses before sending anything when the raw
  contribution is over a witness size ceiling, with the label
  `witness-too-large` and the existing `failed_too_large` sentence
  (`witness_copy.rs:805`): "nothing left the machine". The ceiling is a
  configured byte figure derived from measured witness throughput and its
  request limit, not a guess; until it is measured, 256 KB of
  raw contribution (decision 9, lowered from 512 KB by the owner on
  2026-11-05 until batching is measured). The refusal is final for that trace under the
  current ceiling and is never retried, so an oversized trace cannot fail
  forever in the retry loop; it is re-offered automatically if the
  ceiling is raised. Splitting one session across several witness
  requests is not proposed: a certificate covers one envelope, and a
  merged artifact would need a new certificate format.
- **Raising the ceiling** is witness work, outside this spec: classifier
  throughput inside the enclave, and window concurrency for the
  self-hosted backend, whose window concurrency is a setting its own
  comment says is kept sequential on purpose
  (`privacy_filter_self_hosted.rs:163-166`, `:193`). The measurements, the in-process
  changes and the replica design are in
  [2026-11-05-witness-scaling-design.md](2026-11-05-witness-scaling-design.md):
  a request's time is classifier round trips, one per text leaf, not
  bytes.
- **Labels only**, as everywhere on this path: `witness-timeout`,
  `witness-too-large`, `witness_saturated`. No content, no host.

### 6.1 The witness worker (built 2026-11-05)

Owner direction, 2026-11-05: "the witness should not synchronously block
anything." This changes section 6's assumption that the upload pass makes
the witness call. The daemon now has a witness worker
(`daemon/witness_worker.rs`), and every daemon witness call runs there.

- **Async.** The upload pass runs every gate it always ran, then hands an
  approved entry that still needs a witness review to the worker and moves
  on. It never awaits a witness call, so watching, expiry, history,
  admission and other uploads are never held up by one.
- **Bounded concurrency.** One background call at a time
  (`BACKGROUND_SLOTS`), declared background so a busy witness turns it
  away first. A person's explicit review (`witness_preview_request` with
  `background: true`) has its own lane (`INTERACTIVE_SLOTS`), so it never
  waits behind background work.
- **Result.** The worker pins the certified review on the entry, which
  stays approved (`Queue::pin_witnessed_approval`); the next pass uploads
  exactly those bytes through the existing witness-pin path, which
  re-mints the claim, checks the certified grants and applies the risk and
  Scrub holds for a session nobody looked at. The worker never uploads.
- **Persistence and restart.** Nothing new is persisted. The entry stays
  approved while the witness works and the in-flight set is in memory, so
  a restart, stop or quiesce loses only the call, and the next pass sends
  the raw session again. That re-send happens only when the first request's
  result was never received: a received result is pinned on disk before
  anything else, and a pinned entry is never sent to the witness again. A
  result applies only while the approval it ran under still holds (a
  cancel or revoke meanwhile drops it).
- **Retries.** `witness_saturated` defers the entry and every other queued
  one until the witness's own delay. A witness whose evidence could not be
  fetched (`submit::witness_failure_is_transient`: `witness_timeout`,
  `witness_unreachable`, `witness_attestation_unavailable`,
  `witness_collateral_unavailable`) keeps the approval and is retried on
  the daemon's transient-upload backoff (#1964), and the worker takes no
  other session until that retry. Once that budget is spent the entry
  returns to Pending under the same label, held for a person; it is never
  refused. Evidence that was fetched and did not verify
  (`submit::witness_trust_hold`) is never retried: the entry returns to
  Pending under `witness-not-trusted`, or `witness-measurement-not-accepted`
  for a measurement a new accepted list may admit. This replaces the "3
  consecutive timeouts, then 'Try again later'" rule earlier in this
  section.
- **Pause, unenroll, panics.** A job checks pause, quiesce, the unenroll
  hold and shutdown before it sends anything. Unenroll stops a call in
  flight and waits for the worker. A panic in a job refuses that entry
  under `redaction-panicked` and rests the lane with backoff; the worker
  carries on.
- **Phase 4 plugs in here.** Today an entry with a plain (local) pin under
  a witness still refuses `witness_certificate_missing` at upload and is
  not sent to the worker (`witness_worker::needs_witness_job`). Phase 4
  widens that predicate to plain pins and runs the subset check in the
  worker between "witnessed" and "pin": pass pins the witness's bytes as
  now, fail saves them and returns the entry to Pending with
  `witness-changed-after-review`. The row states in section 6 map onto the
  worker: "Running privacy check..." while the entry is in flight
  (`status.witness_capacity.checking_sessions`), "Privacy check
  unavailable. Trying again." while it waits out a retry.
- **CLI.** `submit` still waits for its results, but works on two
  sessions at once (`CLI_SUBMIT_CONCURRENCY`) and prints progress per
  session.

## 7. Prerequisite: devices must be able to learn a new witness measurement

**Owner decision (2026-11-04): hold the witness redeploy until this
exists.** This spec depends on a redeploy (Q1's path prefixes, and the
image gap in 7.3), so this section ships first.

### 7.1 The problem

A device writes its witness pins once, when it joins, and never again:

- Wallet sign-up writes `witness: Some(witness)` into the new config
  (`daemon/account_onboarding.rs:1147-1151`). The module header says the
  bootstrap is trust-on-first-use over HTTPS from the chosen origin, and
  "Later witness checks enforce the persisted pins"
  (`account_onboarding.rs:4-7`).
- NEAR AI sign-up does the same with `commons.witness`
  (`daemon/nearai_onboarding.rs:1488-1494`).
- Connecting inference writes a witness once, with origin
  `ConnectedInference` (`daemon/inference_connection.rs:755`).
- A CLI client can read pins from `TRACE_COMMONS_WITNESS_*` environment
  variables (`config.rs:657-680`).

Nothing refreshes them. The servers learn new sets from their own
configuration: ingest's certificate check from
`TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS`
(`redaction_witness/config.rs:60`), and new sign-ups from the
provisioning document (`TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON`,
`trace_commons_ingest_internal/near_provisioning.rs:192`). A new witness
image changes the measured configuration (mrconfigid), so after a
redeploy every existing device refuses the witness with
`witness_expected_measurement` until someone re-pins it by hand. Under
this spec that refusal happens at submission, so every approved trace on
every existing device would stop uploading.

### 7.2 Design: a signed accepted-measurements list

- **Served** by the commons at a new unauthenticated read route (name to
  be chosen; none exists today), as one JSON document: a monotonically
  increasing `version`, an `issued_at`, and a list of accepted entries,
  each `{measurements, signing_address, not_before, not_after}`. The
  signing address is carried because a new image may also change it.
- **Signed** by a Cloud KMS asymmetric key in the commons' GCP project,
  algorithm `ec-sign-ed25519`, non-exportable (decision 11). Signing
  permission (`roles/cloudkms.signerVerifier`, granted on that key only)
  is held by the owner's account alone, and KMS audit logging is on, so
  every signature is logged. There is no key file anywhere; GCP is the
  recovery path. The key is separate from every service key, and no
  service host can sign with it.
- **Trust root.** The key's Ed25519 public key ships **in the client
  binary** as a release constant (a small set, so rotation can overlap;
  see below). Trust therefore does not rest on the HTTPS origin that
  served the bootstrap pins, nor on GCP at verification time: the client
  verifies an ordinary Ed25519 signature offline. A client with no
  compiled-in key never refreshes and behaves as today.
- **What is signed.** The document's canonical JSON bytes
  (`trace-commons-protocol` `canonical_json::canonicalize`). The served
  form carries the document plus a list of signatures, each naming the
  key version that made it. A client accepts the document if any one
  signature verifies against a compiled-in key.
- **Refreshed** by the client at start-up and daily, and on a
  `witness_expected_measurement` refusal before reporting it. A document
  is accepted only if its signature verifies, its `version` is higher
  than the stored one (no rollback), and its witness URL matches the
  configured one. The verified document replaces the stored accepted
  set; it never edits the URL.
- **Rollover.** The operator publishes old + new, deploys the new image,
  then publishes new only (or lets the old entry's `not_after` pass).
  Clients accept any unexpired entry.
- **Signing-key rotation.** Create a new KMS key version. Ship a client
  release whose compiled-in set holds both the old and the new public key,
  and publish lists signed by both versions meanwhile. Once that release
  has reached devices, sign with the new version only, then disable and
  later destroy the old version. A client never learns a signing key from
  the network.
- **Failure behaviour.** On the client: fetch failure, no verifying
  signature, lower version, or only expired entries -- keep the stored
  set, log a label. Never widen to unpinned, never fall back to TOFU from
  the serving origin. If KMS is unavailable, or the owner's account
  cannot sign, no new list is published; clients keep what they have and
  the served list stays the last signed one. That only blocks a redeploy,
  which must wait for a signed list carrying its measurement.
- **Consent interaction.** `witness` is in the approval fingerprint
  (`preview.rs:2654-2660`) and in the grant-terms void rule (spec
  2026-09-23 R6), so a naive refresh re-asks every approval and voids
  every automatic grant. A refresh delivered by a verified list should
  not: it is the same witness, re-measured, endorsed by the list key.
  The accepted set therefore lives outside the fingerprinted field; only
  a change of witness URL voids (decision 10).
- **Ships before the redeploy**, in a client release that has reached the
  devices that matter. Devices that have not updated keep refusing,
  which is the existing fail-closed behaviour, not a new failure.
- **Ingest** keeps its own environment configuration; serving the list
  does not change what ingest verifies.

### 7.3 Operational gaps found (open items)

- **The live witness image predates the tool-output detectors** (#1366,
  merged 2026-10-11 as a68b86898). Its deterministic pass lacks
  `tool_output_detectors` (`trace_contribution.rs:4626-4634`), which the
  client runs. Under section 4 that is a "witness removed less" case: the
  check would fail, correctly, on any trace with a git author, IP, private
  host, account name or phone in tool output. The subset check cannot
  ship ahead of a redeploy, so phase order is: pin refresh, then redeploy,
  then the check.
- **Deploy provenance.** The live witness's mrconfigid matches no
  documented deploy. Nobody can say from the repository which build is
  running. Each deploy should record its commit, image digest and
  resulting measurement in the operator docs, and the signed list's
  entries should name that commit.

## 8. Alternatives

| Option | For | Against |
|---|---|---|
| **A. Keep witness at preview, better errors and deadlines** | Contributor approves the exact certified bytes; no check to build. | Raw session still leaves before any decision; viewing still depends on the witness. The owner's objection stands. |
| **B. Local-only redaction for interactive approvals** | Fast, offline, simple. | Uncertified upload from a contributor enrolled as certified — the downgrade `preview.rs:786-797` exists to prevent. No classifier pass at all. |
| **C. Witness before preview behind an explicit "send for final check" step** | Consent is explicit. | This is today's flow (confirm modal + `raw_session_confirmed`). Same objection as A. |
| **D. Witness at submission, no check** | Already built (approve without preview). | The approval is of something other than what is sent; a witness bug that adds text would go out unseen. |
| **E. Witness at submission, subset check (recommended)** | Preview is local and offline; raw leaves only on submit; the person approves an upper bound on what is sent; certified bytes still go out. | A check to build and test; a second look on divergence; Windows needs Q1 first. |

**Recommendation: E.** It depends on a witness redeploy, which is held
until devices can refresh their measurement pins (section 7).

## 9. Owner decisions and open questions

### 9.1 Decided (owner, 2026-11-05: "go with recs")

1. **Path prefixes go to the witness**, as hints added to the witness's
   own, in a new measurement. It rides the redeploy that also brings the
   #1366 detectors (section 7.3). Windows witnessed clients keep today's
   flow until that redeploy is live.
2. **Consent rule:** "this, or this with more removed, and nothing else."
3. **A person-approved trace that passes the check uploads whatever the
   certificate's verdict.** The hold for traces nobody looked at stays.
4. **The copy calls the witness "privacy check".**
5. **The section 3 strings are approved as drafted**, including the
   `witness-changed-after-review` row state.
6. **The approved preview's size is shown unqualified** before
   submission.
7. **The second look shows the witness's bytes**, though the raw session
   has already gone to the witness under the first approval (2.1).
8. **`witness_preview_request` is retired for ordinary previews**, and
   kept for admission and token review.

### 9.2 Decided later the same day (owner, 2026-11-05)

9. **Witness size ceiling: 256 KB of raw contribution** (owner decision
   2026-11-05; was 512 KB) until throughput and batching are measured.
   The witness's request body cap is 1 MiB to match (#1973). Originally:
   512 KB
   is measured, refused before anything is sent (section 6).
10. **A signed measurement refresh keeps approvals and automatic
    grants.** Only a change of witness URL voids and re-asks (section
    7.2).
11. **The accepted-measurements list is signed by a Cloud KMS key**, not
    an offline key: `ec-sign-ed25519`, non-exportable, in the commons'
    GCP project, signing limited to the owner's account on that key only,
    KMS audit logging on, GCP as the recovery path. Clients compile in
    its public key (section 7.2). This replaces the recommended offline
    key with a second holder.
12. **"Privacy check timed out. Try again later." is approved** (section
    3).

## 10. Phased plan (sketch, no code)

Order is forced by section 7: refresh, then redeploy, then the check.

1. **Measurement refresh in clients (section 7.2).** KMS key created and
   its signing grant scoped; signed list format, compiled-in public key
   set, refresh at start-up, daily and on refusal,
   rollback protection, accepted set outside the approval fingerprint.
   Served route on the commons. Ship in a client release and let it
   reach devices.
2. **Witness redeploy.** One image carrying the #1366 detectors, the
   client path prefixes in the request (Q1), and the submission-length
   request limit. Publish old + new in the signed list first; record
   commit, image digest and measurement in the operator docs. Held by the
   owner until phase 1 has reached devices.
3. **Check.** `subset_check` in the contributor crate with label-only
   errors. Tests: property tests that splice random grammar tokens into
   random envelopes and must pass; any inserted byte, reorder, dropped
   event or non-stripped key removal must fail; renumbering must pass
   when injective and fail when it merges or splits; a replay of the
   deterministic + mock-classifier pipeline through the real
   `redact_trace` against a deterministic-only preview of the same raw
   must pass.
4. **Daemon and submit.** Lift the preview refusal for the ordinary
   path; replace the `witness_certificate_missing` refusal for a plain
   pin with witness + check; on fail, R5 save and
   `witness-changed-after-review`; 570 s submission deadline, timeout
   retry bound, size ceiling (section 6). Existing witness pins
   unchanged.
5. **macOS.** Strings, row states, preview footer, second-preview
   header; ordinary preview stops offering the witness review.
6. **GTK, Windows, Tauri** parity with macOS (Tauri minimal). Windows
   switches only once the phase 2 image, with path prefixes, is live
   (decision 1).
7. **Docs.** Update the 2026-09-02 witness-client spec and the
   2026-09-04 native-inference onboarding dependency 1 to point here.
