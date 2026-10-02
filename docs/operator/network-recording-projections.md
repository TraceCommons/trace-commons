# Anonymized network recording projections

`scripts/operator/anonymize-network-recording.py` processes **explicitly supplied
local response files offline**. It never discovers enrollment, reads credentials,
connects to a daemon, makes network requests or redeems an invite. No dependencies
are required beyond Python's standard library.

```bash
python3 scripts/operator/anonymize-network-recording.py \
  --input /private/capture/response.json --output /private/capture/projection.json \
  --surface ironwire_summary --source-kind authorized_recording \
  --authorized-source --capture-date 2026-10-02
```

`--authorized-source` records the caller's assertion that this response is
approved for use. It does not establish participant consent. Obtain the response
through already authorized read access and keep raw bytes outside git. Do not
pass a token or invite as a CLI argument. Use `--source-kind synthetic` for
invented test inputs. Non-200 HTTP responses use their observed `--http-status`
and emit only `readable:false` and that status, never the response body. Do not
substitute an HTTP status for a socket refusal.

The accepted surfaces are `invite_lookup`, `ironwire_log`, `ironwire_summary`,
`credit_summary`, `settlement_posture`, `mission_catalog` and the distinctly
labelled `commons_credit_ipc` result/refusal from the existing local daemon.
IronWire summary matches Cargo-pinned `b1bd241`, including flattened groups and
route totals. Unknown keys/versions, duplicate JSON keys, oversized/deep input,
nonfinite/negative counts and inconsistent summary proof/count totals fail
closed. A refusal creates no output; existing files and symlinks cannot be
clobbered. Errors print fixed labels, never input paths or response text.

Outputs contain only bounded numeric values, booleans, nulls and exact owned
labels. They discard issuer/model/backend names, account/session/call/mission
identifiers, source timestamps, hashes, tokens, URLs, paths, prose, confidence
objects and bodies. They preserve unknown numeric values as null, preserve
capture disabled and proof status, and label IronWire prices as catalogue prices
rather than billed spend. Currency is absent unless the observed posture says
both graded and live HTTP settlement. Even then it is an earned ledger figure,
not a payment promise.

These outputs are **allowlisted projections, not wire-response replay fixtures**.
Removing names, identifiers and timestamps intentionally prevents joins and
exact network replay. A mission catalogue projection retains only the schema,
entry count and presence of a next page. It contains no package or digest and
cannot establish package validity; do not rewrite a mission package while
retaining its original hash. The content digest covers the sanitized payload
only, avoiding an oracle over a low-entropy invite or raw source bytes.

## Committed evidence and limits

Files under `scripts/operator/fixtures/network-recordings/` have explicit
`source_kind`, surface, capture date, representation and sanitized payload digest:

- `ironwire-fake-local-projection.json` is **synthetic**. It projects the existing
  fake-local September 3 test recording, not an actual participant.
- `ironwire-log-local-pilot-projection.json` is an authorized October 2 local
  pilot read from the running native app's configured IronWire port. The GET
  `/log?limit=20` returned 200 with 20 recent historical rows. The separately
  reported `last_24h` window was empty; recent rows are not asserted to be from
  that window. Tokens and raw bytes stayed in memory and were never committed.
- `ironwire-summary-local-pilot-projection.json` records **404** from the same
  installed proxy's `/summary`. It is evidence of unavailable support on that
  installed version, not an empty measured summary.
- `commons-credit-local-pilot-ipc-projection.json` records the running daemon's
  `unknown_method` refusal for `commons_credit_summary`. It makes no claim about
  points or settlement and is not an HTTP credit/settlement recording.
- `mission-catalog-public-projection.json` records the fixed public
  `GET /v1/missions?limit=1` returning **503** on October 2. No credentials,
  redirects or response body were recorded.

Successful authorized invite lookup, current IronWire summary, HTTP account
credit/settlement and mission publication captures still require available
approved sources. These limitations must remain explicit when assessing Z1.
An unavailable response or synthetic fixture does not satisfy successful pilot
recording acceptance. Tests do not establish live services or contributor consent.

Run the privacy tests with:

```bash
python3 -m unittest discover -s scripts/operator/tests -p test_anonymize_network_recording.py
```
