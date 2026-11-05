# Stable witness hostname

**Status: deployed but not ready for client discovery.** The custom hostname initially passed HTTPS and a synthetic job, then began consistently failing TLS before reaching the ingress. The existing Phala endpoint remains healthy and discovery still uses it. The remaining blocker is the gateway custom-SNI route; do not treat DNS or a previously valid certificate as an availability check.

`https://witness.tracecommons.ai` reaches the redaction witness through Phala's TLS passthrough gateway. Cloudflare supplies DNS only (`proxied: false`); the measured `ingress` container terminates TLS inside the witness CVM and forwards TCP to `witness:8088`. Never enable the Cloudflare HTTP proxy on this record: witness requests contain raw transcripts.

The deployment uses digest-pinned dstack-ingress 2.8 with `CHALLENGE_TYPE=tls-alpn-01`. Its certificate and ACME account persist in `witness-cert-data`; certificate evidence persists in `witness-tls-evidences`. The CVM has no DNS API credential. ACME renewals require the hostname and TXT routing to remain correct and port 443 reachable. The original direct Phala URL remains available without redirects for existing contributors.

## Deployment recorded 2026-11-05

- CVM: `8b8e6543-9743-41fc-ac05-a6b414888d5e`; app `f1654b0beac2ac2afae4235ee3d907096cd8f3de`.
- Current instance: `aebf544394423a3ed2b83d924387c34be7a432c1`.
- Witness image remains the running `6abd2d127` build: `ghcr.io/tracecommons/trace-commons-witness@sha256:af99d33f29d2fc9401062f0b3c3ff8bd3304c6fe76d9b8871460e1f6b9e65546`.
- Ingress: `ghcr.io/dstack-tee/dstack-ingress:2.8@sha256:60e682449534715a2f2e5b72326ca4229209c8e26c931c6f34d27e02200ac086`, upstream source `d43a8e5c6a873e9e85ca67941cd08c988faa36a8`.
- Authoritative compose hash from Phala and live quote: `2c35c88d975ebe3685b2087304a665b56630a112c81eca9227b07afea460c1b8`.
- MRTD remains `f06dfda6dce1cf904d4e2bab1dc370634cf95cefa2ceb2de2eee127c9382698090d7a4a13e14c536ec6c9c3c8fa87077`.
- MRCONFIGID: `012c35c88d975ebe3685b2087304a665b56630a112c81eca9227b07afea460c1b8000000000000000000000000000000`.
- Signing address remains `0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798`.
- Public logs and sysinfo remain disabled. Sealed classifier credentials and results volume were retained.
- Pilot ingest accepts the new measurement in both its own certificate pins and legacy provisioning pins. Signed accepted list version 2 retains the original URL and previous entries, and adds the new measurement. The public served list was signature-verified.

DNS records (TTL 60):

| Type | Name | Value |
| --- | --- | --- |
| CNAME, DNS only | `witness.tracecommons.ai` | `gateway.dstack-pha-prod9.phala.network` |
| TXT | `_dstack-app-address.witness.tracecommons.ai` | `aebf544394423a3ed2b83d924387c34be7a432c1:443` |
| CAA | `witness.tracecommons.ai` | `0 issue "letsencrypt.org;validationmethods=tls-alpn-01;accounturi=https://acme-v02.api.letsencrypt.org/acme/acct/3961162402"` |

TLS-ALPN routing names the **instance**, not the app. Update TXT after an instance replacement. Losing the certificate volume creates a new ACME account; update CAA to its verified account URI before issuance. Keep the old account's records while it still serves, then remove obsolete issuance authorization.

## Verification

1. Fetch `/v1/diagnostics` over ordinary verified HTTPS on both URLs and compare signing address and measurement.
2. Fetch `/evidences/sha256sum.txt`, `acme-account.json`, `cert-witness.tracecommons.ai.pem` and `quote.json` over verified HTTPS. Verify the manifest's file hashes. Verify the TDX quote signature with Intel collateral or Phala's attestation verifier. Verify its report data binds SHA-256 of `sha256sum.txt`, and the quote's configuration measurement matches the deployed compose hash. Do not trust the echoed `report_data` field alone.
3. Compare the certificate on the actual TLS connection to the first certificate in `cert-witness.tracecommons.ai.pem`.
4. Check CAA through multiple public resolvers; its account URI must match the published ACME account. Check for unexpected certificates through Certificate Transparency.
5. Run a synthetic attestation, job submission and fetch on both endpoints. This checks the witness protocol; it does not prove a contributor has migrated or an ingest submission has been admitted.

Observed on 2026-11-05: valid Let's Encrypt leaf for the custom hostname, expiry 2027-02-03; evidence hashes and served leaf match; report-data and configuration measurement bindings match; Phala verification API reports `quote.verified: true`. A fresh nonce-bound witness quote also verified with the unchanged pinned signing address and deployed measurement. CAA was confirmed through Cloudflare and Google public DNS. Synthetic job submission/fetch succeeded on both hostnames with the new measurement and unchanged signer. The larger custom-host fixture completed after 55.5 seconds, returned the v2 pipeline and medium residual-risk verdict, and removed the synthetic host marker. Subsequent small-fixture attempts and repeated diagnostics checks failed TLS handshakes before HTTP. Python failed 12/12 and curl failed 5/5 on the custom hostname while corresponding legacy checks passed. The failure reproduced from the GCP pilot host. Direct instance TLS passthrough and nested certificate-validated TLS through the gateway both reached the ingress; nested checks passed 3/3. Public Cloudflare/Google DNS and both authoritative nameservers returned the expected records. Brief CAA removal, app-ID TXT routing and a legacy TXT alias did not resolve the failure; each test was reverted. This isolates the blocker to the gateway custom-domain path, without establishing its internal cause.

## Gateway routing reproduction

These read-only checks isolate the current provider blocker. Both connect to
`66.220.6.109:443`. The first uses the public custom SNI and currently fails
before HTTP. The second selects the known instance's TLS passthrough route
while still validating the certificate against the custom hostname:

```bash
openssl s_client -connect 66.220.6.109:443 \
  -servername witness.tracecommons.ai \
  -verify_hostname witness.tracecommons.ai -verify_return_error </dev/null

openssl s_client -connect 66.220.6.109:443 \
  -servername aebf544394423a3ed2b83d924387c34be7a432c1-443s.dstack-pha-prod9.phala.network \
  -verify_hostname witness.tracecommons.ai -verify_return_error </dev/null
```

Use the verifier machine's normal CA trust bundle. A custom-SNI failure with
a successful direct passthrough handshake demonstrates that the ingress and
certificate work; it does not identify which gateway resolver or routing
component failed. The provider must diagnose that boundary before cutover.
The reproduction is prepared here; no external support message was sent.

## Repair investigation, 2026-11-05

The gateway's published image and informational RPC identify dstack-gateway
0.6.1 with the expected `_dstack-app-address` prefix. That version uses
TTL-aware lookup caching and a ten-second maximum negative-cache lifetime;
the persistent custom-SNI failure is not explained by normal negative caching.
The gateway supports `core.proxy.app_address_dns_servers` as an operator-side
resolver override, but the tenant API has no resolver update or cache-flush
operation. Gateway logs are needed to distinguish a lookup error from an
address-selection error before applying a provider-side correction.

The tenant network-config API can change gateway URLs, but this guest is on
dstack 0.5.9: its URL list stops after the first successful gateway, and a new
registration replaces its single WireGuard interface. Appending another URL
would not exercise it while prod9 registers successfully; putting another
cluster first could strand the original prod9 URLs. Network config is copied
at boot, so changing the API configuration without restart does not refresh
that guest copy. No network configuration was changed in this investigation.

A [provider report](phala-custom-sni-report-2026-11-05.md) is prepared for
review; it has not been sent. See the [supported network API](https://docs.phala.com/api-reference/cvms/update-kmsgateway-urls),
[pinned guest implementation](https://github.com/Dstack-TEE/dstack/blob/v0.5.9/dstack-util/src/system_setup.rs),
and [gateway resolver implementation](https://github.com/Dstack-TEE/dstack/blob/gateway-v0.6.1/dstack/gateway/src/proxy/tls_passthough.rs).

## Client transition

Discovery still advertises the original Phala URL. Existing clients do not automatically adopt this hostname. Witness URL changes require explicit contributor confirmation and void the old automatic grants.

Accepted-list V1 has exactly one URL. Clients fetch one fixed ingest route and reject a list for any other URL. URL-specific query delivery and client support are implemented in this branch. Do not replace the legacy list or flip discovery until that server support is deployed, its client support is released and the custom-domain gateway fault is resolved. Custom-domain signed V3 is staged, not enabled, at `/etc/tracecommons/witness/accepted-lists/e2b3d84969c4681419afdd98023a53db98edc87e4509dc5f0d1bea70bfd8fd6f.json`. Keep legacy V2 and the old URL available during that transition. Both legacy provisioning and the connected-inference catalog need coordinated updates; an installed older catalog selection is not automatically replaced.

## Future host moves and rollback

Keep `witness.tracecommons.ai` stable. Verify the destination's attestation format and witness API are supported, prepublish its verified measurement/signing identity in the appropriate signed lists, and add its certificate pins to ingest. Deploy TLS inside the destination CVM, verify evidence and synthetic traffic, then change DNS routing and the CAA account pin. Retain the previous endpoint while work in flight drains. This supports compatible host moves without an app endpoint change; a new attestation technology may still need client changes.

List versions must always increase, including during rollback. Never restore an older signed file and expect clients to accept it. For deployment rollback, retain and republish the old measurement, then restore the recorded previous full compose/manifest. Reverting compose can itself produce a different hash if the CLI's generated metadata changes: read the resulting instance and live quote before relying on pins. Keep DNS on a serving endpoint; do not point the custom hostname to an HTTP listener without TLS.

**Phala CLI gotcha:** on this Phala-managed KMS CVM, `phala deploy --prepare-only` applied the update directly and returned `prepare_only: false`; it is only a preparation gate for on-chain KMS. Do not use it as a dry run. Read the installed CLI behavior before any future deployment. Updates without `-e` retained the existing sealed environment in this deployment.

References: [Phala custom domains](https://docs.phala.com/phala-cloud/networking/setup-custom-domain), [dstack-ingress 2.8 release](https://github.com/Dstack-TEE/dstack-examples/releases/tag/dstack-ingress-v2.8), [signed-list operation](witness-accepted-list.md).
