# Stable witness domain

Owner approved establishing witness.tracecommons.ai so a later compatible CVM-host move does not change the app endpoint.

Use DNS-only Cloudflare CNAME and dstack instance-routing TXT records, with digest-pinned dstack-ingress 2.8 terminating TLS inside the existing CVM. Choose tls-alpn-01 so the guest needs no DNS credential. Preserve the running witness image, app id, sealed API key, results volume and old direct Phala URL. Retain public logs/sysinfo off. Publish the new attested measurement for old clients and ingest before relying on it. Publish account-bound CAA and verify the served leaf against attestation evidence.

A plain CNAME alone cannot supply a valid TLS certificate; an external HTTP proxy adds an unapproved raw-transcript recipient. Therefore use in-CVM ingress.

Initially leave discovery and the old URL-scoped signed list on the old hostname. V1 lists cover one URL, so discovery moves only after URL-specific list delivery is available; existing grants require explicit renewed consent when URL changes. No app-update-free transition of already configured URLs is claimed. Future compatible hosts retain the stable hostname and roll measurements/signers through signed lists. Keep list versions monotonically increasing on rollback.

## Compatible client transition approved

Owner approved including compatible client transition after reviewing the initial domain constraint. Keep V1 signed payloads and the unqualified legacy route unchanged. Add optional `witness_url` query routing: the configured directory contains one signed document per lowercase SHA256 of URL with trailing slashes removed, suffixed `.json`. Query URLs are HTTPS, bounded to 2048 characters, and have no credentials, query or fragment. An explicit URL may fall back to the legacy file only if its payload names that URL. Missing routes are 404, malformed requests 400, corrupt or mismatched files 503. No arbitrary outbound fetch, raw filename, URL/path logging, auth or tenant predicate is added.

New clients encode their configured witness URL as a query parameter on the same fixed ingest origin and retain bounded reads, host policy, no redirects, signatures and exact signed URL matching. An older server ignores the query and legacy clients continue working; a mismatched list is refused. URL changes still require explicit confirmation and invalidate grants. The existing global version high-water mark stays intact: retain legacy V2 and publish custom V3 or higher; never use equal versions for a transition, and rollback lists must exceed every version already seen. Longer coexistence requires coordinated publication versions.

The custom-SNI gateway failure blocks discovery cutover, not implementation. Do not change either server publishing surface until the new server route and client support are released and the domain passes repeated HTTPS and job checks.
