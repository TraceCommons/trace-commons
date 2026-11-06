# Publish witness domains and measurements without another app release

Clients with signed-endpoint support can automatically move to a new witness domain or compatible CVM deployment. Publish a higher-version signed configuration containing the domain and its attestation pins. Clients retain their enrollment identity and existing grants, verify the configuration, then verify the selected destination's fresh attestation before sending raw data.

This capability needs one initial client release. Older clients keep using the existing URL and V1 accepted-measurements list. A new attestation technology or incompatible witness API still needs client implementation work.

## The document

`GET /v1/public/witness-configuration` on ingest serves one signed document, using the same payload/signatures envelope and owner-controlled KMS key as the [V1 accepted list](witness-accepted-list.md). Its payload is:

```json
{
  "schema": "trace-commons.witness-configuration.v1",
  "version": 1,
  "issued_at": "2026-11-05T00:00:00Z",
  "enrollment_urls": [
    "https://old-witness.example",
    "https://witness.tracecommons.ai"
  ],
  "endpoints": [
    {
      "url": "https://witness.tracecommons.ai",
      "entries": [
        {
          "measurements": "mrtd=<96 hex>,mrconfigid=<96 hex>",
          "signing_address": "0x<40 hex>",
          "not_before": "2026-11-05T00:00:00Z",
          "not_after": "2027-11-05T00:00:00Z",
          "commit": "<image source commit>"
        }
      ]
    }
  ]
}
```

The example contains placeholders and cannot be signed as-is.

`enrollment_urls` retains the identities devices enrolled with, including retired URLs. Automatic routing applies only when the device's configured witness URL is both a compiled Commons enrollment identity and a signed alias. The compiled identities are:

- `https://f1654b0beac2ac2afae4235ee3d907096cd8f3de-8088.dstack-pha-prod9.phala.network`
- `https://witness.tracecommons.ai`

Trailing slashes are ignored for identity matching. A signer-supplied alias cannot add a self-hosted witness to this delegation. Custom configured witnesses ignore this configuration and its local rollback state, including invalid cache files; their original same-URL V1 accepted-measurements support remains available. `endpoints` lists active destinations in preference order. Each destination has its own signing-address/measurement pairs; a pin at one destination never authorizes another. Keep retired enrollment aliases so existing devices can still recognize future configurations, but remove retired active endpoints.

URLs must be strict HTTPS without credentials, query, fragment, whitespace, controls or backslashes, at most 2048 bytes. Only trailing slashes are ignored for matching; do not otherwise normalize URL text. Limits are 32 enrollment aliases, 8 endpoints, 32 entries per endpoint and 64 KiB for the whole served document. Unknown fields and malformed pins are refused. An empty active endpoint set explicitly withdraws service; a matching configuration with no currently valid candidates refuses raw sends.

## Publish

1. Verify each new destination's TLS, witness API, nonce-bound quote, signing address and measured deployment. Domain publication does not make an unattested host trusted. Add the corresponding server-side certificate acceptance pins as part of the deployment; this public configuration changes client routing, not ingest certificate verification.
2. Write the payload with a version strictly above every configuration version previously published. Versions belong to this configuration stream, independently of the legacy V1 list versions. Rollback also publishes a higher version.
3. Validate and canonicalize:

   ```bash
   trace-commons-witness-list configuration-payload \
     --input configuration.json --output configuration-payload.json
   ```

4. Sign with the existing owner-only KMS key:

   ```bash
   trace-commons-witness-list configuration-sign \
     --payload configuration-payload.json \
     --public-key-pem list-key-v1.pem \
     --location <LOCATION> --keyring <KEYRING> --key <KEY> --key-version 1 \
     --project <PROJECT> --output witness-configuration.json
   ```

   The tool verifies the signature locally before writing. It never stores the private key. For list-key rotation, `--append-to` adds another signature over the same payload; clients must already contain the new public key before relying on it alone.

5. Verify as a client would, including enrollment membership:

   ```bash
   trace-commons-witness-list configuration-verify \
     --signed witness-configuration.json --public-key-pem list-key-v1.pem \
     --witness-url <a retained enrollment URL>
   ```

6. Atomically install the exact signed bytes at the path configured by `TRACE_COMMONS_WITNESS_CONFIGURATION_PATH`. Setting that variable initially requires an ingest restart. The route reads the file on every request, so later atomic replacements do not. Check the public route returns byte-identical signed bytes and verify them again.
7. Release the initial supported client before relying on automatic moves. Clients refresh at startup, about daily and after measurement refusal. Keep the old endpoint and V1 lists for older clients. Once supported clients are deployed, later higher-version signed configurations can add or prefer new domains without changing the app.

No configured file means fixed 404; a missing, malformed, unsigned or oversized configured file means fixed 503. The server neither signs nor fetches the listed domains. Publishing infrastructure does not need the signing key.

## Client safety and retry behavior

The client verifies cached bytes on read, retains the last verified configuration on fetch failure and serializes cache and rollback-state reads/writes with the same process-shared file lock. `witness-configuration-high-water.json` separately stores the highest configuration version and the SHA-256 of its exact signed payload. A checked, monotonic local cache generation makes actual installations and same-payload repairs visible to the supervisor even when another process repairs the cache; it releases measurement holds without changing signed policy. Installation durably writes this watermark before replacing the cache. A crash between writes fails closed until the matching publication completes; a newer valid signed document can repair a damaged cache or replace one signed only by a removed key. The same version can repair or be re-signed only when its payload exactly matches the watermark; it cannot change policy. Lower versions and different payloads at the same version are refused.

A missing or damaged cache with a watermark never restores enrollment or V1 pins for Commons routing. An invalid watermark also fails closed. For migration from the unreleased cache-only implementation, a missing watermark is initialized only from a fully verified existing cache; an invalid prior cache cannot establish a replacement version floor. Keep the watermark when repairing cache files. These checks protect retained state, not a fresh state directory after deliberate deletion of both files. A matching configuration is authoritative: expired or withdrawn endpoint entries do not fall back to older signup or V1 pins. A verified configuration for unrelated enrollment URLs cannot reroute a custom witness.

For Commons onboarding and connected-inference discovery, continue publishing one of the compiled stable enrollment identities above even after its active deployment retires. Keep its signup pins valid and preserve it in every later signed configuration. Put future provider domains in the signed `endpoints` list; do not make new clients enroll with an uncompiled provider URL. Existing stored enrollment identities stay unchanged. Enrollment validates the published witness URL, pins and administrator policy without requiring a live connection to that retired alias; actual witness traffic uses the freshly attested signed destination.

The stored enrollment settings and grants remain unchanged during signed runtime routing updates. Manual changes to configured witness identity still invalidate grants. Existing nonempty administrator host restrictions remain absolute; empty policies retain their existing unset-equivalent meaning. Only the chosen witness transport gets a derived signed host when no enforcing policy exists; ingestion, receipts and other clients keep their own host restrictions.

Within one uploader/review invocation, fallback is allowed only while probing candidates, before raw data is sent. A POST, busy response, job/poll error or ambiguous completion never triggers another endpoint inside that invocation. A later separately scheduled attempt may resolve a newer signed configuration and must attest again; this feature does not add persistent per-session endpoint affinity. Candidate validity and configuration changes are checked again before handing raw data to the transport.

## Current rollout

The `witness.tracecommons.ai` gateway route is still failing as recorded in [the hostname runbook](witness-custom-domain.md). Do not make it the only active destination. The initial configuration should retain the healthy direct Phala endpoint; both the original and future stable URL can be enrollment aliases. Signed configuration publication, code deployment, the initial app release and actual traffic cutover are separate steps.

Initial configuration version 1 is published by pilot ingest build `68555db70`
at the public configuration route. Its signature verifies for both enrollment
aliases, and its served bytes match
`/etc/tracecommons/witness/witness-configuration.json`
(SHA-256 `d94720ab68815f7f71acaba5ce90ae0fd51267c0a2e6549043d47b03df0f2212`).
Only the healthy legacy Phala endpoint is active; the stable custom hostname
is an enrollment alias. The initial supporting client release is a separate
step. Keep Phala active until runtime destination disclosure and consent copy
are updated before a first signed host move.
