# The signed witness accepted-measurements list

A contributor device pins its redaction witness once, when it joins: a URL, a
signing address and one or more measurement sets. A witness redeploy changes
the image measurement (`mrconfigid`), and without a way to learn the new one
every existing device refuses the witness with `witness_expected_measurement`
until someone re-pins it by hand.

The commons therefore publishes one small signed document, the accepted
list, at `GET /v1/public/witness-accepted-list` on ingest. Clients fetch it at
start-up, about daily, and when a witness is refused for its measurement;
verify its signature against list-signing public keys compiled into the
client; and accept a witness whose measurement **and** signing address match
any entry valid at the time, in addition to their sign-up pins.

Design: spec `docs/superpowers/specs/2026-11-05-witness-at-submission-design.md`,
section 7, and owner decisions Q10 and Q11 (2026-11-05). Format:
`crates/trace-commons-protocol/src/witness_accepted_list.rs`.

What a client does with a list:

- Uses it only if a signature verifies under a compiled-in key.
- Uses it only if its `witness_url` equals the configured witness URL.
- Replaces its stored list only with a strictly higher `version`. A replayed
  or repeated version is ignored.
- Judges each entry's `not_before` / `not_after` at the moment it checks a
  witness, so a not-yet-valid entry can be published ahead of time.
- On any failure (fetch, signature, version, URL, a measurement that does not
  parse) keeps what it has and logs a fixed label. It never goes unpinned and
  never trusts an unsigned list.
- Keeps approvals and automatic grants: the list is stored apart from the
  witness settings, so a refresh re-asks nothing. A change of witness URL
  still re-asks, as before (Q10).

**Current state: on.** Clients trust one list-signing key, compiled into
`TRUSTED_LIST_SIGNING_KEYS` in
`crates/trace-commons-contributor/src/witness/accepted_list.rs`:

| Key | Key id | Public key (raw, hex) |
|---|---|---|
| list-signing key, KMS version 1 (created 2026-11-05) | `ea396d6a7f0e867f` | `fed526376edb29b3328cb6c4a0a1f6a2d6c1267641b705cd276c5adf21d0a523` |

Every signature in a served list names its key id; a list whose signatures
all name other ids is refused. Until a signed list is installed on ingest,
the route answers 404 and clients keep their sign-up pins.

## The tool

```bash
cargo build --release -p trace-commons-server --bin trace-commons-witness-list
```

Subcommands: `payload` (validate an operator-written list and write the
canonical bytes that are signed), `sign` (sign through Cloud KMS and write
the served document), `verify` (check a served document as a client would),
`public-key` (convert the KMS public key to the form compiled into clients).
Everything runs on an operator machine with `gcloud` signed in as the owner.
The ingest host never holds the key and never signs.

## The list-signing key (owner, once)

The key is a Google Cloud KMS asymmetric signing key, algorithm
`ec-sign-ed25519`, in the commons' GCP project. It cannot be exported, and
there is no key file anywhere. Placeholders below: `<PROJECT>`, `<LOCATION>`,
`<KEYRING>`, `<KEY>`, `<OWNER_EMAIL>`.

```bash
gcloud kms keyrings create <KEYRING> \
  --location <LOCATION> --project <PROJECT>

gcloud kms keys create <KEY> \
  --keyring <KEYRING> --location <LOCATION> --project <PROJECT> \
  --purpose asymmetric-signing \
  --default-algorithm ec-sign-ed25519
```

`--protection-level` defaults to `software`. Use `hsm` instead if
`gcloud kms keys create --help` and the KMS documentation for `<LOCATION>`
list Ed25519 for HSM keys there.

Signing permission for the owner's account only:

```bash
gcloud kms keys add-iam-policy-binding <KEY> \
  --keyring <KEYRING> --location <LOCATION> --project <PROJECT> \
  --member user:<OWNER_EMAIL> --role roles/cloudkms.signerVerifier
```

`roles/cloudkms.signerVerifier` covers signing and reading the public key.
Grant it to nobody else, and never to a service account an ingest, witness or
CI host runs as. Check after any IAM change, on the key and on the project:

```bash
gcloud kms keys get-iam-policy <KEY> \
  --keyring <KEYRING> --location <LOCATION> --project <PROJECT>
gcloud projects get-iam-policy <PROJECT> \
  --flatten 'bindings[].members' \
  --filter 'bindings.role:roles/cloudkms' \
  --format 'table(bindings.role, bindings.members)'
```

Turn on KMS data-access audit logging, so every signature is recorded (admin
activity logs are always on). Add this to the project's IAM policy
(`gcloud projects get-iam-policy <PROJECT> --format json > policy.json`, edit,
then `gcloud projects set-iam-policy <PROJECT> policy.json`):

```json
"auditConfigs": [
  {
    "service": "cloudkms.googleapis.com",
    "auditLogConfigs": [
      { "logType": "ADMIN_READ" },
      { "logType": "DATA_READ" },
      { "logType": "DATA_WRITE" }
    ]
  }
]
```

Merge it with any `auditConfigs` already present rather than replacing them.

### Export the public key and compile it in

```bash
gcloud kms keys versions get-public-key 1 \
  --key <KEY> --keyring <KEYRING> --location <LOCATION> --project <PROJECT> \
  --public-key-format pem --output-file list-key-v1.pem

trace-commons-witness-list public-key --pem list-key-v1.pem
```

It prints `public_key_hex=<64 hex>` and `key_id=<16 hex>` (the first 16 hex
characters of the key's SHA-256; every signature names it). The public key is
not secret.

Add the `public_key_hex` value to `TRUSTED_LIST_SIGNING_KEYS` and the
`key_id` to `TRUSTED_LIST_SIGNING_KEY_IDS` in
`crates/trace-commons-contributor/src/witness/accepted_list.rs`, and update
`this_build_trusts_exactly_the_commons_list_signing_key`, which pins the set.
Ship that client release and let it reach devices before relying on the
list. (Version 1 was compiled in this way; see the table above.) Keep the PEM beside this runbook's notes for `sign` and
`verify`; it can be exported again at any time.

## Publish a list

1. Write the list. Measurements use the client pin spelling
   (`mrtd=<96 hex>,mrconfigid=<96 hex>`, the same strings as
   `expected_measurements`), never the certificate's `mrtd:...+mrconfigid:...`
   form. Name the commit each image was built from.

   ```json
   {
     "schema": "trace-commons.witness-accepted-list.v1",
     "version": 1,
     "issued_at": "2026-11-05T00:00:00Z",
     "witness_url": "<the witness base URL clients have configured>",
     "entries": [
       {
         "measurements": "mrtd=<96 hex>,mrconfigid=<96 hex>",
         "signing_address": "0x<40 hex>",
         "not_before": "2026-11-05T00:00:00Z",
         "not_after": "2027-11-05T00:00:00Z",
         "commit": "<commit>"
       }
     ]
   }
   ```

   `version` must be higher than every version published before; clients
   ignore anything else. At most 32 entries.

2. Build the payload. This parses every entry with the client's own parsers,
   so a list no client would accept never reaches the key:

   ```bash
   trace-commons-witness-list payload --input list.json --output payload.json
   ```

3. Sign it. The tool runs `gcloud kms asymmetric-sign` over the payload bytes
   (no digest: Ed25519 signs the message itself), checks the signature
   against the public key, and only then writes the served document:

   ```bash
   trace-commons-witness-list sign \
     --payload payload.json --public-key-pem list-key-v1.pem \
     --location <LOCATION> --keyring <KEYRING> --key <KEY> --key-version 1 \
     --project <PROJECT> --output witness-accepted-list.json
   ```

4. Verify it as a client would:

   ```bash
   trace-commons-witness-list verify --signed witness-accepted-list.json \
     --public-key-pem list-key-v1.pem --witness-url <witness base URL>
   ```

5. Install it on the ingest host at the path in
   `TRACE_COMMONS_WITNESS_ACCEPTED_LIST_PATH` (see
   [env-reference](./env-reference.md)). Copy to a temporary name beside it
   and `mv` into place, so a request never reads half a file. The file is read
   on every request: a new list needs no restart. Setting the variable the
   first time does.

6. Check what is served:

   ```bash
   curl -fsS <ingest origin>/v1/public/witness-accepted-list -o served.json
   cmp served.json witness-accepted-list.json
   ```

   Unset, the route answers 404 `witness_accepted_list_not_published`. A
   missing, unreadable or malformed file answers 503
   `witness_accepted_list_unavailable` and is never served.

If KMS is unavailable, no new list is published and clients keep the one they
have. There is no fallback signer.

## Witness redeploy (rollover)

1. Publish a list holding the **old and the new** measurement (new `version`).
   Wait until devices have it: they refresh within about a day, or at their
   next start.
2. Deploy the new witness image. Devices accept it through the new entry.
   Record the commit, image digest and resulting measurement in the deploy's
   operator notes.
3. Publish a list holding only the new measurement, or let the old entry's
   `not_after` pass.

A device that has not taken the list yet refuses the new image with
`witness_expected_measurement`, fetches the list (at most once per ten
minutes), and tries again if a newer list arrived.

## Multiple witness URLs during a hostname transition

The signed V1 payload still names exactly one `witness_url`. The unqualified
`GET /v1/public/witness-accepted-list` continues to serve
`TRACE_COMMONS_WITNESS_ACCEPTED_LIST_PATH` for existing clients.

Clients with hostname-transition support send their configured witness URL as
an encoded `witness_url` query parameter on that same public ingest route.
Configure `TRACE_COMMONS_WITNESS_ACCEPTED_LIST_DIRECTORY` with URL-specific
signed documents. Each filename is the lowercase SHA-256 hex of the exact URL
with trailing `/` characters removed, followed by `.json`. Do not otherwise
normalize or reserialize the URL. For example:

```python
import hashlib
url = "https://witness.tracecommons.ai"
filename = hashlib.sha256(url.rstrip("/").encode("utf-8")).hexdigest() + ".json"
```

The server never fetches a URL supplied by a request. It selects a bounded
local signed file and returns its bytes unchanged. A missing URL-specific
file may fall back to the legacy file only if the signed payload names that
same URL. A present unreadable, malformed or wrong-URL file fails closed.
Clients still verify the signature and signed URL themselves. An old server
ignores the query and continues to work for clients configured with the
legacy URL; a client configured with another URL refuses that legacy list.

**Versions are global on each device, even across URL changes.** Keep the
legacy version fixed during the initial transition and give the target URL a
strictly higher version than every legacy version already published. Equal
versions for two URL variants do not support migration: a client that has
already stored that version ignores the second one. Coordinate later
publications and rollbacks with the same rule; rolling back the URL requires
a newly signed, higher-version document for that URL, never an older backup.

Release order:

1. Deploy the URL-aware ingest route while retaining the existing legacy file.
2. Stage and verify the target URL's signed list. Set the directory variable
   and restart ingest once; subsequent atomic file replacements need no restart.
3. Release query-aware clients. Verify the legacy route and both explicit URL
   requests independently, including signatures and strict URL matching.
4. Only after the new domain is stable and eligible clients have updated, move
   legacy provisioning and connected-inference publishing surfaces. URL changes
   still require contributor confirmation and invalidate old grants. Retain
   current measurement pins during that ceremony; the prior URL's cached list
   cannot authorize the new URL.
5. Keep the old URL and its list available for contributors who have not moved.

For the 2026-11-05 rollout, legacy V2 remains served. Custom-host V3 is signed
and staged at
`/etc/tracecommons/witness/accepted-lists/e2b3d84969c4681419afdd98023a53db98edc87e4509dc5f0d1bea70bfd8fd6f.json`;
the directory is not enabled and discovery is unchanged. The custom-domain
gateway route remains blocked as recorded in
[the hostname runbook](witness-custom-domain.md).

## Rotating the list-signing key

1. Create a new key version (`gcloud kms keys versions create --key <KEY>
   --keyring <KEYRING> --location <LOCATION> --project <PROJECT>`) and export
   its public key as above.
2. Add its `public_key_hex` to `TRUSTED_LIST_SIGNING_KEYS` (and its key id to
   `TRUSTED_LIST_SIGNING_KEY_IDS`) beside the old one
   and ship that client release.
3. Until devices have it, sign each list with both versions:

   ```bash
   trace-commons-witness-list sign ... --key-version 1 --output signed-v1.json
   trace-commons-witness-list sign ... --key-version 2 \
     --public-key-pem list-key-v2.pem --append-to signed-v1.json \
     --output witness-accepted-list.json
   ```

4. Once devices have the new key, sign with the new version alone, then remove
   the old key from `TRUSTED_LIST_SIGNING_KEYS` in a later release and disable
   the old key version.

A client holds at most a few trusted keys; a list may carry up to four
signatures.

## Known limit

A list entry with a **different** signing address is honoured where the
client verifies a witness live (the attestation and the certificate it gets
back in the same exchange). A certificate the client stored earlier, from an
explicit witness review, is still checked against the sign-up address, so a
stored review from a re-addressed witness is treated as stale and asked
again. dstack derives the signing key from a stable app id, so an ordinary
image upgrade keeps the address.
