# Witness review recovery implementation plan

**Goal:** Let an approved ordinary trace upload without inference receipts, and retain its certified review across an authorized signed witness signer rotation.

**Architecture:** Select the explicit-review profile from the actual captured request before body-consent filtering, matching ordinary upload behavior. Validate saved certificates against current signed routing candidates and paired measurement pins from the real contributor state directory. A matching signed withdrawal remains authoritative; no signup fallback. Keep receipt validation for requested attested-inference contributions and retain all source, fingerprint, consent, grant, exact-byte and server admission checks.

**User decisions:** Ordinary clients must not require inference receipts when the server does not. The owner approved one specific real trace and authorized fixing the client blockers. No body-export consent or enrollment/grant change is part of this repair.

- [x] Isolate from latest origin/main using `.claude/bin/tc-worktree witness-rotation-review-fix`.
- [x] Add regressions for ordinary explicit review with no call or a legacy call while body export is off; keep actually bound calls fail-closed without consent.
- [x] Add saved-review regressions for rotation, withdrawn signup authority, signer/measurement pairing, expiry and corrupt cache.
- [x] Pass the actual state directory through review creation, display, approval and upload validation.
- [x] Demonstrate the original receipt and saved-review regressions fail under the old behavior; restore fixes and run combined contributor checks.
- [x] Run contributor suite, relevant client/server contracts, formatting and clippy; build FFI and run the native Swift suite as required by repo guidance.
- [x] Independently review the diff; fix cached-send signer revalidation and rotated redaction-disclosure findings.
- [ ] Exercise the repaired client on the already approved real trace and verify ingest acceptance. Do not approve another trace or alter consent to force success.

No new dependencies or license-boundary changes. The primary checkout and the running replacement CVM/ingest are untouched by the code repair.

## Live result

The exact approved trace was loaded through Codex compressed-source support
and its queued hash matched. A first retry returned the generic witness
malformed-response label. A second retry passed witness redaction,
certificate verification and client checks, then ingest returned
`account_limit_reached`. Authenticated account status confirmed exhausted
processing allowance with an advisory retry of approximately five days.
No consent, enrollment, grants or quota policy were changed. No accepted
contribution or installed-app update is claimed.
