# NEAR passkey signer: a separate app that can only sign you in

Date: 2026-10-15
Status: design, for owner review
Companion: [near.com NEP-413 sign-in proposal](2026-10-15-near-com-nep413-signin-proposal.md)

## Why

A near.com passkey controls the user's NEAR account. If Trace Commons, or any other app, could use that passkey directly, a compromise of that app could reach the account. Asking NEAR's security team to review all of Trace Commons before trusting it with the passkey is not practical.

Instead, one small app is allowed to use near.com passkeys. That app can do exactly one thing: sign a [NEP-413](https://github.com/near/NEPs/blob/master/neps/nep-0413.md) sign-in message the user has approved. Any app in the NEAR ecosystem can call it, and NEAR reviews only that app and the near.com endpoint it talks to.

## Decisions

| Decision | Choice |
|---|---|
| What the passkey controls | The user's NEAR account. The signer is held to a wallet-grade standard. |
| How near.com signs | The passkey logs in to near.com, and near.com's server signs on the account. "Sign-in" means near.com signing one NEP-413 payload. |
| Shape | A standalone app with its own identity, launched on demand, reached over local IPC. It is not part of the Trace Commons bundle. |
| First platform | macOS. Windows and Linux get separate follow-up specs. |
| Which callers may request which recipients | A reviewed registry, plus user consent for every sign-in. |
| Repository | `nearai/near-signer`, owned by NEAR. iqlusion contributes and maintains it. |
| Who signs releases | iqlusion's Apple Developer ID and Sparkle key. near.com's `apple-app-site-association` lists iqlusion's team ID with the signer's bundle ID. |
| License | MIT OR Apache-2.0, so ecosystem apps can depend on its client library. |

## How the passkey is restricted

Four independent layers. A sign-in signature reaches a caller only if all four agree.

1. **The OS: who can use the passkey.** On Apple platforms, a native app can use a passkey for `near.com` only if near.com's `apple-app-site-association` lists the app's App ID under `webcredentials.apps`. Only the signer is listed. Trace Commons and every other app cannot start a near.com passkey ceremony at all.
2. **near.com: what a sign-in session can do.** A WebAuthn assertion from a native app records the origin `https://near.com`, not which app made it. So near.com cannot restrict by caller. It restricts by purpose: sign-in challenges come from a dedicated endpoint, each is bound when issued to the hash of one NEP-413 payload, and the session it produces can sign that payload and nothing else (see the companion proposal).
3. **The signer: which caller, which recipient, and with whose consent.** Described below.
4. **NEP-413 itself.** The signed bytes begin with the NEP-413 tag (2^31 + 413), which a NEAR transaction can never start with. A sign-in signature cannot be replayed as a transaction, whatever a caller does with it.

## Components

### The signer app (`nearai/near-signer`)

- A macOS `.app` in `/Applications`, with bundle ID `com.iqlusion.near-signer` (to be confirmed). It is signed with iqlusion's Developer ID, notarized, and distributed as its own DMG with Sparkle updates.
- It is written entirely in Swift. Apple frameworks cover the UI, XPC, caller identity, the `AuthenticationServices` passkey ceremony, and CryptoKit's SHA-256 and Ed25519 for NEP-413 hashing and signature checks. Apart from Sparkle there is no third-party code and no FFI boundary in what NEAR reviews.
- No Dock icon and no menu bar item. It shows a window only to ask for consent, plus a small settings window for revoking remembered approvals.
- It talks to `near.com` and to one NEAR RPC endpoint, for signature verification, and to nothing else.
- It stores no NEAR key. Its only state is the user's remembered approvals.

### The registry

- `registry/callers.json` maps a verified caller (Apple team ID plus bundle ID) to the recipients it may request.
- It is compiled into the app, so changing it requires a new signed release. The signer never fetches a registry over the network.
- First entry: `KXSWJN7WY8.ai.tracecommons.shell` may request `cloud.near.ai` and `tracecommons.ai`.
- Another ecosystem app joins with a pull request to `registry/`. NEAR security reviewers are code owners of `registry/` and of the release pipeline.

### The caller library

A small, MIT OR Apache-2.0 Swift package (and a C header for Rust callers) that opens the XPC connection, sends a request and decodes the reply. Callers do not otherwise link the signer.

## Flow

1. **The caller asks.** It builds a NEP-413 request (`message`, `recipient`, a 32-byte `nonce`, and an optional `callbackUrl`) and sends it over XPC to the Mach service `com.iqlusion.near-signer.xpc`.
2. **The signer identifies the caller.** macOS supplies the caller's code signature with the connection, from which the signer reads its team ID and bundle ID. A caller cannot supply an identity of its own. A caller with an invalid signature, or one that is not in the registry, is refused (`caller_not_registered`) before any window appears.
3. **The signer checks the request.** It refuses (with the label in brackets):
   - a recipient not registered for this caller (`recipient_not_allowed`);
   - a nonce whose first 8 bytes are not a timestamp within the last 5 minutes (`nonce_stale`);
   - a message longer than 1,024 characters, or containing control characters (`message_rejected`).
4. **The signer asks the user.** Its own window, which the caller cannot draw over, shows the verified caller's name and team, the recipient, the account and the message, with Cancel and Sign in. A declined request returns `user_declined`. All strings in this window need owner approval before release.
5. **The signer gets a challenge.** It serializes the NEP-413 payload, hashes it, and asks near.com for a sign-in challenge bound to that hash.
6. **The passkey ceremony runs.** The signer runs the assertion with `rpId near.com`, which requires user verification (Touch ID or the device password) every time.
7. **near.com signs.** The signer sends the assertion and the payload. near.com checks both, signs the payload with the account's key and returns `{account_id, public_key, signature}`.
8. **The signer verifies before returning.** It checks the signature over its own serialization of the payload, and checks over NEAR RPC that `public_key` is a current key on `account_id`. It returns only `{account_id, public_key, signature}`. A failure returns `signature_invalid` or `near_refused`.

"Remember for {caller} and {recipient}" skips the consent window for later requests with that same pair. It never skips step 6: every sign-in still needs the passkey's user verification. Remembered pairs can be revoked in the signer's settings.

## Lifecycle on macOS

- On first launch, the signer registers its own LaunchAgent with `SMAppService.agent`. That agent hosts the XPC Mach service. launchd starts it when a caller connects, and it exits when idle.
- **Installing alongside Trace Commons.** Trace Commons' first run checks for the signer. If it is missing, Trace Commons offers to install it. Before installing it, Trace Commons verifies that the download is notarized and signed by iqlusion's team ID (pinned).
- Updates come from the signer's own Sparkle feed, never from Trace Commons.
- **Version compatibility.** The XPC interface is versioned. A caller that needs a newer version than the installed signer offers gets `signer_outdated`, and the caller can then offer an update.

## Failure handling

Every failure fails closed: the caller gets a label and no signature. The labels are:

| Label | Meaning |
|---|---|
| `caller_not_registered` | The caller's signature is invalid, or the caller is not in the registry. |
| `recipient_not_allowed` | The recipient is not registered for this caller. |
| `nonce_stale` | The nonce's timestamp is missing or older than 5 minutes. |
| `message_rejected` | The message is too long or contains control characters. |
| `user_declined` | The user cancelled the consent window. |
| `passkey_unavailable` | No near.com passkey, or the ceremony failed. |
| `near_refused` | near.com refused the challenge or the signing request. |
| `signature_invalid` | The returned signature does not verify, or the key is not on the account. |
| `signer_outdated` | The installed signer is older than the caller needs. |
| `busy` | The caller already has a request in flight, or is rate-limited. |

- A caller gets one request in flight at a time, and is rate-limited.
- No reply, error or log line contains the payload, the account, a token or a URL. Logs carry labels and hashes only.
- The signer never falls back to another signing path. Whether to offer one is the caller's choice. Trace Commons, for example, keeps its existing browser flow.

## Trace Commons changes

- `native_wallet_flow` in the daemon gains a second route, "native signer", beside the existing browser route (the near-connect page).
- On macOS the native signer route is used when the signer is installed and the setting is on. Otherwise the browser route runs as today.
- The NEP-413 payload Trace Commons already builds ("Sign in to NEAR AI Cloud", recipient `cloud.near.ai`, a 32-byte nonce starting with a timestamp) is sent to the signer unchanged. The server-side verification in `account_near.rs` and Cloud's verification do not change.
- First run gains the "install the signer" step described above.

## What NEAR's security review covers

1. The signer: its Swift code (UI, XPC, NEP-413 handling and verification) and the registry. Estimated at about 2,000 to 3,000 lines.
2. Its release pipeline: iqlusion's Developer ID and notarization, the Sparkle signing key, and the CI that builds releases.
3. near.com's NEP-413 sign-in endpoint (companion proposal).

Trace Commons is outside the review. A fully compromised Trace Commons can do no more than ask the user, through the signer's own window, to sign in to `cloud.near.ai` or `tracecommons.ai`.

## Testing

- **Rust core.**
  - NEP-413 serialization against the NEP's published test vectors.
  - The tag prefix.
  - Nonce window boundaries.
  - A table test that produces every refusal label.
- **Caller identity.** Two test-signed caller apps, one registered and one not. Using a real XPC connection, the unregistered caller is refused before any window appears.
- **Mock near.com.**
  - A payload changed after the challenge was issued is refused.
  - An expired challenge is refused.
  - A wrong signature, or a key not on the account, is refused.
- **End to end against near.com staging,** once the endpoint exists. Until then, an end-to-end run against the mock is the release gate.
- **No live passkey test is possible until near.com lists the signer in its AASA.** The rollout below accounts for that.

## Rollout

1. These two documents are reviewed by the owner and then sent to NEAR. Nothing goes out without the owner's go-ahead.
2. The signer is built against the mock endpoint, in `nearai/near-signer` (private for now).
3. The Trace Commons daemon route ships behind a setting that is off by default.
4. near.com staging adds the endpoint and an AASA entry for a staging build of the signer, and the end-to-end run is done against it.
5. NEAR's security review.
6. Release. The Trace Commons setting is turned on, and the browser route stays as the fallback.
7. Windows and Linux follow in separate specs.

## Open questions

1. **Windows.** I believe Windows has no OS mechanism that limits which apps can use a relying party's passkeys. If so, layer 1 does not exist there, and only layers 2 to 4 protect the account. This needs to be confirmed against Windows' WebAuthn API before the Windows spec. Linux has no platform passkeys.
2. **The final bundle ID** and whether the signer gets its own Sparkle feed host.
3. **Whether NEAR wants the registry review gate in addition to its code-owner review,** for example a two-person rule on `registry/`.
4. **Copy.** Every string in the consent window and the settings window needs owner approval.
