# Native passkey release qualification

The native SwiftUI bundle is `ai.tracecommons.shell`. Its signed entitlement must
contain exactly `com.apple.developer.associated-domains =
[webcredentials:tracecommons.ai]`, and the embedded Apple-issued profile must
permit that domain. The generic verifier still supports the Tauri bundle without
requiring native Associated Domains.

The committed renewed profile is **Trace Commons Developer ID (macOS)
2026-09-28**, UUID `76f964cd-8094-4e91-9173-e907e0cf42ff`, expiring
`2044-09-24T12:44:05Z`. Its decoded Associated Domains grant is Apple's scalar
`*`; its application identifier is `KXSWJN7WY8.ai.tracecommons.shell` and its
team identifier is `KXSWJN7WY8`. The committed CMS SHA-256 is
`af4c2f61086afc8caf0b381a8c0751e8b6a930316fa734933c64169cd09a1988`.
These metadata were decoded from the supplied profile bytes; older issue text
reported a different expiry and is not the source of truth for this file.

`macos/scripts/make-release-dmg.sh` validates the native app/team, keychain grant,
Associated Domains grant and minimum 180-day expiry before building or importing
signing credentials. It embeds the selected CMS bytes unchanged, signs the
native app with the committed entitlement plist, and checks the actual signature
and embedded profile before packaging. `TC_MACOS_PROVISION_PROFILE` may explicitly
select another local Apple-issued CMS file; the same checks apply. Never edit or
manufacture a CMS grant.

The metadata unit tests use synthetic dictionaries, and the shell fixtures use
ad-hoc-signed temporary bundles. They prove refusal behavior, including wrong
team/domain/grant and missing signed Associated Domains. They do **not** prove
Developer ID launch, Apple platform passkey origin behavior or successful login.

Run the release checks against the actual Developer ID signed native bundle:

```bash
scripts/ci/verify-macos-entitlements.sh /path/to/TraceCommons.app
scripts/check-aasa.sh https://tracecommons.ai KXSWJN7WY8.ai.tracecommons.shell
```

The first command retains the real launch/Keychain reachability probe. Setting
`TC_VERIFY_STATIC_ONLY=1` explicitly skips that probe and cannot qualify a launch.
Normal notarization, stapling, Gatekeeper and clean-machine checks remain in the
existing release flow and [client end-to-end verification](client-end-to-end-verification.md).

Before claiming native passkeys work, complete a signed staging create **and**
sign-in. Record only the observed `clientDataJSON.origin` and confirm exact
membership in the configured origin allowlist; the expected native origin
`https://tracecommons.ai` remains an expectation until observed. Do not record
challenges, bearer/session tokens, user/account IDs or credential bytes. Verify
both apex AASA and Apple's CDN contain `KXSWJN7WY8.ai.tracecommons.shell`. A profile
and local fixture checks do not satisfy that runtime gate.
