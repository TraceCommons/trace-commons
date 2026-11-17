# Witness review recovery implementation plan

**Goal:** Let an approved ordinary trace upload without inference receipts, and retain its certified review across an authorized signed witness signer rotation.

**Architecture:** Select the explicit-review profile from the actual captured request before body-consent filtering, matching ordinary upload behavior. Validate saved certificates against the signers current signed routing authorizes, read from the real contributor state directory. A matching signed withdrawal remains authoritative; no signup fallback. Keep receipt validation for requested attested-inference contributions and retain all source, fingerprint, consent, grant, exact-byte and server admission checks.

**User decisions:** Ordinary clients must not require inference receipts when the server does not. The owner approved one specific real trace and authorized fixing the client blockers. No body-export consent or enrollment/grant change is part of this repair.

- [x] Isolate from latest origin/main using `.claude/bin/tc-worktree witness-rotation-review-fix`.
- [x] Add regressions for ordinary explicit review with no call or a legacy call while body export is off; keep actually bound calls fail-closed without consent.
- [x] Add saved-review regressions for rotation, withdrawn signup authority, measurement change, expiry and corrupt cache.
- [x] Pass the actual state directory through review creation, display, approval and upload validation.
- [x] Demonstrate the original receipt and saved-review regressions fail under the old behavior; restore fixes and run combined contributor checks.
- [x] Run contributor suite, relevant client/server contracts, formatting and clippy; build FFI and run the native Swift suite as required by repo guidance.
- [x] Independently review the diff; fix cached-send signer revalidation and rotated redaction-disclosure findings.
- [x] Exercise the repaired client on the already approved trace. It passed witness redaction, certificate verification and the client checks; ingest acceptance was not reached (an account limit unrelated to this change). No other trace was approved and no consent was altered.
- [x] Owner review: a saved review rechecks only that its signer is still authorized. A change to published measurements does not make it stale; measurements were verified against the attestation quote when the review was made.

No new dependencies or license-boundary changes.
