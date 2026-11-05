# Stable witness domain

Owner approved establishing witness.tracecommons.ai so a later compatible CVM-host move does not change the app endpoint.

Use DNS-only Cloudflare CNAME and dstack instance-routing TXT records, with digest-pinned dstack-ingress 2.8 terminating TLS inside the existing CVM. Choose tls-alpn-01 so the guest needs no DNS credential. Preserve the running witness image, app id, sealed API key, results volume and old direct Phala URL. Retain public logs/sysinfo off. Publish the new attested measurement for old clients and ingest before relying on it. Publish account-bound CAA and verify the served leaf against attestation evidence.

A plain CNAME alone cannot supply a valid TLS certificate; an external HTTP proxy adds an unapproved raw-transcript recipient. Therefore use in-CVM ingress.

Initially leave discovery and the old URL-scoped signed list on the old hostname. V1 lists cover one URL, so discovery moves only after URL-specific list delivery is available; existing grants require explicit renewed consent when URL changes. No app-update-free transition of already configured URLs is claimed. Future compatible hosts retain the stable hostname and roll measurements/signers through signed lists. Keep list versions monotonically increasing on rollback.
