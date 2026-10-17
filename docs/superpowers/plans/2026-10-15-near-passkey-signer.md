# NEAR Passkey Signer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a macOS signer app that is the only app allowed to use near.com passkeys and can only produce NEP-413 sign-in signatures, plus the Trace Commons route that uses it.

**Architecture:** The signer is a Swift package with three products:
- `NearSignerKit`, a library holding the protocol, the policy, the registry, crypto and orchestration. It has no UI.
- `NearSigner`, the app: the consent window, the XPC LaunchAgent and the passkey ceremony.
- `NearSignerClient`, the caller library.

Trace Commons' daemon gains a native-signer completion path beside the existing browser callback. Its macOS app calls `NearSignerClient` and hands the result to the daemon.

**Tech Stack:** Swift 6, macOS 14+, Foundation, CryptoKit, AuthenticationServices, ServiceManagement, Security, and Sparkle (pinned exactly, as in `macos/Package.swift`). On the Trace Commons side, Rust in `crates/trace-commons-contributor`.

**Spec:** `docs/superpowers/specs/2026-10-15-near-passkey-signer-design.md` and `docs/superpowers/specs/2026-10-15-near-com-nep413-signin-proposal.md`.

**One deviation from the spec, made in this plan's commit:** the spec's "Rust core behind a C ABI" becomes plain Swift using CryptoKit. CryptoKit provides SHA-256 and Ed25519 verification, so the audited app has no third-party code apart from Sparkle, and no FFI boundary. The spec is updated to match.

## Global Constraints

- **Repo.** `nearai/near-signer`, a private repository (owner-approved 2026-10-15), cloned at `~/code/near-signer`. Work goes on branches with pull requests; never push directly to `main`.
- **License.** `MIT OR Apache-2.0` (`LICENSE-MIT` and `LICENSE-APACHE`).
- **Platform.** macOS 14 or later, Swift tools 6.0. The app bundle ID is `com.iqlusion.near-signer`, and the XPC Mach service is `com.iqlusion.near-signer.xpc`.
- **Team ID.** `IQLUSION_TEAM_ID = "KXSWJN7WY8"`. This is the team that signs Trace Commons (`macos/entitlements.plist`). Confirm it is iqlusion's Developer ID team before Task 9.
- **Dependencies.** None besides Apple frameworks and Sparkle, which is pinned exactly. Any other dependency needs the owner's approval first.
- **NEP-413 digest.** `SHA256(u32le(2147484061) || borsh_string(message) || nonce[32] || borsh_string(recipient) || u8(callback present) || [borsh_string(callback)])`, where `borsh_string` is a u32le byte length followed by the UTF-8 bytes. This must match `signing_digest` in `crates/trace-commons-contributor/src/daemon/nearai_credential/near_wallet.rs:300-315`.
- **Request limits.**
  - nonce: exactly 32 bytes, with the first 8 bytes a big-endian Unix timestamp in milliseconds, at most 5 minutes old and at most 30 seconds in the future;
  - message: 1 to 1,024 Unicode scalars, none of them a control character;
  - recipient: a lowercase DNS name.
- **Refusal labels,** spelled exactly: `caller_not_registered`, `recipient_not_allowed`, `nonce_stale`, `message_rejected`, `user_declined`, `passkey_unavailable`, `near_refused`, `signature_invalid`, `signer_outdated`, `busy`.
- **Logging.** No reply, error or log line contains a payload, an account, a token or a URL. Use labels and hashes only (`os.Logger` with `privacy: .private` is not enough on its own: do not log those values at all).
- **Fail closed.** Every failure returns a label and no signature. There is no fallback path inside the signer.
- **Copy.** All UI strings are drafts marked `// COPY: needs owner approval` and live in one file, `Sources/NearSigner/Copy.swift`.
- **Trace Commons.** Follow that repo's `CLAUDE.md`:
  - `RUSTFLAGS="-D warnings"`;
  - its clippy allow-list;
  - no new dependencies;
  - fail closed;
  - label-only errors.

## Review Focus

1. **Remembered approval used across accounts.** If a user remembered Trace Commons → `cloud.near.ai` while signed in as `alice.near` and later picks `bob.near` at the passkey prompt, the result is a sign-in as `bob.near` without a consent window that named him. Expected: a remembered approval covers caller and recipient only, and the account that signed is always shown in a brief post-sign-in notice. Test in Task 7: `remembered_pair_still_reports_signed_account`.
2. **A request while the consent window is open.** A second request from the same caller while the window is up must get `busy` and must not queue a second window. Test in Task 7: `second_request_while_window_open_is_busy`.
3. **near.com returns a signature for a different payload.** For example, it echoes an old nonce. Expected: `signature_invalid`, because the signer verifies against its own serialization and never against anything near.com sends back. Test in Task 4: `signature_over_other_nonce_is_invalid`.
4. **The caller quits mid-ceremony.** Expected: the ceremony is cancelled, no window remains, and the next caller is not blocked. Test in Task 7: `caller_disconnect_cancels_and_releases`.
5. **The clock is skewed.** The caller's nonce timestamp comes from the caller's clock, which is the same machine as the signer's, but the machine clock may be wrong compared with near.com's. Expected: the signer judges freshness against its own clock only, and near.com's challenge expiry protects the server side. Test in Task 2: `future_nonce_beyond_30s_is_stale`.

---

## File Structure

`~/code/near-signer/`:

| Path | Responsibility |
|---|---|
| `Package.swift` | Products `NearSignerKit` (library), `NearSignerClient` (library) and `NearSigner` (executable app target). Sparkle is pinned exactly. |
| `Sources/NearSignerKit/NEP413.swift` | The payload type, the digest, and NEAR key and base58 decoding. |
| `Sources/NearSignerKit/Policy.swift` | Request validation, returning `Refusal`. |
| `Sources/NearSignerKit/Registry.swift` | Loads `registry/callers.json` from the bundle and looks up callers. |
| `Sources/NearSignerKit/NearCom.swift` | The `NearComAPI` protocol, the wire types and the URLSession implementation. |
| `Sources/NearSignerKit/Verify.swift` | Signature verification and the on-chain access-key check. |
| `Sources/NearSignerKit/SignInService.swift` | The orchestrator for flow steps 2 to 8, single-flight per caller, and rate limiting. |
| `Sources/NearSignerKit/Protocol.swift` | The XPC `@objc` protocol, the wire request and reply, and the protocol version. Shared with the client. |
| `Sources/NearSignerClient/NearSignerClient.swift` | The caller library. |
| `Sources/NearSigner/main.swift`, `XPCListener.swift`, `CallerIdentity.swift`, `Passkey.swift`, `ConsentWindow.swift`, `Approvals.swift`, `Copy.swift`, `AgentRegistration.swift` | The app. |
| `Resources/registry/callers.json` | The reviewed caller registry. |
| `Resources/LaunchAgents/com.iqlusion.near-signer.agent.plist` | The LaunchAgent. |
| `App/Info.plist`, `App/entitlements.plist` | `LSUIElement`, and `webcredentials:near.com`. |
| `scripts/make-release.sh`, `scripts/verify-entitlements.sh` | Build, sign, notarize and verify. |
| `Tests/NearSignerKitTests/*`, `Tests/NearSignerTests/*` | Tests. |
| `.github/CODEOWNERS` | Assigns `registry/` and `scripts/` to NEAR security, as a placeholder until the repo exists. |

In Trace Commons:

| Path | Change |
|---|---|
| `crates/trace-commons-contributor/src/daemon/nearai_credential/near_wallet.rs` | `verify_callback` is split, and a shared `verify_signed` is added. |
| `crates/trace-commons-contributor/src/daemon/native_flow.rs` | New wallet actions `native_payload` and `native_complete`. |
| `docs/contributor-daemon-ipc-v1_1.md` | Documents both actions. |
| `macos/Sources/TCShellCore/NearSignerRoute.swift` (new) | Decides between the signer and the browser route, and runs the signer route. |
| `macos/Sources/TraceCommonsApp/...` | First-run detection and the setting. |
| `macos/Package.swift` | Adds `NearSignerClient` as a local path dependency until the repo is published, then pinned exactly. |

---

### Task 1: NEP-413 payload and digest

**Files:**
- Create: `Package.swift`, `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`, `Sources/NearSignerKit/NEP413.swift`
- Test: `Tests/NearSignerKitTests/NEP413Tests.swift`

**Interfaces:**
- Produces:
  - `struct NEP413Payload: Codable, Equatable { let message: String; let recipient: String; let nonce: Data; let callbackURL: String? }`
  - `func nep413Digest(_ p: NEP413Payload) -> Data`, which returns 32 bytes
  - `func decodeNearPublicKey(_ s: String) -> Data?`, which accepts only the `ed25519:` prefix followed by base58 and returns 32 bytes or nil

- [ ] **Step 1: Write the failing tests.**
  - `digest_matches_trace_commons_vector`: build the payload `message "Sign in to NEAR AI Cloud"`, `recipient "cloud.near.ai"`, nonce bytes `0..<32`, no callback. Assert the hex digest equals the value computed by Trace Commons' `signing_digest` for the same input. Get the value by adding a temporary `#[test]` that prints it, or take it from the NEP-413 spec's published example if one exists; record where it came from in a comment.
  - `digest_starts_from_tag`: assert that the pre-hash encoding's first 4 bytes are `9D 01 00 80` (that is, 2147484061 as u32le). Expose `func nep413Encoding(_:) -> Data` as `internal` for this test.
  - `callback_changes_digest`: the same payload with `callbackURL: "https://x"` gives a different digest.
  - `decode_key_rejects`: assert nil for a missing prefix, `secp256k1:...`, the non-base58 character `0`, and a value that decodes to 31 bytes.
- [ ] **Step 2: Run `swift test --filter NEP413Tests`.** Expected: FAIL (does not compile).
- [ ] **Step 3: Implement.** Use CryptoKit's `SHA256`. Base58 is a hand-written decoder, mirroring `decode_public_key` at `near_wallet.rs:269`.
- [ ] **Step 4: Run `swift test --filter NEP413Tests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Serialize and hash NEP-413 sign-in payloads".

### Task 2: Request policy

**Files:**
- Create: `Sources/NearSignerKit/Policy.swift`
- Test: `Tests/NearSignerKitTests/PolicyTests.swift`

**Interfaces:**
- Consumes: `NEP413Payload` (Task 1).
- Produces:
  - `enum Refusal: String, Error { case caller_not_registered, recipient_not_allowed, nonce_stale, message_rejected, user_declined, passkey_unavailable, near_refused, signature_invalid, signer_outdated, busy }`
  - `func validate(_ p: NEP413Payload, allowedRecipients: Set<String>, now: Date) -> Refusal?`, which returns nil when the request is acceptable

- [ ] **Step 1: Write the failing table test `policy_table`.** One row per case, each with the expected result:

  | Input | Expected |
  |---|---|
  | Recipient not in the set | `recipient_not_allowed` |
  | A nonce that is not 32 bytes | `nonce_stale` |
  | A nonce timestamped now − 301 s | `nonce_stale` |
  | A nonce timestamped now − 299 s | nil |
  | A nonce timestamped now + 31 s (`future_nonce_beyond_30s_is_stale`) | `nonce_stale` |
  | An empty message | `message_rejected` |
  | A message of 1,025 scalars | `message_rejected` |
  | A message of exactly 1,024 scalars | nil |
  | A message containing `\u{0007}` | `message_rejected` |
  | A message containing `\n` | `message_rejected` |
  | A recipient `Cloud.near.ai` (uppercase) | `recipient_not_allowed` |
  | Valid | nil |

- [ ] **Step 2: Run `swift test --filter PolicyTests`.** Expected: FAIL.
- [ ] **Step 3: Implement `validate`.** Check the recipient first, then the nonce, then the message. The order fixes which label is returned when several checks fail, so the table must assert it.
- [ ] **Step 4: Run `swift test --filter PolicyTests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Refuse sign-in requests outside the signer's policy".

### Task 3: Caller registry

**Files:**
- Create: `Sources/NearSignerKit/Registry.swift`, `Resources/registry/callers.json`, `.github/CODEOWNERS`
- Test: `Tests/NearSignerKitTests/RegistryTests.swift`

**Interfaces:**
- Produces:
  - `struct CallerID: Hashable { let teamID: String; let bundleID: String }`
  - `struct Registry { init(json: Data) throws; static func bundled() throws -> Registry; func recipients(for: CallerID) -> Set<String>? }`
- The JSON shape: `{"version":1,"callers":[{"team_id":"KXSWJN7WY8","bundle_id":"ai.tracecommons.shell","recipients":["cloud.near.ai","tracecommons.ai"]}]}`

- [ ] **Step 1: Write the failing tests.**
  - `bundled_registry_has_trace_commons`: the Trace Commons caller maps to exactly `{"cloud.near.ai","tracecommons.ai"}`.
  - `unknown_caller_is_nil`.
  - `malformed_registry_throws`, covering each of: an unknown top-level key, a duplicate caller, a recipient that is not a lowercase DNS name, and `version` ≠ 1.
  - `same_bundle_other_team_is_nil`.
- [ ] **Step 2: Run `swift test --filter RegistryTests`.** Expected: FAIL.
- [ ] **Step 3: Implement.** Use a strict decode that rejects unknown keys by decoding into a `[String: Any]` and checking the key set. Load `bundled()` from `Bundle.module`. Write `CODEOWNERS` with `/Resources/registry/ @nearai/security` and `/scripts/ @nearai/security`; the team handle is a placeholder until the repo exists.
- [ ] **Step 4: Run `swift test --filter RegistryTests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Ship the reviewed caller registry".

### Task 4: Signature verification and on-chain key check

**Files:**
- Create: `Sources/NearSignerKit/Verify.swift`
- Test: `Tests/NearSignerKitTests/VerifyTests.swift`

**Interfaces:**
- Consumes: `nep413Digest` and `decodeNearPublicKey` (Task 1).
- Produces:
  - `func verifySignature(payload: NEP413Payload, publicKey: String, signatureBase64: String) -> Bool`
  - `func accessKeyIsFullAccess(rpcViewAccessKeyResponse: Data) -> Bool`
  - `protocol NearRPC { func viewAccessKey(account: String, publicKey: String) async throws -> Data }`, with an implementation `URLSessionNearRPC(endpoint: URL)` that sends a JSON-RPC `query` with `request_type: "view_access_key"` and `finality: "final"`

- [ ] **Step 1: Write the failing tests.**
  - `valid_signature_verifies`: generate a key with `Curve25519.Signing.PrivateKey`, sign `nep413Digest(p)`, and verify true.
  - `signature_over_other_nonce_is_invalid`: sign a payload with a different nonce, then verify against `p`. Expected: false.
  - `non_canonical_base64_is_invalid`.
  - `wrong_length_signature_is_invalid`.
  - `full_access_response_true`: a fixture shaped like `{"result":{"permission":"FullAccess","nonce":1,...}}`.
  - `function_call_key_false`: a fixture with `"permission":{"FunctionCall":{...}}`.
  - `rpc_error_false`: a fixture with `{"error":...}`.
- [ ] **Step 2: Run `swift test --filter VerifyTests`.** Expected: FAIL.
- [ ] **Step 3: Implement.** Use `Curve25519.Signing.PublicKey(rawRepresentation:).isValidSignature(_:for:)` over the 32-byte digest, which is what near_wallet.rs verifies. Require base64 to round-trip unchanged, as near_wallet.rs does at line 134.
- [ ] **Step 4: Run `swift test --filter VerifyTests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Verify near.com's signature and that its key is a full-access key on the account".

### Task 5: near.com client

**Files:**
- Create: `Sources/NearSignerKit/NearCom.swift`
- Test: `Tests/NearSignerKitTests/NearComTests.swift`

**Interfaces:**
- Consumes: `NEP413Payload` and `nep413Digest`.
- Produces:
  - `struct Challenge { let id: String; let challenge: Data; let expiresAt: Date }`
  - `struct Assertion: Codable { credentialID, authenticatorData, clientDataJSON, signature, userHandle: Data }`
  - `struct SignedSignIn: Equatable { let accountID: String; let publicKey: String; let signatureBase64: String }`
  - `protocol NearComAPI { func challenge(payloadHash: Data, accountHint: String?) async throws -> Challenge; func sign(challengeID: String, assertion: Assertion, payload: NEP413Payload) async throws -> SignedSignIn }`
  - `URLSessionNearCom(base: URL)`
  - Error mapping: any non-2xx response, a label the signer does not know, or a decoding failure throws `Refusal.near_refused`.

- [ ] **Step 1: Write the failing tests.** Use a `URLProtocol` stub; no new dependency.
  - `challenge_request_shape`: the body is exactly `{"payload_hash": base64url(sha256), "account_hint": null}` at `POST /signin/nep413/challenge`.
  - `sign_request_shape`: `POST /signin/nep413/sign`, with field names and nonce encoding matching the proposal document.
  - `server_error_is_near_refused`, for each of: 400 `payload_mismatch`, 401, 500, and a body that is not JSON.
  - `expires_more_than_2_min_ahead_is_refused`: a challenge whose `expires_at` is more than 2 minutes ahead is refused, because the proposal caps expiry at 2 minutes.
- [ ] **Step 2: Run `swift test --filter NearComTests`.** Expected: FAIL.
- [ ] **Step 3: Implement** with `URLSession`, an ephemeral configuration and a 15-second timeout. The base URL is `https://near.com`, with an `NEAR_SIGNER_NEARCOM_BASE` override honoured only in DEBUG builds.
- [ ] **Step 4: Run `swift test --filter NearComTests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Talk to near.com's NEP-413 sign-in endpoints".

### Task 6: XPC protocol and caller library

**Files:**
- Create: `Sources/NearSignerKit/Protocol.swift`, `Sources/NearSignerClient/NearSignerClient.swift`
- Test: `Tests/NearSignerKitTests/ProtocolTests.swift`

**Interfaces:**
- Produces:
  - `let signerProtocolVersion = 1`
  - `@objc protocol NearSignerXPC { func signIn(_ request: Data, minimumVersion: Int, reply: @escaping (Data?, String?) -> Void) }`. The request is JSON-encoded `NEP413Payload`. The reply is either the JSON-encoded `SignedSignIn` or a `Refusal` raw value.
  - `final class NearSignerClient { init(); func signIn(_ p: NEP413Payload, minimumVersion: Int = 1) async throws -> SignedSignIn }`. It throws `Refusal`, and `signer_outdated` when `minimumVersion > signerProtocolVersion`, or when the service is unreachable because no signer is installed.

- [ ] **Step 1: Write the failing tests.** `request_round_trip` and `refusal_round_trip` exercise the codecs without XPC. `newer_minimum_is_outdated` exercises the server-side version check function, `func versionRefusal(minimumVersion: Int) -> Refusal?`.
- [ ] **Step 2: Run `swift test --filter ProtocolTests`.** Expected: FAIL.
- [ ] **Step 3: Implement.** The client uses `NSXPCConnection(machServiceName: "com.iqlusion.near-signer.xpc")`, with a 10-minute reply timeout to cover a user who is slow at the consent window.
- [ ] **Step 4: Run `swift test --filter ProtocolTests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Define the signer's XPC protocol and caller library".

### Task 7: Sign-in orchestration

**Files:**
- Create: `Sources/NearSignerKit/SignInService.swift`
- Test: `Tests/NearSignerKitTests/SignInServiceTests.swift`

**Interfaces:**
- Consumes: Tasks 2 to 5.
- Produces:
  - `protocol ConsentPrompt { func ask(caller: CallerID, callerName: String, payload: NEP413Payload, accountHint: String?) async -> ConsentDecision }`, where `enum ConsentDecision { case approve(remember: Bool), decline }`
  - `protocol PasskeyCeremony { func assert(challenge: Data, accountHint: String?) async throws -> Assertion }`
  - `protocol ApprovalStore { func isRemembered(_: CallerID, _ recipient: String) -> Bool; func remember(_: CallerID, _ recipient: String) }`
  - `protocol SignInNotice { func signedIn(account: String, caller: CallerID) }`
  - `actor SignInService { init(registry:, nearCom:, rpc:, passkey:, consent:, approvals:, notice:, clock:); func signIn(caller: CallerID?, callerName: String, payload: NEP413Payload) async -> Result<SignedSignIn, Refusal> }`
  - `func cancel(caller: CallerID)`

- [ ] **Step 1: Write the failing tests.** Use fakes for all the protocols.
  - `nil_caller_refused_before_prompt`: an invalid code signature gives `caller_not_registered`, and the prompt is never called.
  - `unregistered_refused_before_prompt`.
  - `policy_refusal_before_prompt`.
  - `decline_is_user_declined`.
  - `happy_path_returns_verified_signature`: the fake near.com signs with a real Curve25519 key, and the fake RPC answers FullAccess.
  - `payload_swapped_by_server_is_signature_invalid`.
  - `function_call_key_is_signature_invalid`.
  - `passkey_error_is_passkey_unavailable`.
  - `remembered_pair_skips_prompt_but_not_passkey`: the passkey fake's call count is 1.
  - `remembered_pair_still_reports_signed_account`: `SignInNotice` receives the account.
  - `second_request_while_window_open_is_busy`.
  - `caller_disconnect_cancels_and_releases`: `cancel` during the passkey step returns `passkey_unavailable` to the first request, and the next request proceeds.
  - `rate_limit_busy`: a 6th request from one caller within 60 seconds gives `busy`.
- [ ] **Step 2: Run `swift test --filter SignInServiceTests`.** Expected: FAIL.
- [ ] **Step 3: Implement.** Follow the spec's flow order, steps 2 to 8. Verify against the signer's own payload, never against an echo from near.com. Add a `lastRefusalLabel` property for logging, which carries labels only.
- [ ] **Step 4: Run `swift test --filter SignInServiceTests`.** Expected: PASS.
- [ ] **Step 5: Commit** "Orchestrate a sign-in from caller check to verified signature".

### Task 8: The app (XPC listener, caller identity, passkey, consent, approvals)

**Files:**
- Create: `Sources/NearSigner/*`, `App/Info.plist`, `App/entitlements.plist`, `Resources/LaunchAgents/com.iqlusion.near-signer.agent.plist`
- Test: `Tests/NearSignerTests/CallerIdentityTests.swift`, `Tests/NearSignerTests/ApprovalsTests.swift`

**Interfaces:**
- Consumes: `SignInService` and the protocols from Task 7.
- Produces:
  - `func callerIdentity(auditToken: audit_token_t) -> (CallerID, String)?`, using `SecCodeCopyGuestWithAttributes` with `kSecGuestAttributeAudit`, `SecCodeCheckValidity` with a requirement of `anchor apple generic`, and `SecCodeCopySigningInformation` for the team identifier and bundle ID. It returns nil when anything is invalid. The display name comes from the bundle's `CFBundleDisplayName`.
  - `KeychainApprovalStore: ApprovalStore`, which stores items in the signer's own keychain access group.
  - `ASPasskeyCeremony: PasskeyCeremony`, using `ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: "near.com")` and a `createCredentialAssertionRequest` with `userVerificationPreference = .required`.
  - `ConsentWindowPrompt: ConsentPrompt`: an `NSPanel` at floating level showing the caller name, team, recipient and message, with Cancel and Sign in and a "Remember" checkbox. Strings come from `Copy.swift`.
  - `AgentRegistration.registerIfNeeded()`, using `SMAppService.agent(plistName:)`.
  - `Info.plist` sets `LSUIElement` to true.
  - `entitlements.plist` sets `com.apple.developer.associated-domains = ["webcredentials:near.com"]` and `keychain-access-groups = ["KXSWJN7WY8.com.iqlusion.near-signer"]`.

- [ ] **Step 1: Write the failing tests.**
  - `own_process_identity`: `callerIdentity` on the test process's own audit token (`xpc_connection` is not needed; `task_info(TASK_AUDIT_TOKEN)`) returns nil when the test binary is ad-hoc signed, because the binary has no team.
  - `signed_identity` is gated on the `NEAR_SIGNER_TEST_SIGNED_CALLER` environment variable pointing at a Developer ID–signed helper. It returns that helper's team and bundle ID, and is skipped if the variable is unset.
  - `remember_round_trip`, and `remember_is_per_pair`.
- [ ] **Step 2: Run `swift test --filter NearSignerTests`.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - `XPCListener` builds `CallerIdentity` from `connection.auditToken` (via `xpc_connection_get_audit_token` through the private accessor pattern that Apple sample code uses; if it is unavailable in the SDK, use `NSXPCConnection.effectiveUserIdentifier` plus `SecCodeCopyGuestWithAttributes` with `kSecGuestAttributePid`, and record that the pid is racy).
  - The invalidation handler calls `SignInService.cancel(caller:)`.
  - The consent and settings windows draw only text from `Copy.swift`.
- [ ] **Step 4: Run `swift test --filter NearSignerTests`.** Expected: PASS. Then run `swift build -c release`; it must succeed.
- [ ] **Step 5: Commit** "Run the signer as an on-demand agent that asks before every sign-in".

### Task 9: Release pipeline and entitlement checks

**Files:**
- Create: `scripts/make-release.sh`, `scripts/verify-entitlements.sh`, `.github/workflows/ci.yml`
- Test: `scripts/test-verify-entitlements.sh`

**Interfaces:**
- Produces:
  - `scripts/make-release.sh` produces a Developer ID–signed, notarized and stapled `NearSigner.dmg`, with Sparkle components signed individually. Base it on `macos/scripts/make-release-dmg.sh` in Trace Commons, and reuse its API-key notarization.
  - `scripts/verify-entitlements.sh <app>` exits non-zero unless all of these hold:
    - the entitlements equal the expected set exactly;
    - the team is `KXSWJN7WY8`;
    - the bundle ID is `com.iqlusion.near-signer`;
    - the embedded profile grants `webcredentials:near.com`.

- [ ] **Step 1: Write the failing test.** `scripts/test-verify-entitlements.sh` runs the verifier against four fixture `.app` stubs with an extra entitlement, a wrong team, a missing domain, and the correct set. It expects exit 1 three times, then 0.
- [ ] **Step 2: Run the test.** Expected: FAIL (the verifier doesn't exist yet).
- [ ] **Step 3: Implement both scripts and CI.** CI runs `swift test`, `swift build -c release` and the verifier test on `macos-26`. CI does not sign: signing and notarization run only in the release workflow, with secrets.
- [ ] **Step 4: Run the test.** Expected: PASS.
- [ ] **Step 5: Commit** "Build, notarize and verify signer releases".

### Task 10: Trace Commons daemon: native-signer completion

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/nearai_credential/near_wallet.rs` (around lines 105-155), `crates/trace-commons-contributor/src/daemon/native_flow.rs` (`handle_wallet` at line 205), `docs/contributor-daemon-ipc-v1_1.md`
- Test: the tests module in `near_wallet.rs`, and the tests in `native_flow.rs`

**Interfaces:**
- Produces:
  - `NearWalletChallenge::native_payload(&self) -> serde_json::Value` returns `{"message","recipient","nonce":base64}`. These are the same constants the browser route signs.
  - `NearWalletChallenge::verify_signed(&mut self, account_id: &str, public_key: &str, signature: &str, now_ms: u64) -> Result<VerifiedNearWalletSignIn, NearWalletError>`. This holds the shared checks taken from `verify_callback`: expiry, consumed, account ID, key, signature length, round-trip encoding and ed25519 verification over `signing_digest`. `verify_callback` now calls it after its state check.
  - `native_wallet_flow` actions:
    - `native_payload {flow_id}` returns the payload of the flow's live challenge;
    - `native_complete {flow_id, account_id, public_key, signature}` runs `verify_signed`, then continues exactly as a verified browser callback does today.
  - Errors use the existing `native_wallet_*` labels and add one new label, `native_wallet_signature_invalid`.

- [ ] **Step 1: Write the failing tests.**
  - `native_signed_matches_browser_digest`: sign `signing_digest` with a test key, and `verify_signed` accepts it.
  - `native_signed_rejects_other_nonce`.
  - `native_signed_is_single_use`: a second call gives `AlreadyUsed`.
  - `verify_callback_unchanged`: the existing callback tests still pass.
  - In `native_flow`: `native_complete_without_open_flow_is_refused`, and `native_payload_matches_browser_configuration` (the same message, recipient and nonce as `browser_configuration()`).
- [ ] **Step 2: Run the failing tests.** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor near_wallet native_flow`. Expected: FAIL.
- [ ] **Step 3: Implement.** Refactor without changing browser behaviour, add the two actions, and document both in the IPC doc's `native_wallet_flow` row.
- [ ] **Step 4: Verify.**
  - Run `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`; expected 0 failed.
  - Run `cargo clippy -p trace-commons-contributor --all-targets -- <CLAUDE.md allow-list> -D warnings`; expected clean.
  - Run `cargo fmt --all -- --check`.
- [ ] **Step 5: Commit** "Let the wallet flow finish with a signature from the native signer".

### Task 11: Trace Commons macOS: route, setting and first-run install

**Files:**
- Create: `macos/Sources/TCShellCore/NearSignerRoute.swift`
- Modify: `macos/Package.swift`, the first-run NEAR wallet step in `macos/Sources/TraceCommonsApp/` (the caller of `DaemonClient.nativeWalletFlow`, `DaemonClient.swift:82`), Settings > Private AI, and `macos/entitlements.plist` (no change expected; assert this)
- Test: `macos/Tests/TCShellCoreTests/NearSignerRouteTests.swift`

**Interfaces:**
- Consumes: `NearSignerClient` (Task 6); the daemon actions `native_payload` and `native_complete` (Task 10).
- Produces:
  - `enum WalletRoute { case nativeSigner, browser }`
  - `func chooseRoute(settingOn: Bool, signerInstalled: Bool) -> WalletRoute`
  - `func signerInstalled() -> Bool`: the app is found for bundle ID `com.iqlusion.near-signer`, and `SecStaticCodeCheckValidity` passes with the requirement `anchor apple generic and identifier "com.iqlusion.near-signer" and certificate leaf[subject.OU] = "KXSWJN7WY8"`.
  - `func runNativeSigner(flowID:) async -> Result<Void, String>` gets `native_payload`, calls `NearSignerClient.signIn`, then sends `native_complete`. Any `Refusal` falls back to the browser route only if the user chooses it. It is never automatic.
  - The setting "Use the NEAR signer app" is off by default, and its copy needs owner approval.

- [ ] **Step 1: Write the failing tests.**
  - `route_table`: the setting off gives browser, and the setting on with no signer gives browser. Only the setting on with the signer installed gives nativeSigner.
  - `requirement_string_pins_team_and_bundle`.
  - `refusal_never_auto_falls_back`: with a fake client throwing `near_refused`, the browser is not opened.
- [ ] **Step 2: Run the failing tests.** Build the FFI dylib (`cargo build -p trace-commons-contributor-ffi`), then run `swift test --filter NearSignerRouteTests` in `macos/`. Expected: FAIL.
- [ ] **Step 3: Implement.** Do not add the "install the signer" download step until the signer has a published release URL. Behind the setting, a missing signer means the browser route, and the setting's caption names the signer, with copy pending approval.
- [ ] **Step 4: Verify.** Run `swift test` in `macos/`, which must show no new failures relative to main. The Insights OCR test is a known issue on dev machines. Then run `scripts/ci/native-passkey-entitlements.py` to confirm Trace Commons' own entitlements are unchanged.
- [ ] **Step 5: Commit** "Sign in to NEAR through the signer app when it is installed and turned on".

### Task 12: End-to-end against a stub near.com

**Files:**
- Create: `Tests/NearSignerTests/EndToEndTests.swift`, `scripts/stub-nearcom.swift` (a tiny `NWListener` HTTP stub; no dependency)
- Test: the same

**Interfaces:**
- Consumes: everything in the signer. The stub implements the proposal's two endpoints and signs with a fixed test key. A fake RPC returns FullAccess for that key.

- [ ] **Step 1: Write the failing test.** `stub_round_trip`: `SignInService` with `URLSessionNearCom(base: stub)`, a fake passkey, and auto-approve consent returns a signature that `verifySignature` accepts. The stub refuses a `payload_hash` mismatch with 400 `payload_mismatch`, which surfaces as `near_refused`.
- [ ] **Step 2: Run `swift test --filter EndToEndTests`.** Expected: FAIL.
- [ ] **Step 3: Implement the stub.**
- [ ] **Step 4: Run `swift test`.** Expected: all pass.
- [ ] **Step 5: Commit** "Test a full sign-in against a stub near.com".

---

## Not in this plan

These wait on NEAR or the owner:
- the live near.com staging run and the AASA entry;
- the first-run download and install step;
- the Windows and Linux signers;
- all UI copy;
- creating `nearai/near-signer`.
