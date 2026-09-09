# NEAR AI attestation drill

`POST /v1/admin/near-attestation-drill`

## What a passing run establishes

That at the moment the drill ran, the endpoint answering our inference
requests was an Intel TDX enclave, running an image whose measurements match
what we pinned, and the key that signed the drill's own inference receipt was
the key that enclave's hardware attests.

Everything in that sentence is scoped to one run and one request. It is not a
standing property of the endpoint, and a pass an hour ago says nothing about
the next request. Read the section below on what it does not cover before
relying on it for anything wider.

Nine steps, in order. Every one must pass; a step that did not run is
reported as `not_run` and is not a pass.

| Step | What it establishes |
|---|---|
| `report_fetched` | The endpoint served an attestation report for a nonce we generated this run. |
| `quote_verified` | The TDX quote in that report chains to Intel's root, via freshly fetched Intel collateral. |
| `tcb_up_to_date` | Intel's TCB verdict for the platform is `UpToDate`. |
| `nonce_bound_in_quote` | Our nonce is at `report_data[32..64]` of the **verified** quote — the report is fresh, not a replay. |
| `signer_binding_default_mode` | `report_data[20..32]` are zero, so `[0..20]` is a raw signing address. See "the zero-padding assertion" below. |
| `measurements_pinned` | Every pinned measurement register matches the verified quote. |
| `completion_performed` | One minimal completion succeeded. **This is the step that costs money.** |
| `receipt_verified` | Its receipt is validly signed over the exact request bytes we sent, the exact response body we received, and the model we asked for. |
| `receipt_signer_is_attested_key` | The receipt's recovered signer is the address the quote attests. |

The last step is the point of the drill. The other eight can all pass while
proving nothing together: an endpoint can proxy somebody else's genuine
attestation report and sign its own receipts with its own key, and every
individual check still comes back green. `receipt_signer_is_attested_key` is
what closes that.

## What it does not establish

- Nothing about any *contributor's* inference. The report is a public,
  unauthenticated document and the receipt covers only the request this drill
  itself made.
- Nothing about the image's *contents*. A measurement match says the image is
  the one whose registers you pinned, not that the image is trustworthy. That
  judgement happens once, when you decide what to pin.

## What it costs

Step 7 is a real, billed chat completion against the configured NEAR AI
model. It is bounded deliberately: one user message of one word, `max_tokens:
1`, `temperature: 0`, non-streaming. That is the smallest paid request the
receipt endpoint will produce a signature for.

Steps 1-6 cost nothing, and the drill **refuses to reach step 7 if any of
them failed** — there is nothing to learn by paying for a completion against
an endpoint we have not established. A run that refuses on configuration
never spends anything.

Rollout-smoke evidence goes stale after 24 hours, so a deployment that keeps
`near_attestation` green is paying for roughly one completion a day. See
"when the check is required" below for which deployments that applies to.

## Configuration

| Env | Required | Meaning |
|---|---|---|
| `TRACE_COMMONS_NEAR_AI_BASE_URL` | yes | The endpoint's `/v1` root, e.g. `https://qwen3-6-35b.completions.near.ai/v1`. Shared with the scorer. |
| `TRACE_COMMONS_NEAR_AI_MODEL` | yes | Model id. Shared with the scorer. |
| `TRACE_COMMONS_NEAR_AI_API_KEY` | yes | Bearer token. Never logged, never on the CLI. Shared with the scorer. |
| `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` | **yes, for the drill to mean anything** | Comma-separated `key=value` pins over `mrtd`, `rtmr0`, `rtmr1`, `rtmr2`, `rtmr3`. |
| `TRACE_COMMONS_NEAR_AI_PCCS_URL` | no | Collateral source. Defaults to Intel's own PCS, `https://api.trustedservices.intel.com`. |
| `TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS` | no | Per-call timeout, default 60. |

Any of the first three missing and the drill refuses with
`missing_control:near_ai_base_url` / `near_ai_model` / `near_ai_api_key`. It
never skips to a pass.

**`TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` must be set for the drill to
mean anything.** With it unset the drill still runs, and still fails, with
`missing_control:near_ai_expected_measurements` on the `measurements_pinned`
step. That is deliberate: an unpinned drill proves the endpoint is *an*
enclave, not *the* enclave, and reporting that as a pass would be worse than
not running it.

### The cargo feature

Fetching Intel collateral requires the binary to be built with
`--features near-attestation-collateral`. It is off by default because it
pulls a second async HTTP stack into every build.

Without it the route still exists and still runs; the `quote_verified` step
refuses with `missing_control:near_ai_attestation_collateral_client`. If you
see that, the fix is to rebuild with the feature, not to change anything
about the endpoint.

## Where expected measurements come from

**Verify a quote from a known-good endpoint and copy the registers off the
`VerifiedQuote`.**

Concretely: run the drill against the endpoint you intend to pin, with no
measurements configured. It will fail on `measurements_pinned` — and the
response body carries `outcome.mrtd` and `outcome.rtmr`, read out of the
quote that just verified against Intel collateral. Those are the values to
pin.

**Do not copy them from the report's `info.tcb_info` JSON, and do not copy
them from a NEAR AI web page or release note.** Both are the server's own
claim about itself, unsigned. Pinning against a claim the server makes
verifies exactly nothing: an endpoint that wanted to lie about its image
would simply lie in both places. The whole reason the pin is worth having is
that the value it is compared against comes out of a quote Intel signed.

The drill reports where the two disagree, in `outcome.json_claim_anomalies`.
That is reporting only, never gating — it means the endpoint is describing
itself inaccurately in a way the quote exposes, which is worth knowing on its
own terms.

## The zero-padding assertion

NEAR AI's own verifier README documents `report_data[0..32] =
SHA256(signing_address || spki_hash)`. That is true only when the report is
fetched with `?include_tls_fingerprint=true`. In the default mode — which is
what this drill uses — `report_data[0..20]` is the raw signing address and
`[20..32]` are zero. Both were confirmed against the live service.

The drill therefore asserts the zero padding as its own step. If the fetch
ever grows that flag, the padding stops being zero and
`signer_binding_default_mode` fails loudly, instead of the signer comparison
quietly checking an address against the first twenty bytes of a hash.

## Reading a failure

`blocking_gaps` names every step that did not pass, as
`<step>:<reason>`. The reasons are stable labels, never messages.

| Reason | What happened | What to do |
|---|---|---|
| `missing_control:near_ai_api_key` (or `_base_url`, `_model`) | The endpoint is not configured. | Set the env. Nothing reached the network. |
| `missing_control:near_ai_attestation_collateral_client` | Built without `near-attestation-collateral`. | Rebuild with the feature. |
| `missing_control:near_ai_expected_measurements` | Nothing pinned. | See "where expected measurements come from". |
| `quote_verified:verification_failed` | The quote did not chain to Intel, or the collateral was stale, or the platform matched no TCB level at all. | Investigate before anything else. This is the step everything else rests on. |
| `tcb_up_to_date:tcb_status:<status>` | Intel's verdict for the platform is not `UpToDate`. | The named status maps to an Intel security advisory. This is NEAR AI's platform to patch; the correct response is to raise it with them, not to accept the status. |
| `nonce_bound_in_quote:nonce_not_in_report_data` | The report is not bound to the nonce we sent. | Treat as a replayed or proxied report until proven otherwise. |
| `signer_binding_default_mode:report_data_signer_padding_not_zero` | The report came back in `include_tls_fingerprint` mode. | See above. Do not "fix" this by loosening the signer comparison. |
| `measurements_pinned:mismatch:<fields>` | The image changed. | See "after an image upgrade" below. |
| `receipt_verified:request_hash_mismatch` | The receipt is not over the bytes we sent. | A genuine mismatch here is serious; it is also what a caller bug looks like, so confirm the drill was not modified to re-serialize its request. |
| `receipt_signer_is_attested_key:receipt_signer_is_not_the_attested_key` | The signing key is not the attested one. | **Stop.** This is the substitution the drill exists to catch. Do not route inference through this endpoint until it is explained. |

There is deliberately **no configurable allow-list** for the TCB status, and
no switch that turns any step into a warning. A knob like that is the one
someone reaches for at 2am to make a red drill green, and the value of this
drill rests on there being no such shortcut in it.

That is a claim about this code, not about your deployment. Someone with
commit access can of course change any of it, and someone with the env can
unset the measurement pins — the point is that both are visible changes
somebody has to make and defend, rather than a supported setting. If you find
yourself wanting one, the thing to change is the endpoint, not the drill.

## After a NEAR AI image upgrade

A `measurements_pinned:mismatch` after NEAR AI ships a new image is
**expected**. It is not a bug in the drill; it is the drill doing its job.

The fix is to **re-pin, after review**:

1. Confirm from NEAR AI that an image change happened, and what changed.
2. Re-read the new registers off the verified quote, as above.
3. Update `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` and restart.
4. Re-run the drill and record fresh evidence.

The fix is **never** to unset `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`
or to stop running the drill. Say it plainly: a deployment that answers a red
drill by turning the check off is worse off than one that never had the check
at all. The first has a green board and no measurement pinning; the second at
least knows it has none.

## Running it

```bash
curl -sS -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"purpose":"weekly attestation drill","record_evidence":true}' \
  "$BASE/v1/admin/near-attestation-drill" | jq
```

`record_evidence: true` writes a `near_attestation` rollout-smoke evidence
row. A failed drill records **failed** evidence — the row follows the result.

### When the check is required

`near_attestation` is a required rollout-smoke check **only on a deployment
that has a NEAR AI endpoint configured** (`TRACE_COMMONS_NEAR_AI_BASE_URL`,
`_MODEL` and `_API_KEY` all set). Elsewhere it is reported in the summary's
`not_applicable_checks` and left out of `required_checks`, rather than
sitting permanently red on a deployment that routes no inference through NEAR
AI. A required check nobody can ever turn green teaches operators to ignore
red checks, which is the opposite of what this surface is for.

That condition keys on **whether the surface is in use at all — never on the
drill's outcome.** Once an endpoint is configured, the code offers no
allow-list, no severity dial and no acknowledgement flag: a red drill blocks
promotion, and the supported way to clear it is to fix what it found.

Rollout-smoke evidence goes stale after 24 hours, so a deployment in the
required case is paying for roughly one minimal completion a day.

Admin bearer token only. The response body is safe to paste into a ticket:
it carries the nonce, the verified measurement registers, the TCB status and
a per-step verdict, and it carries the API key, the receipt, the completion
text, the completion id and the signing addresses only as digests, or not at
all.

Note that this drill is **not** in the `REQUIRED_DRILLS` loop in
[`scripts/operator/smoke-gate.sh`](../../scripts/operator/smoke-gate.sh).
That loop POSTs an empty body and asserts a `success` field; this drill takes
a JSON body and reports `ready`, as every drill added since that script was
written does. Run it with the curl above.

## The attested-key drift probe

`POST /v1/admin/near-attestation-key-drift-drill`

A second drill against the same endpoint and the same credential, asking a
different question. The drill above fetches the `signing_algo=ecdsa` report and
checks the gateway. This one fetches the **`signing_algo=ed25519`** report and
derives the **per-model** keys that appear only in `model_attestations` — the
keys a `provider_tee` receipt is actually signed with, and therefore the keys
the client already depends on.

It is read-only. It pins nothing, admits nothing, spends nothing (there is no
paid completion in this drill at all) and changes no admission behaviour.
Running it can only produce a report and, optionally, one hash-only evidence
row.

Same auth as its neighbour: an **admin** bearer token, over
`authenticate_with_tenant_access_grant` plus `require_admin`.

### Why it reports several findings rather than one verdict

Under static pins, a silent key rotation at NEAR AI does not present as
"rotation". It presents as every contribution being refused with a signature
error that names nothing. The probe exists to tell an operator **which** thing
moved, so the response names each difference separately rather than collapsing
them into one red light:

| Label | What moved | Where to go |
| --- | --- | --- |
| `report_unavailable` | The report fetched before and does not now | The endpoint, or the network to it |
| `credential_rejected` | The endpoint stopped accepting a credential it accepted | Our NEAR AI credential |
| `gateway_key_rotated` | The gateway's ed25519 key changed | NEAR AI re-keyed the gateway |
| `model_keys_rotated` | The per-model key set changed | NEAR AI re-keyed the model; anything pinning those keys is now stale |
| `model_entry_count_changed:<before>-><after>` | The number of enclaves serving the model changed | Capacity change, or a model served from a new enclave |
| `measurement_moved:<register>` | One measurement register moved — one finding per register | A redeployed image; re-pin `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` only after establishing what was deployed |
| `tcb_status_changed:<before>-><after>` | Intel's verdict for the platform changed | Intel's TCB, not NEAR AI's code |
| `quote_verification_regressed` | The quote verified before and does not now | Expired collateral, or something worse |
| `quote_verification_recovered` | It did not verify before and does now | Usually a collateral window reopening |

The same findings appear structurally in `drift` (a tagged object per finding,
carrying the before/after values where there are any) and flattened in
`drift_labels`.

Alongside the findings, `credential` answers separately whether our existing
API key reaches the report endpoint at all: `accepted`, `unauthorized` or
`inconclusive`. `unauthorized` also pushes a named
`report_credential_unauthorized` gap, so "our key is not authorized for this
endpoint" never arrives as an anonymous HTTP-status failure. The probe cannot
tell an unset key from a wrong one — only that the endpoint refused.

### The baseline

The probe stores nothing. A run compares against the previous run's outcome
**only if you hand it one**:

```bash
# First run. Nothing to compare against; this is a baseline, not a failure.
curl -sS -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"purpose":"attested key drift baseline"}' \
  "$BASE/v1/admin/near-attestation-key-drift-drill" \
  | jq '.outcome' > attested-key-baseline.json
```

The response's `outcome` object is exactly what the next run accepts as
`baseline`, verbatim:

```bash
jq -n --slurpfile b attested-key-baseline.json \
  '{purpose:"attested key drift", record_evidence:true, baseline:$b[0]}' \
| curl -sS -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d @- "$BASE/v1/admin/near-attestation-key-drift-drill" | jq
```

Read the result in two parts, because they mean different things:

- `baseline_compared` is `false` when you sent no baseline. `drift` is then
  empty because there was nothing to compare, **not** because nothing moved.
  A first run is a baseline.
- `ready` describes **this run only** — every step passed. `drift_detected`
  describes the comparison. They are independent: a run can pass every step
  and still have drifted, which is the interesting case.

Roll the baseline forward — replace `attested-key-baseline.json` with the new
`outcome` — only once you have accounted for whatever the run reported.
Rolling it forward over an unexplained `model_keys_rotated` is how a rotation
becomes the new normal without anyone deciding that it should.

Two runs against **different models** are not comparable and report nothing:
per-model keys differ per model by design, so calling that drift would be
exactly wrong. The `model_label` in each outcome says which model was probed.

### Evidence and when the check is required

`record_evidence: true` writes a `near_attestation_key_drift` rollout-smoke
evidence row — its own check name, deliberately not the ECDSA drill's, so one
drill's evidence cannot satisfy the other's gate. The row is **failed** when
the run did not pass every step *or* when drift was found against a supplied
baseline. Green evidence beside a moved key would be worse than no evidence.

Like `near_attestation`, `near_attestation_key_drift` is a required
rollout-smoke check only where a NEAR AI endpoint is configured
(`TRACE_COMMONS_NEAR_AI_BASE_URL`, `_MODEL` and `_API_KEY` all set), and the
condition keys on the surface being in use — never on any drill's result.
Elsewhere it is reported in `not_applicable_checks`. Evidence goes stale after
24 hours.

Like its neighbour, this drill is not in the `REQUIRED_DRILLS` loop in
`scripts/operator/smoke-gate.sh`; run it with the curl above.

The response body is safe to paste into a ticket. Keys appear only as
`sha256:` digests, the API key and the base URL appear not at all, and the
values that do appear in full — the nonce this process generated, and the
measurement registers — are public image identifiers that a mismatch is
useless without.
