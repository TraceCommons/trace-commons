# Key Rotation (GCP Cloud KMS)

Procedure for rotating the KEK that wraps every per-artifact DEK. v1 is
GCP-only; the operator-facing surface is `gcloud kms keys versions`.

## Why and when

- **Routine rotation.** Cloud KMS auto-rotation is the default. Whether
  it's monthly or quarterly, the procedure below applies on each
  promotion.
- **Suspected compromise.** Run immediately; skip the staging delay.
- **Adapter swap (Phase B).** Replacing the KEK provider entirely is a
  separate, larger procedure not covered here.

## Downtime profile

If you stage the new version with the running workload retaining decrypt
rights on the previous version, **rotation is zero-downtime**. The path
below preserves this invariant.

## Procedure

### 1. Stage a new key version

```sh
gcloud kms keys versions create \
  --location=<loc> \
  --keyring=<ring> \
  --key=<key> \
  --primary
```

`--primary` makes the new version the default for **encrypt** operations
immediately. Decrypt operations still resolve any version that wrapped
existing DEKs, so already-wrapped artifacts keep working.

### 2. Verify with the key-rotation drill

```sh
curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  "$BASE/v1/admin/key-rotation-drill" | jq
```

Expect: HTTP 200, `success: true`. The drill performs a wrap+unwrap round
trip plus a context-binding check.

### 3. Observe encrypt traffic for one hour

New uploads will use the new version for wrapping. Watch logs for:

- **Absent:** `KekContextMismatch` (the most common rotation failure;
  means `tenant_ctx` does not match the AAD that was used to wrap).
- **Absent:** `GcpKmsEncryptFailed` (workload identity lost
  encrypter role).
- **Present:** normal `KekWrap` events.

Also check `GET /v1/admin/operational-summary` — the
`kek_provider.active_key_version` reflection should advance.

### 4. Retire the old version (only after the claim-lifetime window)

The maximum claim lifetime + refresh window is the time during which the
**old** version must remain `enabled` for decrypt. With default upload
claim TTLs in low minutes, waiting ~1 hour is usually sufficient. If
unsure, leave both versions enabled longer.

```sh
gcloud kms keys versions disable \
  --location=<loc> \
  --keyring=<ring> \
  --key=<key> \
  <OLD_VERSION>
```

Do **not** `destroy` a key version unless you are explicitly purging old
ciphertext. Destruction is irreversible after the GCP soft-delete window.

## Rollback

If the new version is bad (e.g. workload identity didn't propagate the
new ACL), roll back **before** disabling the old version:

```sh
gcloud kms keys versions update \
  --location=<loc> --keyring=<ring> --key=<key> \
  <PREVIOUS_VERSION> --primary
```

This re-points encrypt operations at the previous version. New artifacts
are wrapped with the old version until you re-stage; existing artifacts
are unaffected.

## Watch list

| Symptom | Likely cause |
|---|---|
| `KekContextMismatch` after rotation | `tenant_ctx` ID changed or AAD bytes differ between encrypt and decrypt. Check that no env vars holding tenant labels changed in the same window. |
| `GcpKmsEncryptFailed` | Workload identity lost `cryptoKeyEncrypterDecrypter`. |
| `GcpKmsDecryptFailed` on old artifacts | Old version was disabled too early. Re-enable it. |
| `KekDowngradeRejected` | A non-`gcp_kms` provider crept into the config. |
| `KekProviderUnknown` / `KekProviderUnavailable` | Typo in `TRACE_COMMONS_KEK_PROVIDER` or KMS endpoint unreachable. |

See [`hash-only-logging.md`](hash-only-logging.md) for the full
classifier table.

## Script

[`scripts/operator/rotate-kek.sh`](../../scripts/operator/rotate-kek.sh)
runs steps 1–2 in one command. Step 4 (retiring the old version) is
**not** automated — the operator runs the disable command manually after
the observation window.
