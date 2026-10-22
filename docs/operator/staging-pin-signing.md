# Signing the staging NEAR AI pin set

A staging build of the contributor app checks every NEAR AI quote against
measurement sets it takes from one signed document, served by the staging
site at `/near-ai-pins/current.json`, and from nowhere else (staging design,
section 5). This runbook covers the key that signs that document and
`trace-commons-staging-pins`, the tool that observes the measurements and
signs them. The staging environment runbook, when it lands, links here for
this step.

Everything below runs on an operator machine. Nothing here runs on the
staging host, and the staging host never holds the signing key.

## Build the tool

```bash
cargo build --release -p trace-commons-server \
  --features near-ai-staging,near-attestation-collateral \
  --bin trace-commons-staging-pins
```

`near-ai-staging` compiles the tool at all and points it at NEAR AI staging.
`near-attestation-collateral` compiles the Intel collateral client that
`observe` verifies quotes with; without it `observe` refuses with
`collateral_missing_control:near_ai_attestation_collateral_client` and prints
no set. `sign`, `verify` and `public-key` need only `near-ai-staging`.

## The signing key (owner, once)

The key is an Ed25519 seed: 32 random bytes written as 64 hex characters,
with no newline. It lives only in Secret Manager as
`tc-staging-nearai-pin-signing-key`. **It does not exist yet.** The owner
creates it once, piping the seed straight in so it never lands in a file or
on a command line:

```bash
openssl rand -hex 32 | tr -d '\n' \
  | gcloud secrets create tc-staging-nearai-pin-signing-key \
      --project <staging project> --data-file=-
```

Access:

- Grant `roles/secretmanager.secretAccessor` on this secret to the owner
  only (and later a dedicated pin-signing service account, staging spec
  section 10, item 7).
- **Never** to `tc-staging-runtime`, the staging host's service account. A
  compromised staging host can then serve a stale set, which expires, but
  never sign a new one. Check after any IAM change:

  ```bash
  gcloud secrets get-iam-policy tc-staging-nearai-pin-signing-key \
    --project <staging project>
  ```

  The output must not name `tc-staging-runtime`.

Read the public key off the secret. It is not secret, and it is what a
staging build compiles in (`STAGING_PIN_PUBLIC_KEY` in the contributor's
`channel.rs`):

```bash
gcloud secrets versions access latest \
    --secret=tc-staging-nearai-pin-signing-key --project <staging project> \
  | trace-commons-staging-pins public-key
```

It prints `public_key=<64 hex>` and `key_id=<16 hex>`. The key id is the
first 16 hex characters of the public key's SHA-256; every signed document
carries it.

The change that sets `STAGING_PIN_PUBLIC_KEY` to the printed `public_key`
must also add it, as one line, to `scripts/ci/staging-pin-public-keys.txt`,
in the same commit. That file is what the production release scan looks for:
a release binary carrying a listed key is refused.

Rotating the key means a new staging build with the new public key, and the
new key added to `scripts/ci/staging-pin-public-keys.txt` in the same change
(keep the retired key listed too). Keep the sequence climbing across a
rotation: devices remember the highest sequence they have accepted,
whichever key signed it.

## Re-signing

Re-sign when NEAR AI staging redeploys a model and its measurements change
(`observe` shows it), and before the served set expires (at most 14 days
after it was signed; 7 is the default).

1. Observe. The staging inference key goes in on stdin:

   ```bash
   gcloud secrets versions access latest \
       --secret=tc-staging-nearai-inference-key --project <staging project> \
     | trace-commons-staging-pins observe --model <model id> --model <model id> \
     > observed.txt
   ```

   Name every model the staging app uses, and name first the model the
   staging ingest's own receipts come from: sets are printed in the order
   they are first seen (model order, then quote order), and `sign` pins the
   ingest to the first one. `observe` fetches each model's
   attestation report from the staging gateway for a fresh nonce, verifies
   each quote against Intel collateral, requires an `UpToDate` TCB and the
   nonce bound inside the verified quote, and prints the distinct sets, one
   per line. If any model fails any check, it prints nothing and exits 1.
   `--registers` changes which registers a set names; the default is
   `mrtd,rtmr0,rtmr1,rtmr2,rtmr3`, what production pins.

2. Fetch the document the staging site serves now, so `sign` can refuse a
   sequence devices would treat as a rollback:

   ```bash
   curl -fsS https://staging.tracecommons.ai/near-ai-pins/current.json -o served.json
   ```

   The very first time, there is nothing to fetch: pass `--first` to `sign`
   instead of `--current`.

3. Sign. The signing key goes in on stdin; no argument takes it:

   ```bash
   gcloud secrets versions access latest \
       --secret=tc-staging-nearai-pin-signing-key --project <staging project> \
     | trace-commons-staging-pins sign --sets observed.txt \
         --sequence <served sequence + 1> --ttl-days 7 --current served.json
   ```

   It writes `current.json` (`--out` to change; an existing file is never
   overwritten), checks it with the same verifier the app runs, and prints
   one line on stdout:
   `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=<first set>`. On stderr it
   prints the sequence, the number of sets, the validity window, the key id
   and the payload's SHA-256.

4. Check it:

   ```bash
   trace-commons-staging-pins verify current.json --public-key <public key hex>
   ```

5. Install. Copy `current.json` to the staging host's pin directory, so the
   staging site serves it, and put the printed
   `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=` line in the staging
   ingest's drop-in and restart ingest (section 5.9: the ingest checks its
   own receipts against one set).

## Refusals

Every refusal prints `refused: <label>` on stderr, exits 1 and writes
nothing. An argument error (an unknown flag such as `--key`, a missing
required one) is clap's, and exits 2.

| Label | Meaning |
|---|---|
| `ttl_out_of_range` | `--ttl-days` is 0 or over 14 |
| `sequence_not_above_served` | `--sequence` is not above the served document's |
| `served_document_unreadable` | the `--current` file has no readable sequence |
| `no_sets`, `set_unparseable:line_<n>` | the sets file is empty, or line `n` is not measurement syntax |
| `signing_key_missing`, `signing_key_malformed` | stdin is empty, or not 64 hex characters |
| `output_exists` | the `--out` file already exists |
| `model_<n>:<label>` | `observe` refused the `n`th `--model`; the label names the check (`quote_verification_failed`, `quote_tcb_not_up_to_date`, `report_nonce_mismatch`, `quote_nonce_mismatch`, `report_model_not_attested`, `report_http_status`, ...) |
| `pins_*` | `verify`: the app would refuse the document, for the reason named (`pins_expired`, `pins_unknown_key`, `pins_bad_signature`, ...) |

Neither stream ever carries a key or a seed. Measurement sets appear only on
`observe`'s and `sign`'s stdout, which is their output.
