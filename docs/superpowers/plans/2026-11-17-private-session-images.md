# Private-session image routing and production release

> **For agentic workers:** Disjoint backend parsing, core eligibility, synthetic proxy conformance, and release preparation. One coordinator owns builds and publication.

**Goal:** Permit images only for the exact selected model when its trusted provider catalogue advertises image inputs, preserve image contents on each private-session wire, then admin-merge and publish production 1.0.9.

- [x] Identify hardcoded NEAR AI images=false and verify public /v1/models declarations for current image-capable models. No private session contents or images were sent to a provider.
- [x] Write core, upstream parsing/refresh, and local HTTP wire regressions; run coordinated RED before behavior implementation.
- [x] Propagate exact per-model metadata from provider discovery through backend snapshots into policy. Pins, sticky choices and fallback must check the model actually served. Unknown, malformed or conflicting declarations remain blocked; preserve existing native capability defaults and all privacy/consent gates.
- [x] Verify native image bytes and translated image contents with synthetic loopback providers. Keep request refusal readable; no silent image stripping or privacy downgrade.
- [ ] Prepare 1.0.9 manifest/lock/metainfo versions without unrelated dependency changes. Run scoped test suites, Clippy, formatting, license boundary and independent review; open PR and admin-merge verified head.
- [ ] Build and inspect production candidates, publish CLI first then app at the same merged source, verify published signatures/notarization/checksums/provenance/update feeds and update Homebrew channels. Keep feature flags off and staging unchanged. No user-private image replay or automatic app installation.

Design: an additive Backend image_support map and Candidate map allow the existing centralized capability gate to evaluate the selected model. Provider-wide images stays false for generic Chat/NEAR AI. Parser accepts explicit normalized/nested modality arrays only; exact IDs, conservative malformed/conflicting data handling, and fresh snapshots revoke old support. Models remain evidence-bound rather than inferred from names.

Validation: RED reproduced the original model-image refusal and the added sticky-model, refresh and nested-image boundary failures. Final focused image regressions pass (8 loopback wire cases); all core, upstream and translation tests, scoped Clippy, formatting, four license feature sets, all-feature advisory/source/ban checks and the four license-boundary tests pass. The final proxy regression rerun follows the translation-only error-reporting repair. Independent security review found no remaining blockers.
