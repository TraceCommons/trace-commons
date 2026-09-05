# Verifying the receipt key that is actually attested

**Status:** approved design. Trust boundary decided 2026-09-05.

`trace-commons-attestation` verifies NEAR AI receipts by EIP-191 secp256k1
recovery, so it fetches them with `signing_algo=ecdsa`. Measured live, that
recovers `0x614bc66ff0407dbb70b9c7ca1f5e983e4a02c921` — **a key that appears
nowhere in NEAR AI's attestation report**.

So a verified receipt currently proves NEAR AI produced it. It does not
prove any enclave did.

## What the attestation report actually contains

`GET /v1/attestation/report?model=..&nonce=..`:

| Key | Algo | Attested |
|---|---|---|
| `cb6fc58f…` | ed25519 | **Yes.** Gateway. `report_data == signing_address ‖ request_nonce` inside a TDX quote, with a caller-supplied nonce. |
| `0xe5d0fec4…` | ecdsa | Yes. The MODEL enclave — `dstack-nvidia-0.5.5`, TDX quote plus NVIDIA HOPPER evidence. |
| `0x614bc66f…` | ecdsa | **No.** The key we verify. Absent from the report. |

The same ECDSA signer appeared across two chat ids and two models, so it is
one gateway-level key rather than a per-request one. It is simply not
attested.

## The decision

**The gateway is the trust boundary.** The report supports treating it as a
real one: `ohttp_key_config` and `ohttp_attestation` are signed by the
gateway's ed25519 key, so the gateway-to-model hop is Oblivious HTTP rather
than an unprotected internal call.

Accepting that does **not** accept the ECDSA key, and the two are easy to
conflate. `0x614bc66f…` is not attested *as the gateway* either. The attested
gateway key is the ed25519 one.

**So: fetch and verify the ed25519 receipt.** Then a verified receipt means
"signed by a key committed inside the gateway's TDX quote", which is exactly
the claim now accepted as sufficient.

## Why this makes the primitive stronger, not just different

The ECDSA path *recovers* a signer from a signature and compares it to a
claimed address. Recovery answers "some key produced this", and the
comparison is what turns it into "that key did" — a step that has to be
right, and which `ReceiptError::SignerMismatch` exists to catch.

Ed25519 verifies a signature **against a key you already hold**. There is no
recovered value to mis-compare. Given the attested address from the report,
verification is a direct yes or no.

## Scope

1. **Ed25519 receipt verification** in `trace-commons-attestation`, alongside
   the existing EIP-191 path rather than replacing it. Both forms exist on
   the wire and old receipts do not become unverifiable.
2. **Switch the fetch** in `trace-commons-contributor`'s
   `routing/receipt.rs` to `signing_algo=ed25519`.
3. **An optional check that the signer matches an attestation-report
   address.** Optional because it requires a second network call and a
   policy about how fresh a report must be; the verification itself is
   useful without it.

### Dependency

`ring = "0.17"` is already a **direct** dependency of
`trace-commons-contributor` and `trace-commons-server`, and already in
`trace-commons-attestation`'s transitive graph. Adding it there as a direct
dependency adds **no packages** — the same situation as the `receipt`
feature, whose manifest comment already records that it saves none. The GTK
vendored flatpak source set is therefore unaffected.

### Not in scope

- **Verifying the TDX quote itself.** Nothing in our code path checks the
  quote; we would be trusting the report's self-description. Closing that
  needs `dcap-qvl` and a collateral fetch, and it is a larger dependency and
  policy question. Until it lands, an attestation-report address is a claim
  by NEAR AI, not a proof.
- **Binding a receipt to the model enclave.** The model has its own quote and
  its own key, and nothing observed binds a receipt to it. With the gateway
  as the trust boundary this is not required, but it is why a receipt cannot
  say "this model ran it".

## What a verified receipt will then claim

That NEAR AI's gateway — running in a TDX enclave whose quote commits to the
signing key — produced this response over these exact request bytes.

It will still not say the trace is genuine, that the model enclave served it,
or that unattested turns did not occur.

## Open questions

- **Is `0x614bc66f…` attested anywhere else?** It was absent from one report
  for one model. Worth one question to NEAR AI before concluding it is
  unattested by design rather than by omission.
- **How fresh must an attestation report be** for its address to be trusted?
  The nonce makes a single fetch fresh; nothing says how long that lasts.
- **Does the ed25519 signer rotate?** If it does, a pinned address breaks and
  the report has to be re-fetched on a schedule.

## Related

`2026-09-04-attested-inference-release-design.md` — the system this verifies
for, and the limits it currently states.
