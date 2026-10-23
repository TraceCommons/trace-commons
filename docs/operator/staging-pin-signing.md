# Signing the staging NEAR AI pin set

A staging build of the contributor app checks every NEAR AI quote against
measurement sets it takes from one signed document, served by the staging
site at `/near-ai-pins/current.json`, and from nowhere else (staging design,
section 5). The document is format `trace-commons-near-ai-pins/v2`: next to
the sets it carries the RTMR3 and compose-action allowlist (owner decision
S16), and the app refuses a v1 document (`pins_v1_document`). This runbook covers the key that signs that document and
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
         --review review.json > observed.txt
   ```

   Name every model the staging app uses, and name first the model the
   staging ingest's own receipts come from: sets are printed in the order
   they are first seen (model order, then quote order), and the `ingest=`
   line is read from the first model's first quote. `observe` fetches each
   model's attestation report from the staging gateway for a fresh nonce,
   verifies each quote against Intel collateral, and requires an `UpToDate`
   TCB and the nonce bound inside the verified quote. Then, for each model
   entry (owner decision S16):

   - it replays the entry's own `event_log` with recomputed digests and
     requires the result to equal the verified quote's RTMR3, its RTMR3
     events to be the ten dstack events in order (`system-preparing`,
     `app-id`, `compose-hash`, `instance-id`, `boot-mr-done`, `mr-kms`,
     `os-image-hash`, `key-provider`, `storage-fs`, `system-ready`), and the
     `compose-hash` event to equal the compose hash in MRCONFIGID;
   - it verifies the entry's `compose_manager_attestation` quote the same
     way, requires its registers to equal the model quote's, our nonce at
     `report_data[32..64]`, and the SHA-256 of the actions (sorted-key
     compact JSON) at `report_data[0..32]`, and parses each action by kind
     (`compose_manager_started`, `compose_up`, `compose_down`), refusing an
     unknown kind or a key missing, extra or of the wrong type.

   It prints the distinct sets, one per line, and one
   `ingest=<mrtd and rtmr0-3>` line: the staging ingest's own pin, in
   production's form, because the ingest does not replay RTMR3 (section
   5.9). It writes `review.json` (never overwriting a file): the values of
   the replayed events, and under `compose_actions` the distinct
   compose-manager images (`compose_manager_images`, each
   `<name>@sha256:<64 hex>`) and the distinct compose files the
   `compose_up` actions brought up (`compose_files`, each `file`,
   `file_sha256` and `commit`), as a candidate allowlist in the shape `sign
   --allowlist` reads (owner decision S17). Services, tags and timestamps
   are not written: they are not matched. The per-boot `instance-id` value
   is never written anywhere. If any model fails any check, two entries
   carry different values for one event, a `compose_down` names a file and
   commit no `compose_up` in the run brought up
   (`review_compose_down_unlisted`), or a value cannot be listed
   (`review_compose_actions_unlistable`), it prints nothing, writes no
   review file and exits 1. `--registers` changes which
   registers a set names; the default is `mrtd,rtmr0,rtmr1,rtmr2,mrconfigid`
   (S16), and `sign` refuses a set that does not name at least those.

2. Review the allowlist. Compare `review.json` with
   `deploy/staging-gcp/near-ai-rtmr3-allowlist.json` (`diff`), check every
   change -- the app id, the KMS (`mr_kms`, `key_provider`), the OS image,
   the four progress markers, each compose-manager image digest, and each
   compose file -- and commit the reviewed file in a pull request. For each
   new or changed entry in `compose_files`:

   - fetch `file` from NEAR AI's repository at exactly `commit` (the
     commit, not the tag the action also carries: a tag can move);
   - compute its SHA-256 (`shasum -a 256 <file>`) and require it to equal
     `file_sha256`; a mismatch means the file reviewed is not the file that
     ran, so stop;
   - read the file and check every image it names, and what each service
     is given (mounts, environment, network), since a listed file may start
     any of its services.

   A listed file allows `compose_up` of that exact content and
   `compose_down` of that file at that commit; which services an action
   starts or stops is not checked, because the file defines them all. Never
   sign from an unreviewed `review.json`: the allowlist is what decides
   which post-boot containers a staging app trusts. Values are lower-case
   hex of each event's payload bytes; `key_provider` is the event's payload
   as it is (JSON naming the KMS), hex-encoded. As committed, the file is a
   placeholder with empty values and empty lists, which refuses every
   entry. `sign` accepts that placeholder or an allowlist filled in
   completely (every RTMR3 value, all four markers, at least one
   compose-manager image and at least one compose file) and refuses
   anything between as `allowlist_incomplete`.

3. Fetch the document the staging site serves now, so `sign` can refuse a
   sequence devices would treat as a rollback:

   ```bash
   curl -fsS https://staging.tracecommons.ai/near-ai-pins/current.json -o served.json
   ```

   The very first time, there is nothing to fetch: pass `--first` to `sign`
   instead of `--current`.

4. Sign, from the merged allowlist. The signing key goes in on stdin; no
   argument takes it:

   ```bash
   gcloud secrets versions access latest \
       --secret=tc-staging-nearai-pin-signing-key --project <staging project> \
     | trace-commons-staging-pins sign --sets observed.txt \
         --allowlist deploy/staging-gcp/near-ai-rtmr3-allowlist.json \
         --sequence <served sequence + 1> --ttl-days 7 --current served.json
   ```

   It writes `current.json` (`--out` to change; an existing file is never
   overwritten) with the allowlist inside the signed payload, checks it
   with the same verifier the app runs, and prints one line on stdout:
   `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=<the ingest= line's set>`.
   On stderr it prints the sequence, the number of sets, compose-manager
   images and compose files, the validity window, the key id, and the
   SHA-256 of the allowlist and of the payload.

5. Check it:

   ```bash
   trace-commons-staging-pins verify current.json --public-key <public key hex>
   ```

6. Install. Copy `current.json` to the staging host's pin directory,
   `/var/lib/tracecommons-site/near-ai-pins/current.json` (mode 0644), so the
   staging site serves it. Then rerun `deploy/staging-gcp/render-env.sh` with
   `TC_NEAR_AI_EXPECTED_MEASUREMENTS` set to the value of the printed
   `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=` line and restart ingest
   (section 5.9: the ingest checks its own receipts against one set). See
   `deploy/staging-gcp/README.md`.

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
| `set_missing_register:line_<n>` | line `n` does not name `mrtd`, `rtmr0-2` and `mrconfigid` |
| `ingest_pin_missing`, `ingest_pin_malformed` | the sets file has no `ingest=` line, or more than one, or one that is not exactly `mrtd` and `rtmr0-3` |
| `allowlist_unreadable`, `allowlist_malformed` | the `--allowlist` file cannot be read, or is not the allowlist shape (lower-case hex, the four marker names at most once each; under `compose_actions` exactly `compose_manager_images`, distinct `<name>@sha256:<64 hex>` references, and `compose_files`, each exactly `file` (not empty), `file_sha256` (64 hex) and `commit` (40 hex), at most one per file and commit; no other field) |
| `allowlist_incomplete` | the `--allowlist` file is neither the committed placeholder (every value and list empty) nor filled in completely: an empty app id, KMS, OS image or key provider, a progress marker not listed, or no compose-manager image or no compose file listed. Finish the review from `review.json` |
| `review_exists` | `observe`'s `--review` file already exists |
| `signing_key_missing`, `signing_key_malformed` | stdin is empty, or not 64 hex characters |
| `output_exists` | the `--out` file already exists |
| `model_<n>:<label>` | `observe` refused the `n`th `--model`; the label names the check (`quote_verification_failed`, `quote_tcb_not_up_to_date`, `report_nonce_mismatch`, `quote_nonce_mismatch`, `report_model_not_attested`, `report_http_status`, the replay's `rtmr3_log_missing`, `rtmr3_replay_mismatch`, `rtmr3_event_digest_mismatch`, `rtmr3_event_unknown`, `rtmr3_event_sequence`, `rtmr3_compose_hash_mismatch`, the compose-manager record's `compose_manager_missing`, `compose_manager_quote_unverified`, `compose_manager_other_td`, `compose_manager_nonce_mismatch`, `compose_actions_hash_mismatch`, `compose_manager_malformed`, `compose_action_unknown`, `compose_action_not_allowed` (a compose manager outside its `compose-manager` container), or `review_entries_disagree`, ...); `model_0:review_compose_down_unlisted` or `model_0:review_compose_actions_unlistable` for a run whose review could not be written |
| `pins_*` | `verify`: the app would refuse the document, for the reason named (`pins_expired`, `pins_unknown_key`, `pins_bad_signature`, ...) |

Neither stream ever carries a key or a seed. Measurement sets appear only on
`observe`'s and `sign`'s stdout, which is their output; allowlist values
only in `observe`'s review file and the allowlist file itself.

## What the app refuses

A staging app earns a model's key only if the signed set matches its quote,
the entry's RTMR3 log replays to that quote and carries the allowlisted
values, and its compose-manager record checks and lists only allowlisted
actions, all from one report fetched with one nonce. The daemon's status
reports the last outcome as `near_ai_attestation`: `attested`,
`not_checked`, or the refusal label (the `rtmr3_*` and `compose_*` labels
above, plus `rtmr3_app_id_not_allowed`, `rtmr3_mr_kms_not_allowed`,
`rtmr3_os_image_hash_not_allowed`, `rtmr3_key_provider_not_allowed`,
`rtmr3_marker_not_allowed` and `compose_action_not_allowed`).
