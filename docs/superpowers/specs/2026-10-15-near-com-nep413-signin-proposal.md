# Proposal to NEAR: a NEP-413-only sign-in endpoint on near.com

Date: 2026-10-15
Status: draft, for owner review before it is sent
Companion: [NEAR passkey signer design](2026-10-15-near-passkey-signer-design.md)

## What we are asking for

1. A near.com endpoint pair that issues a passkey challenge bound to one [NEP-413](https://github.com/near/NEPs/blob/master/neps/nep-0413.md) payload, and then signs exactly that payload with the user's account key. A session minted from one of these challenges can do nothing else.
2. An entry in near.com's `apple-app-site-association` that lists the signer app (iqlusion's team ID, bundle ID to be confirmed) under `webcredentials.apps`. On Apple platforms, this makes the signer the only native app that can use a near.com passkey.
3. A repository, `nearai/near-signer`, owned by NEAR, with NEAR security reviewers as code owners for the caller registry and the release pipeline. iqlusion contributes and maintains it and signs releases with its Apple Developer ID.

## Why

Apps in the NEAR ecosystem want users to sign in with their NEAR account from a native app. Today that means a browser round trip, or trusting each app with the passkey. The signer gives every app one reviewed path. Its passkey use is limited to sign-in by the OS, by this endpoint and by the signer, so NEAR reviews one small app and one endpoint, not every app that wants sign-in.

## Endpoint contract

### `POST /signin/nep413/challenge`

Request:

```json
{ "payload_hash": "<base64url sha256 of the borsh-serialized NEP-413 payload, tag included>", "account_hint": "alice.near" }
```

- `account_hint` is optional. It narrows the passkey picker when the signer already knows the account.

Response:

```json
{ "challenge_id": "<opaque>", "challenge": "<base64url WebAuthn challenge>", "expires_at": "<RFC 3339, at most 2 minutes ahead>" }
```

- near.com stores `payload_hash` against `challenge_id`.
- The challenge is single-use.

### `POST /signin/nep413/sign`

Request:

```json
{
  "challenge_id": "<opaque>",
  "assertion": { "credential_id": "...", "authenticator_data": "...", "client_data_json": "...", "signature": "...", "user_handle": "..." },
  "payload": { "message": "Sign in to NEAR AI Cloud", "recipient": "cloud.near.ai", "nonce": "<base64 32 bytes>", "callback_url": null }
}
```

near.com must, in this order:

1. Verify the WebAuthn assertion: `rpId` `near.com`, origin `https://near.com`, user verification performed, and the challenge equal to the one issued for `challenge_id`, which must be unexpired and unused.
2. Serialize `payload` as NEP-413 (borsh, with the 2^31 + 413 tag) and require its SHA-256 to equal the stored `payload_hash`.
3. Check the payload is well formed:
   - `nonce` is 32 bytes;
   - `recipient` is a domain name;
   - `message` is at most 1,024 characters and has no control characters.
4. Sign the serialized payload with a full-access key of the account that owns the credential.
5. Mark the challenge used.

Response:

```json
{ "account_id": "alice.near", "public_key": "ed25519:...", "signature": "<base64 64 bytes>" }
```

The session created by these two calls can only reach `/signin/nep413/sign`, and only once. It cannot sign transactions, add or remove keys, or reach any other near.com route.

### Errors

Every error is a fixed label with no account or payload details:

| Label | Meaning |
|---|---|
| `challenge_expired` | The challenge is past its expiry. |
| `challenge_used` | The challenge was already used. |
| `assertion_invalid` | The WebAuthn assertion does not verify. |
| `payload_mismatch` | The payload does not hash to the bound `payload_hash`. |
| `payload_rejected` | The payload is malformed. |
| `account_unavailable` | The credential's account cannot sign. |

## Why this cannot sign anything but a sign-in

- **The OS** lets only the signer use a near.com passkey, through the AASA entry.
- **This endpoint** binds every challenge to one NEP-413 payload hash, before the user ever sees a passkey prompt. A session can only sign that payload.
- **NEP-413's tag** means the signature can never be valid as a NEAR transaction.
- **The signer** shows the user the verified calling app, the recipient and the message before the passkey runs. It refuses callers and recipients that are not in its reviewed registry.

## What we need from NEAR

1. Agreement on this contract, or the changes you want.
2. A staging deployment of the endpoint and a staging AASA entry for a staging build of the signer.
3. The `nearai/near-signer` repository and the code-owner rules for it.
4. A security review of the signer, its release pipeline and this endpoint before production.

## Open questions for NEAR

1. Which key signs: a full-access key, or a dedicated sign-in key on the account? A dedicated function-call key cannot sign NEP-413 on its own. Whichever key signs is the one the signer checks on chain.
2. Whether near.com wants to record or show which app requested each sign-in (the signer could pass its verified caller identity as an extra, unsigned field).
3. The Windows plan. We believe Windows has no OS-level equivalent of the AASA restriction. If so, a Windows signer relies on this endpoint and the signer's checks alone.
