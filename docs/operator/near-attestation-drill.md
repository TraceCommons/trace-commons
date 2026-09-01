# NEAR AI attestation drill

`POST /v1/admin/near-attestation-drill`

## What it proves

That the endpoint answering our inference requests is an Intel TDX enclave,
running an image we reviewed and pinned, and that the key signing our
inference receipts is the key that enclave's hardware attests.

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
| `receipt_verified` | Its receipt is validly signed over the exact request bytes we sent and the response text we received. |
| `receipt_signer_is_attested_key` | The receipt's recovered signer is the address the quote attests. |

The last step is the point of the drill. The other eight can all pass while
proving nothing together: an endpoint can proxy somebody else's genuine
attestation report and sign its own receipts with its own key, and every
individual check still comes back green. `receipt_signer_is_attested_key` is
what closes that.

## What it does not prove

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
no switch that turns any step into a warning. A knob like that is the lever
someone pulls at 2am to make a red drill green, and the entire value of this
drill is that it cannot be made green except by fixing what it found.

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
drill's outcome.** Once an endpoint is configured there is no allow-list, no
severity dial and no acknowledgement flag: a red drill blocks promotion, and
the only way to clear it is to fix what it found.

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
