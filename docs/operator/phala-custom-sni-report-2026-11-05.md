## Custom-domain TLS passthrough fails on prod9 while direct instance passthrough works

We are attaching `witness.tracecommons.ai` to an existing CVM with in-CVM TLS termination. The hostname initially completed HTTPS and a synthetic job, then started consistently closing the connection during the TLS handshake, before HTTP.

### Deployment

- CVM UUID: `8b8e6543-9743-41fc-ac05-a6b414888d5e`
- App: `f1654b0beac2ac2afae4235ee3d907096cd8f3de`
- Instance: `aebf544394423a3ed2b83d924387c34be7a432c1`
- Node/gateway: prod9; gateway address `66.220.6.109`
- Gateway published image: `ghcr.io/dstack-tee/dstack-gateway:0.6.1@sha256:d99e156cb002591fe65716b0eed232ca4eea199d25f1fce55dd5274ba608ff32`
- Guest: dstack 0.5.9
- Ingress: digest-pinned dstack-ingress 2.8, TLS-ALPN-01, listener 443, backend witness:8088.
- Public logs and sysinfo remain disabled.

### DNS (TTL 60)

- DNS-only CNAME `witness.tracecommons.ai` → `gateway.dstack-pha-prod9.phala.network`
- TXT `_dstack-app-address.witness.tracecommons.ai` → `aebf544394423a3ed2b83d924387c34be7a432c1:443`
- CAA restricts Let's Encrypt TLS-ALPN-01 issuance to the ACME account held by the CVM.

Cloudflare/Google public resolvers and both authoritative nameservers return the expected records. The certificate for witness.tracecommons.ai verifies and expires 2027-02-03. Its served leaf, attestation evidence, quote signature and deployed measurement have been checked.

### Reproduction

Both commands use the same gateway IP, the same guest TLS listener, and verify the certificate against the custom hostname. Use the machine's normal CA bundle.

Fails before receiving the server certificate:

```bash
openssl s_client -connect 66.220.6.109:443 \
  -servername witness.tracecommons.ai \
  -verify_hostname witness.tracecommons.ai -verify_return_error </dev/null
```

Succeeds with TLS 1.3 and certificate verification OK:

```bash
openssl s_client -connect 66.220.6.109:443 \
  -servername aebf544394423a3ed2b83d924387c34be7a432c1-443s.dstack-pha-prod9.phala.network \
  -verify_hostname witness.tracecommons.ai -verify_return_error </dev/null
```

The failure also reproduces from a GCP host. The existing direct app-8088 HTTPS endpoint stays healthy. Direct passthrough proves the ingress and certificate work; custom-SNI resolution/routing is the differing path. Brief CAA removal, app-ID TXT routing, and a legacy TXT alias did not resolve it; those tests were reverted.

### Requested diagnosis

Please inspect prod9 gateway logs for the custom-SNI TXT lookup and resulting app-address selection. The published 0.6.1 implementation has a TTL-aware resolver with a ten-second negative-cache cap, so this persistent failure is unlikely to be ordinary negative caching.

If the gateway's system resolver is failing or filtering the lookup, the supported `core.proxy.app_address_dns_servers` setting allows explicit resolvers (for example 1.1.1.1:53 and 8.8.8.8:53); please validate connectivity/policy and apply an appropriate operator-side correction/reload. We cannot change the shared gateway's resolver through the CVM tenant API.

We have left client discovery on the original endpoint. We have not switched the CVM to another gateway: its current guest supports first-success gateway failover rather than simultaneous gateway clusters, so doing that could disrupt existing clients.
